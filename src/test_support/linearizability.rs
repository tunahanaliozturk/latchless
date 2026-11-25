//! A linearizability checker for histories of a bounded FIFO queue.
//!
//! A history is one list of [`Event`]s per thread, in the order that thread made
//! the calls. Each event records the tick at which the call started, the tick at
//! which it returned, and what it did. The history is linearizable if the calls
//! can be put in one sequential order that keeps every call that returned before
//! another started ahead of it, and that gives the recorded results when run
//! against a bounded `VecDeque`.
//!
//! The search is Wing and Gong's, with the memo Lowe added to it. A thread's calls
//! never overlap, so the calls already placed are always a prefix of each
//! thread's list. A search state is the length of each prefix plus the queue
//! contents, and a state already explored is not explored again. Values must be
//! distinct, or a pop could be matched to the wrong push.

use std::collections::{HashSet, VecDeque};
use std::vec::Vec;

/// What one call did.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Call {
    /// A push of `value`, and whether the queue took it.
    Push { value: usize, accepted: bool },
    /// A pop, and the value it returned.
    Pop { got: Option<usize> },
}

/// One call, with when it started and when it returned.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub(crate) started: u64,
    pub(crate) returned: u64,
    pub(crate) call: Call,
}

impl Event {
    /// An event that overlaps every other unordered one. loom has no clock, so
    /// its histories use these: any order that keeps each thread's own order is
    /// accepted, which checks sequential consistency instead of linearizability.
    /// Ticks 0 and 1 are left for a call made before all of them, and ticks from 4
    /// on for calls made after.
    pub(crate) fn unordered(call: Call) -> Self {
        Self {
            started: 2,
            returned: 3,
            call,
        }
    }
}

/// Hands out the ticks for [`Event`]s from one counter shared by all threads.
///
/// A `SeqCst` counter, so a call whose return tick is lower than another's start
/// tick really did finish first. Taking the ticks orders the calls more than the
/// queue itself would, so these histories check the algorithm's logic (lost,
/// duplicated or reordered values, wrong full and empty answers), not its memory
/// orderings; loom and the aarch64 job are for those.
#[cfg(not(loom))]
#[derive(Debug, Default)]
pub(crate) struct Clock(core::sync::atomic::AtomicU64);

#[cfg(not(loom))]
impl Clock {
    /// Runs one call and records it.
    pub(crate) fn record(&self, call: impl FnOnce() -> Call) -> Event {
        let started = self.tick();
        let call = call();
        let returned = self.tick();
        Event {
            started,
            returned,
            call,
        }
    }

    fn tick(&self) -> u64 {
        self.0.fetch_add(1, core::sync::atomic::Ordering::SeqCst)
    }
}

/// Whether `threads` is a linearizable history of a FIFO queue that holds up to
/// `capacity` values and starts empty.
pub(crate) fn is_linearizable(capacity: usize, threads: &[Vec<Event>]) -> bool {
    let mut search = Search {
        capacity,
        threads,
        seen: HashSet::new(),
    };
    search.explore(&mut std::vec![0; threads.len()], &mut VecDeque::new())
}

struct Search<'a> {
    capacity: usize,
    threads: &'a [Vec<Event>],
    /// States already explored and found to lead nowhere.
    seen: HashSet<(Vec<usize>, VecDeque<usize>)>,
}

impl Search<'_> {
    /// Whether the calls not yet placed can follow `done`, the number of calls
    /// placed from each thread, with `queue` holding what they left.
    fn explore(&mut self, done: &mut Vec<usize>, queue: &mut VecDeque<usize>) -> bool {
        let pending = || {
            self.threads
                .iter()
                .zip(done.iter())
                .filter_map(|(events, &placed)| events.get(placed))
        };
        // The earliest return among the calls left. A call that started after it
        // cannot come next.
        let Some(horizon) = pending().map(|event| event.returned).min() else {
            return true;
        };
        if !self.seen.insert((done.clone(), queue.clone())) {
            return false;
        }
        for thread in 0..self.threads.len() {
            let Some(&event) = self.threads[thread].get(done[thread]) else {
                continue;
            };
            if event.started > horizon {
                continue;
            }
            let found = match event.call {
                Call::Push {
                    value,
                    accepted: true,
                } => {
                    if queue.len() < self.capacity {
                        queue.push_back(value);
                        let found = self.place(thread, done, queue);
                        queue.pop_back();
                        found
                    } else {
                        false
                    }
                }
                Call::Push {
                    accepted: false, ..
                } => queue.len() == self.capacity && self.place(thread, done, queue),
                Call::Pop { got: Some(value) } => {
                    if queue.front() == Some(&value) {
                        queue.pop_front();
                        let found = self.place(thread, done, queue);
                        queue.push_front(value);
                        found
                    } else {
                        false
                    }
                }
                Call::Pop { got: None } => queue.is_empty() && self.place(thread, done, queue),
            };
            if found {
                return true;
            }
        }
        false
    }

    /// Places the next call of `thread`, whose effect is already on `queue`, and
    /// explores from there.
    fn place(&mut self, thread: usize, done: &mut Vec<usize>, queue: &mut VecDeque<usize>) -> bool {
        done[thread] += 1;
        let found = self.explore(done, queue);
        done[thread] -= 1;
        found
    }
}

/// The checker is checked too: a checker that accepts everything would make the
/// stress tests pass whatever the queue does.
#[cfg(not(loom))]
mod tests {
    use super::{Call, Event, is_linearizable};
    use std::vec;

    fn at(started: u64, returned: u64, call: Call) -> Event {
        Event {
            started,
            returned,
            call,
        }
    }

    fn push(value: usize) -> Call {
        Call::Push {
            value,
            accepted: true,
        }
    }

    fn full(value: usize) -> Call {
        Call::Push {
            value,
            accepted: false,
        }
    }

    fn pop(got: Option<usize>) -> Call {
        Call::Pop { got }
    }

    #[test]
    fn a_sequential_history_that_matches_the_queue_is_accepted() {
        let history = [vec![
            at(0, 1, push(1)),
            at(2, 3, push(2)),
            at(4, 5, full(3)),
            at(6, 7, pop(Some(1))),
            at(8, 9, pop(Some(2))),
            at(10, 11, pop(None)),
        ]];
        assert!(is_linearizable(2, &history));
    }

    #[test]
    fn a_value_that_was_never_pushed_is_rejected() {
        let history = [vec![at(0, 1, push(1))], vec![at(2, 3, pop(Some(7)))]];
        assert!(!is_linearizable(2, &history));
    }

    #[test]
    fn a_value_popped_twice_is_rejected() {
        let history = [
            vec![at(0, 1, push(1))],
            vec![at(2, 5, pop(Some(1)))],
            vec![at(3, 4, pop(Some(1)))],
        ];
        assert!(!is_linearizable(2, &history));
    }

    #[test]
    fn values_out_of_order_are_rejected_when_the_pushes_did_not_overlap() {
        let history = [
            vec![at(0, 1, push(1)), at(2, 3, push(2))],
            vec![at(4, 5, pop(Some(2)))],
        ];
        assert!(!is_linearizable(2, &history));
    }

    #[test]
    fn values_out_of_order_are_accepted_when_the_pushes_overlapped() {
        let history = [
            vec![at(0, 3, push(1))],
            vec![at(1, 2, push(2))],
            vec![at(4, 5, pop(Some(2))), at(6, 7, pop(Some(1)))],
        ];
        assert!(is_linearizable(2, &history));
    }

    #[test]
    fn full_on_a_queue_that_cannot_be_full_is_rejected() {
        let history = [vec![at(0, 1, push(1)), at(2, 3, full(2))]];
        assert!(!is_linearizable(2, &history));
    }

    #[test]
    fn empty_on_a_queue_that_cannot_be_empty_is_rejected() {
        let history = [vec![at(0, 1, push(1))], vec![at(2, 3, pop(None))]];
        assert!(!is_linearizable(2, &history));
    }

    #[test]
    fn empty_is_accepted_when_the_pop_overlapped_the_push() {
        let history = [vec![at(0, 3, push(1))], vec![at(1, 2, pop(None))]];
        assert!(is_linearizable(2, &history));
    }

    #[test]
    fn unordered_events_keep_each_threads_own_order() {
        // Each thread pushes, then pops. Both pops finding the queue empty fits
        // no sequential order, overlapping or not.
        let both_empty = [
            vec![Event::unordered(push(1)), Event::unordered(pop(None))],
            vec![Event::unordered(push(2)), Event::unordered(pop(None))],
        ];
        assert!(!is_linearizable(2, &both_empty));
        let one_each = [
            vec![Event::unordered(push(1)), Event::unordered(pop(Some(1)))],
            vec![Event::unordered(push(2)), Event::unordered(pop(Some(2)))],
        ];
        assert!(is_linearizable(2, &one_each));
    }
}
