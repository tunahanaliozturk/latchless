# latchless

Lock-free queues and stacks for Rust, with their own epoch-based memory reclamation.

A latch is what database engineers call a short lock on an in-memory structure. The structures here take none.

## Status

M2 of the [roadmap](docs/roadmap.md) is built: `spsc::RingBuffer`, `mpmc::ArrayQueue` and the test harness the other structures will use. The miri and ThreadSanitizer jobs are configured but have not run yet. Not released; nothing is on crates.io.

What is tested for the ring today:

| Claim | Test |
| --- | --- |
| Values come out in the order they went in, a full ring hands the value back, and the counters wrap correctly. | `the_ring_behaves_like_a_bounded_vecdeque` in `src/spsc/tests.rs` checks 20,000 random steps per capacity from 0 to 5 against a `VecDeque`. |
| No data race and no lost or reordered value in any interleaving of the scenarios given to loom. | `src/spsc/loom_tests.rs`: three values through rings of one and two slots, and a ring dropped with values still in it. |
| The loom tests notice a wrong ordering. | CI builds the ring with its publishing store weakened to Relaxed and requires loom to fail (`loom-rejects-known-wrong` in `.github/workflows/ci.yml`, [ADR 0003](docs/adr/0003-known-wrong-variants.md)). |
| Every value is dropped exactly once, including the ones left in the ring. | Drop-counting element types in `values_left_in_the_ring_are_dropped_once_with_the_last_handle` and the loom scenarios. |
| No undefined behaviour, no race at scale, correct on weak memory hardware. | CI jobs run the tests under miri, under ThreadSanitizer, and on an aarch64 runner. None of the three has run yet. |

What is tested for the queue today:

| Claim | Test |
| --- | --- |
| A single thread sees a bounded FIFO queue: values in order, a full queue hands the value back, an empty one returns `None`, across many laps of every slot and across the lap counter wrapping around `usize`. | `the_queue_behaves_like_a_bounded_vecdeque` in `src/mpmc/tests.rs`, a proptest of sequences up to 64 steps on capacities 0 to 4, half of them starting on the last lap before the wrap. |
| Concurrent histories have a sequential explanation. | `recorded_histories_are_linearizable`: 500 rounds of four threads making 24 random calls each, every history checked by the search in `src/test_support/linearizability.rs` ([ADR 0004](docs/adr/0004-linearizability-checking.md)). The checker has its own tests with histories it must reject. |
| No data race on a slot and no result that fits no sequential order, with two producers and two consumers. | `src/mpmc/loom_tests.rs`. The two scenarios with three or more threads are explored with up to three preemptions per interleaving, not exhaustively ([ADR 0005](docs/adr/0005-array-queue-full-check.md)). |
| The loom tests notice a wrong ordering. | CI builds the queue with its publishing store weakened to Relaxed and requires loom to fail (`loom-rejects-known-wrong-mpmc`). By hand, loom caught every weakened ordering tried except the two compare-and-swaps going from `SeqCst` to `AcqRel`, which loom cannot tell apart; the table is in the [roadmap](docs/roadmap.md). |
| Every value is dropped exactly once, also when a destructor panics. | `values_left_in_the_queue_are_dropped_once_with_it`, `a_panicking_destructor_does_not_stop_the_other_values_being_dropped`, and drop counts in every loom scenario. |
| `ArrayQueue<T>` is `Send` and `Sync` exactly when `T: Send`. | `compile_fail` doctests in `src/mpmc.rs` for `Rc` and `MutexGuard`, and a test that `ArrayQueue<Cell<u8>>` is `Sync`. |

loom checks only the scenarios it is given: two to four threads making one to three calls each. A bug that needs a longer run to show is not ruled out by it; the threaded tests and the aarch64 job are there for that, and they are evidence, not proof.

- [docs/design.md](docs/design.md): what is in the crate, the rules for `unsafe`, and how each structure will be tested.
- [docs/roadmap.md](docs/roadmap.md): milestones, each with an exit test.
- [docs/adr/](docs/adr/): decisions that are hard to reverse.

## What it will contain

| Structure | Algorithm | Progress guarantee |
| --- | --- | --- |
| `spsc::RingBuffer` | Bounded ring, one producer and one consumer | Wait-free |
| `mpmc::ArrayQueue` | Bounded queue with a sequence number per slot (Vyukov) | Lock-free |
| `epoch` | Epoch-based reclamation: pin, defer, collect | Lock-free, reclamation can be delayed by a stalled thread |
| `Stack` | Treiber stack on `epoch` | Lock-free |
| `Queue` | Michael-Scott unbounded queue on `epoch` | Lock-free |

## What it will promise

| Promise | How it will be tested |
| --- | --- |
| No data race, no use after free, no double free, in any interleaving of a bounded scenario. | loom explores every interleaving of small scenarios for each structure. |
| No undefined behaviour in the unsafe code. | The test suite runs under miri. Every `unsafe` block states the invariant it relies on; clippy rejects a block without one. |
| Every operation is linearizable. | Concurrent histories are recorded and checked against a sequential model. |
| Nothing is leaked and every element is dropped once. | Drop-counting element types in all tests, including panics in user `Drop` impls. |
| Memory orderings are right on weak memory hardware, not only on x86. | loom models the orderings; CI also runs the stress tests on aarch64. |
| Speed is measured, not claimed. | criterion benchmarks against crossbeam and a mutex around `VecDeque`, published with the commands. |

## Should you use it

Probably not: [crossbeam](https://github.com/crossbeam-rs/crossbeam) is older, faster in places and far more widely tested. latchless exists to show each algorithm in a small amount of code where every atomic ordering and every `unsafe` block carries its reason.

## Licence

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
