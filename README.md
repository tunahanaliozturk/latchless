# latchless

Lock-free queues and stacks for Rust, with their own epoch-based memory reclamation.

A latch is what database engineers call a short lock on an in-memory structure. The structures here take none.

## Status

M1 of the [roadmap](docs/roadmap.md) is built: `spsc::RingBuffer` and the test harness the other structures will use. The miri and ThreadSanitizer jobs are configured but have not run yet. Not released; nothing is on crates.io.

What is tested for the ring today:

| Claim | Test |
| --- | --- |
| Values come out in the order they went in, a full ring hands the value back, and the counters wrap correctly. | `the_ring_behaves_like_a_bounded_vecdeque` in `src/spsc/tests.rs` checks 20,000 random steps per capacity from 0 to 5 against a `VecDeque`. |
| No data race and no lost or reordered value in any interleaving of the scenarios given to loom. | `src/spsc/loom_tests.rs`: three values through rings of one and two slots, and a ring dropped with values still in it. |
| The loom tests notice a wrong ordering. | CI builds the ring with its publishing store weakened to Relaxed and requires loom to fail (`loom-rejects-known-wrong` in `.github/workflows/ci.yml`, [ADR 0003](docs/adr/0003-known-wrong-variants.md)). |
| Every value is dropped exactly once, including the ones left in the ring. | Drop-counting element types in `values_left_in_the_ring_are_dropped_once_with_the_last_handle` and the loom scenarios. |
| No undefined behaviour, no race at scale, correct on weak memory hardware. | CI jobs run the tests under miri, under ThreadSanitizer, and on an aarch64 runner. None of the three has run yet. |

loom checks only the scenarios it is given, two threads and three values. A bug that needs a longer run to show is not ruled out by it; the threaded test and the aarch64 job are there for that, and they are evidence, not proof.

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
