use super::ArrayQueue;
use crate::test_support::linearizability::{Call, Clock, Event, is_linearizable};
use crate::test_support::{DropCounter, Tracked, Xorshift};
use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use proptest::prelude::*;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Barrier};
use std::thread;
use std::vec::Vec;

/// Runs `pushes` (true for a push, false for a pop) against a queue starting at
/// lap `lap` and against a bounded `VecDeque`, and checks every result matches.
fn matches_the_model(capacity: usize, lap: u64, pushes: &[bool]) {
    let queue = ArrayQueue::starting_at_lap(capacity, lap);
    let mut model = VecDeque::new();
    for (step, &push) in pushes.iter().enumerate() {
        if push {
            let result = queue.push(step);
            if model.len() < capacity {
                assert_eq!(result, Ok(()), "step {step}");
                model.push_back(step);
            } else {
                assert_eq!(result, Err(step), "step {step}");
            }
        } else {
            assert_eq!(queue.pop(), model.pop_front(), "step {step}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Sequences up to 63 steps long on capacities 0 to 4 cover the full and
    /// empty edges and several laps of every slot. About half the runs start on the
    /// last lap before the 64-bit stamps wrap around.
    #[test]
    #[cfg_attr(miri, ignore = "proptest is slow under miri; the seeded test below runs there")]
    fn the_queue_behaves_like_a_bounded_vecdeque(
        capacity in 0_usize..=4,
        near_the_wrap in any::<bool>(),
        pushes in proptest::collection::vec(any::<bool>(), 0..64),
    ) {
        let lap = if near_the_wrap { u64::MAX } else { 0 };
        matches_the_model(capacity, lap, &pushes);
    }
}

#[test]
fn a_seeded_run_across_the_lap_wrap_matches_the_model() {
    let steps = if cfg!(miri) { 200 } else { 5_000 };
    for capacity in 0..=5 {
        let mut random = Xorshift(0x9E37_79B9_7F4A_7C15 + capacity as u64);
        let pushes: Vec<bool> = (0..steps).map(|_| random.next() % 2 == 0).collect();
        matches_the_model(capacity, 0, &pushes);
        matches_the_model(capacity, u64::MAX, &pushes);
    }
}

#[test]
fn a_full_queue_hands_the_value_back() {
    let queue = ArrayQueue::new(2);
    assert_eq!(queue.push('a'), Ok(()));
    assert_eq!(queue.push('b'), Ok(()));
    assert_eq!(queue.push('c'), Err('c'));
    assert_eq!(queue.pop(), Some('a'));
    assert_eq!(queue.push('c'), Ok(()));
    assert_eq!(queue.pop(), Some('b'));
    assert_eq!(queue.pop(), Some('c'));
    assert_eq!(queue.pop(), None);
}

#[test]
fn a_zero_capacity_queue_is_always_full_and_always_empty() {
    let queue = ArrayQueue::new(0);
    assert_eq!(queue.capacity(), 0);
    assert_eq!(queue.push(1), Err(1));
    assert_eq!(queue.pop(), None);
}

#[test]
#[should_panic(expected = "capacity must be at most")]
fn a_capacity_without_room_for_a_lap_is_refused() {
    let _ = ArrayQueue::<()>::new(usize::MAX / 2 + 1);
}

#[test]
fn zero_sized_values_are_counted_like_any_other() {
    let queue = ArrayQueue::new(3);
    for _ in 0..3 {
        assert_eq!(queue.push(()), Ok(()));
    }
    assert_eq!(queue.push(()), Err(()));
    assert_eq!(queue.pop(), Some(()));
    assert_eq!(queue.push(()), Ok(()));
}

#[test]
fn values_left_in_the_queue_are_dropped_once_with_it() {
    let capacity = 4;
    let counter = DropCounter::new();
    let queue = ArrayQueue::new(capacity);
    // Two laps first, so the values left behind straddle the wrap.
    for value in 0..=2 * capacity {
        assert!(queue.push(counter.track(value)).is_ok());
        assert_eq!(queue.pop().map(|tracked| tracked.value), Some(value));
    }
    for value in 0..capacity {
        assert!(queue.push(counter.track(value)).is_ok());
    }
    drop(queue.pop());
    assert_eq!(counter.drops(), 2 * capacity + 2);
    drop(queue);
    assert_eq!(counter.drops(), 2 * capacity + 1 + capacity);
}

/// Counts its own drops, and panics in its destructor if told to.
struct Fuse {
    explode: bool,
    drops: Arc<AtomicUsize>,
}

impl Drop for Fuse {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        assert!(!self.explode, "destructor panic, on purpose");
    }
}

#[test]
fn a_panicking_destructor_does_not_stop_the_other_values_being_dropped() {
    let counts: Vec<Arc<AtomicUsize>> = (0..3).map(|_| Arc::new(AtomicUsize::new(0))).collect();
    let queue = ArrayQueue::new(3);
    for (index, drops) in counts.iter().enumerate() {
        let fuse = Fuse {
            explode: index == 0,
            drops: Arc::clone(drops),
        };
        assert!(queue.push(fuse).is_ok());
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| drop(queue)));
    assert!(outcome.is_err());
    let drops: Vec<usize> = counts.iter().map(|c| c.load(Ordering::SeqCst)).collect();
    assert_eq!(drops, [1, 1, 1]);
}

/// Sets its flag when dropped, also while unwinding from a failed assertion.
struct SetOnDrop<'a>(&'a AtomicBool);

impl Drop for SetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn values_cross_threads_once_each_and_in_order_per_producer() {
    let per_producer = if cfg!(miri) { 100 } else { 100_000 };
    let counter = DropCounter::new();
    let queue = ArrayQueue::new(8);
    // Set when a consumer stops, also by a failed assertion, so the producers do
    // not spin on a full queue forever and the other consumer does not wait for
    // values they gave up on, which would turn a failure into a hang.
    let consumer_gone = AtomicBool::new(false);
    let popped = AtomicUsize::new(0);
    thread::scope(|scope| {
        for producer in 0..2 {
            let values: Vec<Tracked> = (0..per_producer)
                .map(|index| counter.track(producer * per_producer + index))
                .collect();
            let (queue, consumer_gone) = (&queue, &consumer_gone);
            scope.spawn(move || {
                for value in values {
                    let mut value = value;
                    while let Err(back) = queue.push(value) {
                        if consumer_gone.load(Ordering::SeqCst) {
                            return;
                        }
                        value = back;
                        thread::yield_now();
                    }
                }
            });
        }
        for _ in 0..2 {
            scope.spawn(|| {
                let _stop_producers = SetOnDrop(&consumer_gone);
                // The last value seen from each producer, so a value that arrives
                // before an earlier one from the same producer is caught.
                let mut last: [Option<usize>; 2] = [None, None];
                while popped.load(Ordering::SeqCst) < 2 * per_producer {
                    let Some(tracked) = queue.pop() else {
                        if consumer_gone.load(Ordering::SeqCst) {
                            return;
                        }
                        thread::yield_now();
                        continue;
                    };
                    popped.fetch_add(1, Ordering::SeqCst);
                    let producer = tracked.value / per_producer;
                    assert!(last[producer] < Some(tracked.value), "out of order");
                    last[producer] = Some(tracked.value);
                }
            });
        }
    });
    assert_eq!(popped.into_inner(), 2 * per_producer);
    assert_eq!(counter.drops(), 2 * per_producer);
}

#[test]
fn recorded_histories_are_linearizable() {
    let rounds = if cfg!(miri) { 2 } else { 500 };
    let calls = if cfg!(miri) { 6 } else { 24 };
    let threads = 4;
    let mut overlapping_rounds = 0;
    for round in 0..rounds {
        let capacity = 1 + round % 3;
        let queue = ArrayQueue::new(capacity);
        let clock = Clock::default();
        let start = Barrier::new(threads);
        let history: Vec<Vec<Event>> = thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|thread| {
                    let (queue, clock, start) = (&queue, &clock, &start);
                    scope.spawn(move || {
                        let seed = (round * threads + thread) as u64;
                        let mut random =
                            Xorshift(0x2545_F491_4F6C_DD1D ^ seed.wrapping_mul(0x9E37));
                        start.wait();
                        (0..calls)
                            .map(|call| {
                                if random.next() % 2 == 0 {
                                    // Distinct values, so each pop names one push.
                                    let value = thread * calls + call;
                                    clock.record(|| Call::Push {
                                        value,
                                        accepted: queue.push(value).is_ok(),
                                    })
                                } else {
                                    clock.record(|| Call::Pop { got: queue.pop() })
                                }
                            })
                            .collect()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        });
        assert!(
            is_linearizable(capacity, &history),
            "round {round}, capacity {capacity}: {history:#?}"
        );
        if overlaps(&history) {
            overlapping_rounds += 1;
        }
    }
    // A run whose threads never overlapped checked only sequential behaviour.
    assert!(overlapping_rounds > 0, "no round had two calls overlap");
}

/// Whether two calls from different threads were in flight at the same time.
fn overlaps(history: &[Vec<Event>]) -> bool {
    history.iter().enumerate().any(|(index, mine)| {
        history[index + 1..].iter().flatten().any(|theirs| {
            mine.iter()
                .any(|event| event.started < theirs.returned && theirs.started < event.returned)
        })
    })
}

#[test]
fn the_queue_can_be_shared_when_t_is_send_but_not_sync() {
    fn assert_send_and_sync<T: Send + Sync>() {}
    assert_send_and_sync::<ArrayQueue<Cell<u8>>>();
    assert_send_and_sync::<ArrayQueue<Tracked>>();
}
