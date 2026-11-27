//! A bounded queue for any number of producers and consumers.
//!
//! [`ArrayQueue`] is shared by reference, usually behind an `Arc`, and every
//! thread may both push and pop. The algorithm is Dmitry Vyukov's bounded MPMC
//! queue, in the shape crossbeam's `ArrayQueue` uses.
//!
//! # How it works
//!
//! `head` and `tail` are stamps: the low bits are a slot index below the
//! capacity, the high bits count laps, in units of `one_lap`, the smallest power
//! of two above the capacity. Because `one_lap` is above the capacity, adding 1
//! to a stamp never carries into the lap bits, and laps wrap around `u64`
//! without disturbing the index.
//!
//! The stamps are 64 bits on every target, not `usize`. A thread that has read a
//! stamp and not yet made its compare-and-swap is fooled if the stamp comes
//! round to the same value in between; with 64 bits that takes at least 2^63
//! completed calls during the one stall, and with a 32-bit `usize` it would take
//! 2^31 (docs/adr/0006-array-queue-64-bit-stamps.md).
//!
//! Each slot has a stamp of its own that says what the slot is ready for:
//!
//! - equal to the tail stamp that points at it: empty, ready for a push at that
//!   lap;
//! - one more than that: holds the value pushed at that lap, ready for a pop;
//! - a lap later: the value has been popped, and the slot is ready for the push
//!   of the next lap.
//!
//! A push claims a slot by moving `tail` past it with a compare-and-swap, writes
//! the value, then publishes it by advancing the slot's stamp with a Release
//! store. A pop does the same with `head`, reads the value, and frees the slot
//! for the next lap.
//!
//! # Progress
//!
//! No thread takes a lock, but the queue is not lock-free in the formal sense:
//! a stalled thread can stop others from completing. A push that has claimed a
//! slot and stalls before publishing it makes every pop that reaches that slot
//! wait, and no value behind it can be popped until it publishes. A pop that has
//! claimed a value and stalls before freeing the slot makes every push that
//! reaches that slot a lap later wait. A push that reaches a stalled push's slot
//! reports a full queue instead of waiting, and pops move past a stalled pop.
//! crossbeam's `ArrayQueue` and Vyukov's original are not lock-free either, for
//! the same reason.

use alloc::boxed::Box;
use core::fmt;
use core::mem::MaybeUninit;

use crate::cache_padded::CachePadded;
use crate::sync::{AtomicU64, Ordering, UnsafeCell, fence, spin};

#[cfg(all(test, not(loom)))]
mod tests;

#[cfg(all(test, loom))]
mod loom_tests;

/// `n` as a `u64`. No Rust target has a `usize` wider than 64 bits, so nothing
/// is lost.
const fn widen(n: usize) -> u64 {
    n as u64
}

/// Ordering of the store in [`ArrayQueue::push`] that hands a written slot to
/// the consumers. Release, so that writing the value happens before a pop's
/// Acquire load of the slot's stamp sees it ready.
#[cfg(not(latchless_mutation = "mpmc_relaxed_publish"))]
const PUBLISH: Ordering = Ordering::Release;

/// Wrong on purpose. CI builds this variant and requires loom to reject it
/// (docs/adr/0003-known-wrong-variants.md).
#[cfg(latchless_mutation = "mpmc_relaxed_publish")]
const PUBLISH: Ordering = Ordering::Relaxed;

/// One slot of the queue: its stamp and the value it may hold.
struct Slot<T> {
    /// What the slot is ready for; see the module docs.
    stamp: AtomicU64,
    /// Initialised exactly when `stamp` is one more than a tail stamp that
    /// points at this slot, and between a pop's claim and its read.
    value: UnsafeCell<MaybeUninit<T>>,
}

/// A bounded queue for any number of producers and consumers that takes no
/// locks. It is not lock-free: see [the module docs](crate::mpmc#progress).
///
/// # Examples
///
/// ```
/// use latchless::mpmc::ArrayQueue;
///
/// let queue = ArrayQueue::new(2);
/// assert_eq!(queue.push('a'), Ok(()));
/// assert_eq!(queue.push('b'), Ok(()));
/// assert_eq!(queue.push('c'), Err('c'));
/// assert_eq!(queue.pop(), Some('a'));
/// ```
///
/// Shared between threads:
///
/// ```
/// use latchless::mpmc::ArrayQueue;
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// let queue = ArrayQueue::new(4);
/// let sum = AtomicUsize::new(0);
/// std::thread::scope(|scope| {
///     for producer in 0..2 {
///         let queue = &queue;
///         scope.spawn(move || {
///             for value in 1..=50 {
///                 let mut value = producer * 1000 + value;
///                 while let Err(back) = queue.push(value) {
///                     value = back;
///                     std::thread::yield_now();
///                 }
///             }
///         });
///     }
///     for _ in 0..2 {
///         scope.spawn(|| {
///             for _ in 0..50 {
///                 loop {
///                     if let Some(value) = queue.pop() {
///                         sum.fetch_add(value, Ordering::Relaxed);
///                         break;
///                     }
///                     std::thread::yield_now();
///                 }
///             }
///         });
///     }
/// });
/// assert_eq!(sum.into_inner(), 2 * (1..=50).sum::<usize>() + 50 * 1000);
/// ```
pub struct ArrayQueue<T> {
    /// Stamp of the next slot to pop. Moved only by a pop's compare-and-swap.
    head: CachePadded<AtomicU64>,
    /// Stamp of the next slot to push. Moved only by a push's compare-and-swap.
    tail: CachePadded<AtomicU64>,
    /// The lap unit: the smallest power of two above the capacity.
    one_lap: u64,
    slots: Box<[Slot<T>]>,
}

// SAFETY: an `ArrayQueue<T>` owns the values in it, so sending the queue sends
// them, which needs `T: Send` and nothing more.
unsafe impl<T: Send> Send for ArrayQueue<T> {}

// SAFETY: through `&ArrayQueue<T>` a thread can only move values of `T` in and
// out; it never gets a `&T`. So sharing the queue lets values move between
// threads, which needs `T: Send`, and `T: Sync` is not needed. Each slot is
// written by the one push and read by the one pop that won its compare-and-swap
// for that lap, and the stamp's Release/Acquire pairs order those accesses.
unsafe impl<T: Send> Sync for ArrayQueue<T> {}

impl<T> ArrayQueue<T> {
    /// Creates a queue that holds up to `capacity` values.
    ///
    /// A capacity of zero is allowed: every push fails and every pop finds the
    /// queue empty.
    ///
    /// # Panics
    ///
    /// If `capacity` is greater than `usize::MAX / 2`, because a stamp needs
    /// room above the capacity for at least two laps. Also if the slots need
    /// more than `isize::MAX` bytes, the same as `Vec::with_capacity`.
    #[must_use = "a queue that is created and dropped does nothing"]
    pub fn new(capacity: usize) -> Self {
        Self::starting_at_lap(capacity, 0)
    }

    /// Creates the queue with `head` and `tail` at the start of lap `lap`.
    /// `new` starts at lap 0; the tests start near the end of `u64` to see the
    /// laps wrap.
    fn starting_at_lap(capacity: usize, lap: u64) -> Self {
        // This assert is what keeps `capacity + 1` and `one_lap` from overflowing
        // a `u64` on a 64-bit target; with a 32-bit `usize` it holds with room to
        // spare. Allocating the slots would fail first for most such capacities,
        // but the stamps should not depend on how big a slot happens to be.
        assert!(
            capacity <= usize::MAX / 2,
            "capacity must be at most usize::MAX / 2, because a stamp needs a lap above it"
        );
        let one_lap = (widen(capacity) + 1).next_power_of_two();
        let first = lap.wrapping_mul(one_lap);
        let slots = (0..capacity)
            .map(|index| Slot {
                stamp: AtomicU64::new(first + widen(index)),
                value: UnsafeCell::new(MaybeUninit::uninit()),
            })
            .collect();
        Self {
            head: CachePadded::new(AtomicU64::new(first)),
            tail: CachePadded::new(AtomicU64::new(first)),
            one_lap,
            slots,
        }
    }

    /// The most values the queue can hold at once.
    #[must_use = "capacity has no side effects"]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The slot index in a stamp's low bits.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the low bits are below `one_lap`, which is at most `usize::MAX / 2 + 1`, so they fit"
    )]
    fn index(&self, stamp: u64) -> usize {
        (stamp & (self.one_lap - 1)) as usize
    }

    /// The slot a stamp points at. Only called on a queue with at least one slot.
    fn slot(&self, stamp: u64) -> &Slot<T> {
        &self.slots[self.index(stamp)]
    }

    /// The stamp after `stamp`: the next index, or index 0 of the next lap.
    fn next(&self, stamp: u64) -> u64 {
        if self.index(stamp) + 1 < self.capacity() {
            stamp + 1
        } else {
            (stamp & !(self.one_lap - 1)).wrapping_add(self.one_lap)
        }
    }

    /// Appends `value` unless the queue is full.
    ///
    /// # Errors
    ///
    /// Returns `value` back if the queue is full.
    pub fn push(&self, value: T) -> Result<(), T> {
        if self.slots.is_empty() {
            return Err(value);
        }
        // Relaxed: only a guess at the slot to try. The stamp load below decides
        // whether the slot is free, and the compare-and-swap whether this thread
        // gets it.
        let mut tail = self.tail.load(Ordering::Relaxed);
        loop {
            let slot = self.slot(tail);
            // Acquire: pairs with the Release store of the stamp in `pop`, so the
            // pop that emptied this slot a lap ago has finished reading it before
            // this push writes it.
            let stamp = slot.stamp.load(Ordering::Acquire);
            if stamp == tail {
                // SeqCst success: the empty check in `pop` puts a SeqCst fence before
                // reading `tail`, and a SeqCst operation is what such a fence is
                // ordered against. Its Release half matters too: a push that read
                // this `tail` acquires it through the fence in its full check, and
                // then sees the `head` this thread's earlier pops moved. Relaxed
                // failure: the value is only a new guess.
                match self.tail.compare_exchange_weak(
                    tail,
                    self.next(tail),
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        slot.value.with_mut(|cell| {
                            // SAFETY: this thread moved `tail` past the slot at this
                            // lap, so no other push writes it, and its stamp says it
                            // is empty, so no pop reads it until the store below.
                            // Both facts are about this lap unless `tail` came round
                            // to the value checked above in between, which takes at
                            // least 2^63 completed pushes during this one stall; the
                            // crate assumes that does not happen, as crossbeam does
                            // on 64-bit targets (docs/adr/0006-array-queue-64-bit-stamps.md).
                            // The Acquire load of the stamp ordered the last pop's
                            // read before this write. An empty slot holds no value,
                            // so nothing is overwritten without being dropped.
                            unsafe { cell.cast::<T>().write(value) }
                        });
                        // Release (see `PUBLISH`): pairs with the Acquire load of the
                        // stamp in `pop`, so the write above happens before the read.
                        slot.stamp.store(tail + 1, PUBLISH);
                        return Ok(());
                    }
                    Err(current) => tail = current,
                }
            } else if stamp.wrapping_add(self.one_lap) == tail + 1
                || stamp.wrapping_add(self.one_lap) == tail
            {
                // The slot still belongs to the previous lap: it holds that lap's
                // value, or that lap's push has claimed it and is still writing. The
                // queue is full if no pop has claimed that value yet. crossbeam spins
                // in the second case; checking `head` here instead reports a full
                // queue without waiting for a slow push
                // (docs/adr/0005-array-queue-full-check.md).
                // SeqCst fence: pairs with the SeqCst compare-and-swap on `head` in
                // `pop`. A pop whose claim comes before this fence in the single
                // order of SeqCst operations is seen by the load below.
                fence(Ordering::SeqCst);
                // Relaxed: ordered by the fence above.
                let head = self.head.load(Ordering::Relaxed);
                if head.wrapping_add(self.one_lap) == tail {
                    return Err(value);
                }
                // A pop has claimed the value but not freed the slot yet.
                spin();
                // Relaxed: only a new guess, as above.
                tail = self.tail.load(Ordering::Relaxed);
            } else {
                // Another push got here first, so `tail` was stale.
                spin();
                // Relaxed: only a new guess, as above.
                tail = self.tail.load(Ordering::Relaxed);
            }
        }
    }

    /// Removes the oldest value, or returns `None` if the queue is empty.
    pub fn pop(&self) -> Option<T> {
        if self.slots.is_empty() {
            return None;
        }
        // Relaxed: only a guess at the slot to try, as in `push`.
        let mut head = self.head.load(Ordering::Relaxed);
        loop {
            let slot = self.slot(head);
            // Acquire: pairs with the Release store of the stamp in `push`, so the
            // write of the value happens before this pop reads it.
            let stamp = slot.stamp.load(Ordering::Acquire);
            if stamp == head + 1 {
                // SeqCst success: pairs with the SeqCst fence in `push`'s full check,
                // as that fence's comment says. Its Release half matters too, the
                // same way as on `tail`: a pop that read this `head` sees the `tail`
                // this thread's earlier pushes moved. Relaxed failure: only a new
                // guess.
                match self.head.compare_exchange_weak(
                    head,
                    self.next(head),
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let value = slot.value.with(|cell| {
                            // SAFETY: the stamp says the slot holds the value pushed
                            // at this lap, and the Acquire load ordered its write
                            // before this read. This thread moved `head` past it, so
                            // no other pop reads it, and no push writes it until the
                            // store below frees the slot, so it is read once. All of
                            // this is about this lap unless `head` came round to the
                            // value checked above in between, which takes at least
                            // 2^63 completed pops during this one stall; the crate
                            // assumes that does not happen, as crossbeam does on
                            // 64-bit targets (docs/adr/0006-array-queue-64-bit-stamps.md).
                            unsafe { cell.cast::<T>().read() }
                        });
                        // Release: pairs with the Acquire load of the stamp in `push`,
                        // so the read above happens before the next lap's write.
                        slot.stamp
                            .store(head.wrapping_add(self.one_lap), Ordering::Release);
                        return Some(value);
                    }
                    Err(current) => head = current,
                }
            } else if stamp == head {
                // Nothing has been published in this slot at this lap. The queue is
                // empty if no push has claimed the slot yet.
                // SeqCst fence: pairs with the SeqCst compare-and-swap on `tail` in
                // `push`, the same way as the fence in `push`'s full check.
                fence(Ordering::SeqCst);
                // Relaxed: ordered by the fence above.
                let tail = self.tail.load(Ordering::Relaxed);
                if tail == head {
                    return None;
                }
                // A push has claimed the slot but not published it yet.
                spin();
                // Relaxed: only a new guess, as above.
                head = self.head.load(Ordering::Relaxed);
            } else {
                // Another pop got here first, so `head` was stale.
                spin();
                // Relaxed: only a new guess, as above.
                head = self.head.load(Ordering::Relaxed);
            }
        }
    }
}

/// Drops the values from stamp `position` up to `tail` of a queue being
/// dropped.
///
/// Its own `Drop` drains whatever is left, so one destructor that panics does
/// not stop the others, the way `Vec` does it. `position` moves past a value
/// before that value is dropped, so no value is dropped twice.
struct DropRemaining<'a, T> {
    queue: &'a ArrayQueue<T>,
    position: u64,
    tail: u64,
}

impl<T> DropRemaining<'_, T> {
    fn drop_all(&mut self) {
        while self.position != self.tail {
            let position = self.position;
            self.position = self.queue.next(position);
            self.queue.slot(position).value.with_mut(|cell| {
                // SAFETY: the queue is being dropped, so every push and pop has
                // returned, and every stamp in `[head, tail)` belongs to a push that
                // wrote its value and a pop that never claimed it. Nothing else can
                // reach the slot. `self.position` has already moved past it, so the
                // value is dropped once even if its destructor panics.
                unsafe { cell.cast::<T>().drop_in_place() }
            });
        }
    }
}

impl<T> Drop for DropRemaining<'_, T> {
    fn drop(&mut self) {
        // Only does work when `drop_all` was cut short by a panic.
        self.drop_all();
    }
}

impl<T> Drop for ArrayQueue<T> {
    fn drop(&mut self) {
        // Relaxed: `&mut self` means no other thread holds the queue any more, and
        // whatever handed it over (an `Arc` drop, a thread join) ordered every
        // earlier push and pop before this.
        let position = self.head.load(Ordering::Relaxed);
        // Relaxed: as above.
        let tail = self.tail.load(Ordering::Relaxed);
        DropRemaining {
            queue: self,
            position,
            tail,
        }
        .drop_all();
    }
}

impl<T> fmt::Debug for ArrayQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrayQueue")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

/// Negative `Send` and `Sync` checks (docs/design.md, "Rules for unsafe").
///
/// Each doctest is a `compile_fail` for one reason only. `E0277` pins the failure
/// to an unmet trait bound, so an unrelated compile error cannot make one pass.
#[cfg(doctest)]
mod negative_checks {
    /// `ArrayQueue<Rc<()>>` is not `Send`, because `Rc<()>` is not.
    ///
    /// ```compile_fail,E0277
    /// use latchless::mpmc::ArrayQueue;
    /// use std::rc::Rc;
    ///
    /// fn assert_send<T: Send>() {}
    /// assert_send::<ArrayQueue<Rc<()>>>();
    /// ```
    fn queue_needs_t_send_to_be_send() {}

    /// `ArrayQueue<Rc<()>>` is not `Sync`, because sharing the queue moves values
    /// between threads.
    ///
    /// ```compile_fail,E0277
    /// use latchless::mpmc::ArrayQueue;
    /// use std::rc::Rc;
    ///
    /// fn assert_sync<T: Sync>() {}
    /// assert_sync::<ArrayQueue<Rc<()>>>();
    /// ```
    fn queue_needs_t_send_to_be_sync() {}

    /// `T: Sync` is not enough to share the queue: a `MutexGuard` is `Sync` but
    /// must not be popped on another thread than the one that locked it.
    ///
    /// ```compile_fail,E0277
    /// use latchless::mpmc::ArrayQueue;
    /// use std::sync::MutexGuard;
    ///
    /// fn assert_sync<T: Sync>() {}
    /// assert_sync::<ArrayQueue<MutexGuard<'static, ()>>>();
    /// ```
    fn queue_needs_t_send_even_when_t_is_sync() {}
}
