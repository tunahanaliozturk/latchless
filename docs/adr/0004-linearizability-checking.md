# 0004: Queue histories are checked for linearizability by an exhaustive search in the tests

Status: accepted, 2026-11-25

## Context

The M2 exit test asks for recorded stress histories of `mpmc::ArrayQueue` to be linearizable. A stress test that only counts values finds lost and duplicated values, but not a pop that reports the queue empty while it holds a value, a push that reports it full while it has room, or two values that come out in an order no interleaving of the calls explains. Those need the whole history held against a sequential queue.

Checking linearizability is NP-complete in general. The options were a crate (none of the maintained ones is `no_std`-friendly or small, and each would be a new dev-dependency), a port of a checker such as porcupine, or a small search written for this one model.

Two more questions: where the timestamps come from, and what loom, which has no clock, can check.

## Decision

- `src/test_support/linearizability.rs` holds a Wing and Gong search with Lowe's memo, specialised to a bounded FIFO queue. Each thread's calls are sequential, so the calls already placed in a candidate order are a prefix of each thread's list; a state is the prefix lengths plus the queue contents, and a state is never explored twice. Values in a history are distinct, so a pop names exactly one push.
- Timestamps come from one `SeqCst` counter that every thread increments before and after each call. That is sound: a call whose return tick is below another's start tick did finish first. It also adds ordering the queue does not have, so the stress histories test the algorithm's logic, not its memory orderings. loom, ThreadSanitizer and the aarch64 job are for the orderings.
- Under loom every event is marked unordered, which keeps each thread's own order and nothing else. The check becomes sequential consistency, which is the strongest thing a model without real time can state, and it is enough to catch a pair of pops that both miss a value.
- The checker has its own tests, with histories that must be rejected (a value never pushed, a value popped twice, values out of order after non-overlapping pushes, full and empty answers that cannot be right) and ones that must be accepted because the calls overlapped. A checker that accepts everything would make every stress test pass.

## Consequences

- No new dependency for the checker. proptest is added for the sequential model tests, which the roadmap names; it is a dev-dependency only, with its default features off.
- The search is exponential in the worst case. With four threads of 24 calls each and a capacity of three or less it runs 500 rounds in well under a second; larger histories should be split into rounds the same way.
- M4 can reuse the checker for the unbounded `Queue` by passing a capacity no history reaches. The stack needs its own sequential model.
