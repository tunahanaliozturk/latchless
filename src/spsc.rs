//! A bounded ring buffer with one producer and one consumer.
//!
//! [`RingBuffer::new`] returns a [`Producer`] and a [`Consumer`]. Each handle is
//! the only one of its kind, so "one producer, one consumer" is checked by the
//! compiler instead of promised in a comment. Both operations are wait-free:
//! neither ever loops waiting for the other thread.
//!
//! # How it works
//!
//! Two counters, `head` (written only by the consumer) and `tail` (written only
//! by the producer), run through the positions `0..2 * capacity` and wrap to 0.
//! Position `p` lives in slot `p` or `p - capacity`. Counting to twice the
//! capacity is what tells a full ring (`tail - head == capacity`) from an empty
//! one (`tail == head`) without giving up a slot.
//!
//! The slots in `[head, tail)` hold values and belong to the consumer; the rest
//! belong to the producer. A handle moves the boundary only after it has
//! finished with the slot, with a Release store, and the other handle reads the
//! boundary with an Acquire load before touching the slot. Each handle also keeps
//! its own copy of the other side's counter and reloads it only when that copy
//! says the ring is full (producer) or empty (consumer), so in the common case the
//! two threads do not read each other's cache line at all.

use alloc::boxed::Box;
use core::fmt;
use core::iter;
use core::mem::MaybeUninit;

use crate::cache_padded::CachePadded;
use crate::sync::{Arc, AtomicUsize, Ordering, UnsafeCell};

#[cfg(all(test, not(loom)))]
mod tests;

#[cfg(all(test, loom))]
mod loom_tests;

/// Ordering of the store in [`Producer::push`] that hands a written slot to the
/// consumer. Release, so that writing the value happens before the consumer's
/// Acquire load of `tail` sees the new position.
#[cfg(not(latchless_mutation = "spsc_relaxed_publish"))]
const PUBLISH: Ordering = Ordering::Release;

/// Wrong on purpose. CI builds this variant and requires loom to reject it, which
/// shows the loom tests can tell a correct ordering from a broken one
/// (docs/adr/0003-known-wrong-variants.md).
#[cfg(latchless_mutation = "spsc_relaxed_publish")]
const PUBLISH: Ordering = Ordering::Relaxed;

/// The storage shared by a [`Producer`] and a [`Consumer`].
///
/// It is never handed out on its own: [`RingBuffer::new`] returns the two
/// handles, and the buffer is freed, together with any values still in it, when
/// both handles have been dropped.
pub struct RingBuffer<T> {
    /// Position of the next value to pop. Stored only by the consumer.
    head: CachePadded<AtomicUsize>,
    /// Position of the next free slot. Stored only by the producer.
    tail: CachePadded<AtomicUsize>,
    /// The slot of position `p` holds an initialised value exactly when `p` is
    /// in `[head, tail)`.
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

impl<T> RingBuffer<T> {
    /// Creates a ring that holds up to `capacity` values and returns its two
    /// ends.
    ///
    /// A capacity of zero is allowed: every push fails and every pop finds the
    /// ring empty.
    ///
    /// # Panics
    ///
    /// If `capacity` is greater than `usize::MAX / 2`, because the positions run
    /// up to twice the capacity. Also if the slots need more than `isize::MAX`
    /// bytes, the same as `Vec::with_capacity`.
    ///
    /// # Examples
    ///
    /// ```
    /// use latchless::spsc::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(2);
    /// std::thread::scope(|scope| {
    ///     scope.spawn(move || {
    ///         for value in 0..100 {
    ///             let mut value = value;
    ///             while let Err(back) = producer.push(value) {
    ///                 value = back;
    ///                 std::thread::yield_now();
    ///             }
    ///         }
    ///     });
    ///     let mut expected = 0;
    ///     while expected < 100 {
    ///         if let Some(value) = consumer.pop() {
    ///             assert_eq!(value, expected);
    ///             expected += 1;
    ///         }
    ///     }
    /// });
    /// ```
    #[expect(
        clippy::new_ret_no_self,
        reason = "the ring is only reachable through its two handles, as in rtrb"
    )]
    #[must_use = "the ring is freed when both handles are dropped"]
    pub fn new(capacity: usize) -> (Producer<T>, Consumer<T>) {
        // This assert is what makes `2 * capacity` in `next` and `len` safe from
        // overflow. Building the slots below does not, because for a zero-sized `T`
        // it allocates nothing and need not stop at a size that fits.
        assert!(
            capacity <= usize::MAX / 2,
            "capacity must be at most usize::MAX / 2, because positions run up to twice the capacity"
        );
        let slots = iter::repeat_with(|| UnsafeCell::new(MaybeUninit::uninit()))
            .take(capacity)
            .collect();
        let ring = Arc::new(Self {
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            slots,
        });
        let producer = Producer {
            ring: Arc::clone(&ring),
            tail: 0,
            cached_head: 0,
        };
        let consumer = Consumer {
            ring,
            head: 0,
            cached_tail: 0,
        };
        (producer, consumer)
    }

    fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The position after `position`. Only called on a ring with at least one
    /// slot, after a full or empty check that a zero-capacity ring always fails.
    fn next(&self, position: usize) -> usize {
        let next = position + 1;
        if next == 2 * self.capacity() { 0 } else { next }
    }

    /// How many values lie between `head` and `tail`.
    fn len(&self, head: usize, tail: usize) -> usize {
        if head <= tail {
            tail - head
        } else {
            2 * self.capacity() - head + tail
        }
    }

    fn slot(&self, position: usize) -> &UnsafeCell<MaybeUninit<T>> {
        let capacity = self.capacity();
        let index = if position < capacity {
            position
        } else {
            position - capacity
        };
        &self.slots[index]
    }
}

/// Drops the values in positions `[position, tail)` of a ring being dropped.
///
/// Its own `Drop` drains whatever is left. That is what keeps going when one
/// value's destructor panics: unwinding drops this guard, which drops the rest,
/// the way `Vec` does. `position` moves past a value before that value is dropped,
/// so no value is dropped twice.
struct DropRemaining<'a, T> {
    ring: &'a RingBuffer<T>,
    position: usize,
    tail: usize,
}

impl<T> DropRemaining<'_, T> {
    fn drop_all(&mut self) {
        while self.position != self.tail {
            let position = self.position;
            self.position = self.ring.next(position);
            self.ring.slot(position).with_mut(|slot| {
                // SAFETY: `position` was in `[head, tail)`, so the slot holds a value
                // the consumer never took, and no handle is left to touch it.
                // `self.position` has already moved past it, so this value is
                // dropped once, even if its destructor panics and the guard's own
                // `Drop` takes over the loop.
                unsafe { slot.cast::<T>().drop_in_place() }
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

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        // Relaxed: this runs once, after both handles have dropped their `Arc`.
        // The last `Arc` decrement acquires every earlier one, so the final stores
        // of both counters, and every slot write before them, are visible here.
        let position = self.head.load(Ordering::Relaxed);
        // Relaxed: as above.
        let tail = self.tail.load(Ordering::Relaxed);
        DropRemaining {
            ring: self,
            position,
            tail,
        }
        .drop_all();
    }
}

impl<T> fmt::Debug for RingBuffer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RingBuffer")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

/// The writing end of a [`RingBuffer`].
pub struct Producer<T> {
    ring: Arc<RingBuffer<T>>,
    /// The producer's own copy of `ring.tail`; it is the only writer.
    tail: usize,
    /// The last `ring.head` this producer loaded. Never ahead of the real one.
    cached_head: usize,
}

// SAFETY: a `Producer` moves values of `T` to the thread that holds the consumer,
// so `T: Send` is required, and it is enough: the producer only writes slots
// outside `[head, tail)` and stores `tail`, the consumer only reads slots inside
// it and stores `head`, and the Release/Acquire pairs in `push` and `pop` order
// every hand-over of a slot. `Producer` stays `!Sync`; `push` takes `&mut self`.
unsafe impl<T: Send> Send for Producer<T> {}

impl<T> Producer<T> {
    /// Appends `value` unless the ring is full.
    ///
    /// # Errors
    ///
    /// Returns `value` back if the ring is full.
    pub fn push(&mut self, value: T) -> Result<(), T> {
        let ring = &*self.ring;
        let capacity = ring.capacity();
        if ring.len(self.cached_head, self.tail) == capacity {
            // Acquire: pairs with the Release store of `head` in `Consumer::pop`,
            // so the consumer's read of every slot before the loaded position
            // happens before this producer writes that slot again.
            self.cached_head = ring.head.load(Ordering::Acquire);
            if ring.len(self.cached_head, self.tail) == capacity {
                return Err(value);
            }
        }
        ring.slot(self.tail).with_mut(|slot| {
            // SAFETY: the slot is outside `[head, tail)` with `head` no older than
            // `cached_head`, so the consumer is not reading it, and its last read
            // happened before the Acquire load above. Only this producer writes
            // slots. A slot outside the range holds no value, so nothing leaks.
            unsafe { slot.cast::<T>().write(value) }
        });
        self.tail = ring.next(self.tail);
        // Release (see `PUBLISH`): pairs with the Acquire load of `tail` in
        // `Consumer::pop`, so the write above happens before the consumer reads it.
        ring.tail.store(self.tail, PUBLISH);
        Ok(())
    }

    /// The most values the ring can hold at once.
    #[must_use = "capacity has no side effects"]
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }
}

impl<T> fmt::Debug for Producer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Producer")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

/// The reading end of a [`RingBuffer`].
pub struct Consumer<T> {
    ring: Arc<RingBuffer<T>>,
    /// The consumer's own copy of `ring.head`; it is the only writer.
    head: usize,
    /// The last `ring.tail` this consumer loaded. Never ahead of the real one.
    cached_tail: usize,
}

// SAFETY: a `Consumer` receives values of `T` written on the producer's thread, so
// `T: Send` is required, and it is enough for the reasons given on `Producer`'s
// `Send` impl. `Consumer` stays `!Sync`; `pop` takes `&mut self`.
unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Consumer<T> {
    /// Removes the oldest value, or returns `None` if the ring is empty.
    pub fn pop(&mut self) -> Option<T> {
        let ring = &*self.ring;
        if self.head == self.cached_tail {
            // Acquire: pairs with the Release store of `tail` in `Producer::push`,
            // so the write of every slot before the loaded position happens before
            // this consumer reads it.
            self.cached_tail = ring.tail.load(Ordering::Acquire);
            if self.head == self.cached_tail {
                return None;
            }
        }
        let value = ring.slot(self.head).with(|slot| {
            // SAFETY: `head` is in `[head, cached_tail)`, so the slot holds a value
            // whose write happened before the Acquire load of `tail`. The producer
            // does not write it until `head` moves past it below, and moving `head`
            // also means this value is read only once.
            unsafe { slot.cast::<T>().read() }
        });
        self.head = ring.next(self.head);
        // Release: pairs with the Acquire load of `head` in `Producer::push`, so
        // the read above happens before the producer writes the slot again.
        ring.head.store(self.head, Ordering::Release);
        Some(value)
    }

    /// The most values the ring can hold at once.
    #[must_use = "capacity has no side effects"]
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }
}

impl<T> fmt::Debug for Consumer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Consumer")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

/// Negative `Send` and `Sync` checks (docs/design.md, "Rules for unsafe").
///
/// Each doctest is a `compile_fail` that asserts one trait bound on one type and
/// does nothing else. rustdoc checks the `E0277` on them only on a nightly
/// toolchain; on stable, which the CI doctests use, any compile error would make
/// one pass. What shows they fail for the bound is that the same assertion
/// compiles for types that meet it, in `handles_can_be_sent_to_other_threads`
/// (`src/spsc/tests.rs`).
#[cfg(doctest)]
mod negative_checks {
    /// `Producer<Rc<()>>` is not `Send`, because `Rc<()>` is not.
    ///
    /// ```compile_fail,E0277
    /// use latchless::spsc::Producer;
    /// use std::rc::Rc;
    ///
    /// fn assert_send<T: Send>() {}
    /// assert_send::<Producer<Rc<()>>>();
    /// ```
    fn producer_needs_t_send() {}

    /// `Consumer<Rc<()>>` is not `Send`, because `Rc<()>` is not.
    ///
    /// ```compile_fail,E0277
    /// use latchless::spsc::Consumer;
    /// use std::rc::Rc;
    ///
    /// fn assert_send<T: Send>() {}
    /// assert_send::<Consumer<Rc<()>>>();
    /// ```
    fn consumer_needs_t_send() {}

    /// `Producer<u8>` is not `Sync`: it is `Send`, so it can move to another
    /// thread, but two threads cannot share one.
    ///
    /// ```compile_fail,E0277
    /// use latchless::spsc::Producer;
    ///
    /// fn assert_sync<T: Sync>() {}
    /// assert_sync::<Producer<u8>>();
    /// ```
    fn producer_is_not_sync() {}

    /// `Consumer<u8>` is not `Sync`, for the same reason.
    ///
    /// ```compile_fail,E0277
    /// use latchless::spsc::Consumer;
    ///
    /// fn assert_sync<T: Sync>() {}
    /// assert_sync::<Consumer<u8>>();
    /// ```
    fn consumer_is_not_sync() {}
}
