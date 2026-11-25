//! loom scenarios for the queue. Run with
//! `RUSTFLAGS="--cfg loom" cargo test --lib --release`.
//!
//! Each thread makes one or two calls and never retries, so every scenario has a
//! bounded number of calls. The two largest scenarios are explored with at most
//! three preemptions per interleaving, the bound loom's documentation suggests
//! and tokio's CI uses; the others are explored without a bound. The results are checked with the linearizability
//! checker in its unordered mode: loom has no clock, so the check is that some
//! order of the calls that keeps each thread's own order explains every result.
//!
//! No scenario lets two threads wait at the same time. A pop waits for a push
//! that has claimed a slot but not yet published it, and a push waits for a pop
//! that has claimed a value but not yet freed the slot. loom lets waiting threads
//! yield to each other without end while the thread they wait for never runs, and
//! a preemption bound does not stop that, because switching away from a thread
//! that yielded costs no preemption. A scenario with two possible waiters runs
//! past loom's branch limit (docs/adr/0005-array-queue-full-check.md).

use super::ArrayQueue;
use crate::test_support::linearizability::{Call, Event, is_linearizable};
use crate::test_support::{DropCounter, Tracked};
use core::sync::atomic::{AtomicBool, Ordering};
use loom::sync::Arc;
use loom::thread;
use std::vec::Vec;

fn push(queue: &ArrayQueue<Tracked>, value: Tracked) -> Event {
    let number = value.value;
    Event::unordered(Call::Push {
        value: number,
        accepted: queue.push(value).is_ok(),
    })
}

fn pop(queue: &ArrayQueue<Tracked>) -> Event {
    Event::unordered(Call::Pop {
        got: queue.pop().map(|tracked| tracked.value),
    })
}

/// Pops whatever is left after every thread has been joined. These pops come
/// after all the other calls, so they get ticks later than any unordered event.
fn drain(queue: &ArrayQueue<Tracked>) -> Vec<Event> {
    let mut events = Vec::new();
    let mut tick = 4;
    loop {
        let got = queue.pop().map(|tracked| tracked.value);
        events.push(Event {
            started: tick,
            returned: tick + 1,
            call: Call::Pop { got },
        });
        tick += 2;
        if got.is_none() {
            return events;
        }
    }
}

/// One scenario: the queue's capacity, an optional value pushed before any
/// thread starts, each thread's calls (`Some(value)` to push that value, `None`
/// to pop), and the most preemptions loom may put in one interleaving.
struct Scenario {
    capacity: usize,
    prefill: Option<usize>,
    threads: &'static [&'static [Option<usize>]],
    /// `None` explores every interleaving. `LOOM_MAX_PREEMPTIONS`, when set,
    /// overrides this, so a longer run can lift the bound.
    preemption_bound: Option<usize>,
}

impl Scenario {
    /// Runs every interleaving within the preemption bound, checks the results
    /// against a sequential queue, and counts the drops.
    ///
    /// Every caller is a spawned thread. In the first version of these scenarios
    /// the main thread made one of the pops, and a probe showed that pop never
    /// returned a value in any interleaving loom explored; moved to a spawned
    /// thread, it did. Why was not tracked down. So every caller is spawned, and
    /// the check fails unless some explored interleaving has a pop return a value
    /// that a spawned thread pushed.
    fn check(self) {
        let Self {
            capacity,
            prefill,
            threads,
            preemption_bound,
        } = self;
        let mut builder = loom::model::Builder::new();
        if builder.preemption_bound.is_none() {
            builder.preemption_bound = preemption_bound;
        }
        let raced = std::sync::Arc::new(AtomicBool::new(false));
        let saw_a_race = std::sync::Arc::clone(&raced);
        builder.check(move || {
            let counter = DropCounter::new();
            let queue = Arc::new(ArrayQueue::new(capacity));
            let mut history = Vec::new();
            if let Some(value) = prefill {
                assert!(queue.push(counter.track(value)).is_ok());
                history.push(std::vec![Event {
                    started: 0,
                    returned: 1,
                    call: Call::Push {
                        value,
                        accepted: true,
                    },
                }]);
            }
            let handles: Vec<_> = threads
                .iter()
                .map(|calls| {
                    let queue = Arc::clone(&queue);
                    let calls: Vec<Option<Tracked>> = calls
                        .iter()
                        .map(|call| call.map(|value| counter.track(value)))
                        .collect();
                    thread::spawn(move || {
                        calls
                            .into_iter()
                            .map(|call| match call {
                                Some(value) => push(&queue, value),
                                None => pop(&queue),
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            let joined: Vec<Vec<Event>> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            if joined.iter().flatten().any(|event| {
                matches!(event.call, Call::Pop { got: Some(value) } if Some(value) != prefill)
            }) {
                saw_a_race.store(true, Ordering::SeqCst);
            }
            history.extend(joined);
            history.push(drain(&queue));
            assert!(is_linearizable(capacity, &history), "{history:?}");
            drop(queue);
            let pushed = threads.iter().copied().flatten().flatten().count();
            assert_eq!(counter.drops(), pushed + usize::from(prefill.is_some()));
        });
        assert!(
            raced.load(Ordering::SeqCst),
            "no interleaving had a pop return a value a spawned thread pushed"
        );
    }
}

/// The exit test of M2: two producers and two consumers, one call each. One
/// value is in the queue before they start and there are three slots, so the
/// consumers race for that value, the producers race for the next two slots, and
/// the consumer that loses is the only thread that can end up waiting on a push.
#[test]
fn two_producers_and_two_consumers_agree_with_a_sequential_queue() {
    Scenario {
        capacity: 3,
        prefill: Some(9),
        threads: &[&[Some(0)], &[Some(1)], &[None], &[None]],
        // Unbounded, this did not finish in twenty minutes.
        preemption_bound: Some(3),
    }
    .check();
}

/// Two producers and one consumer on a queue of one slot, so a second push
/// succeeds only after the pop has freed the slot and the stamps have moved to
/// the next lap. This is where the Release store that frees a slot has to come
/// before the next lap's write.
#[test]
fn a_slot_freed_by_a_pop_is_reused_safely_by_the_next_push() {
    Scenario {
        capacity: 1,
        prefill: None,
        threads: &[&[Some(0)], &[Some(1)], &[None]],
        // Unbounded, this did not finish in ten minutes; bounded, it takes seconds.
        preemption_bound: Some(3),
    }
    .check();
}

/// Each thread pushes, then pops. At least one of the pops finds a value; both
/// finding the queue empty fits no sequential order. This is the case the `SeqCst`
/// fences are for.
#[test]
fn a_pop_after_a_push_on_each_thread_cannot_find_both_empty() {
    Scenario {
        capacity: 2,
        prefill: None,
        threads: &[&[Some(0), None], &[Some(1), None]],
        preemption_bound: None,
    }
    .check();
}

/// Values still in the queue are dropped once, by whichever thread lets go of
/// the queue last. The main thread lets go first, so the last one is spawned.
#[test]
fn values_left_behind_are_dropped_once_whichever_thread_goes_last() {
    loom::model(|| {
        let counter = DropCounter::new();
        let queue = Arc::new(ArrayQueue::new(2));
        let values = [counter.track(0), counter.track(1)];
        let pusher = {
            let queue = Arc::clone(&queue);
            thread::spawn(move || {
                for value in values {
                    assert!(queue.push(value).is_ok());
                }
                drop(queue);
            })
        };
        let popper = {
            let queue = Arc::clone(&queue);
            thread::spawn(move || {
                if let Some(tracked) = queue.pop() {
                    assert_eq!(tracked.value, 0);
                }
                drop(queue);
            })
        };
        drop(queue);
        pusher.join().unwrap();
        popper.join().unwrap();
        assert_eq!(counter.drops(), 2);
    });
}
