# kilitsiz

Lock-free queues and stacks for Rust, with their own epoch-based memory reclamation.

*Kilitsiz* is Turkish for "without a lock".

## Status

Planning. The repository holds the design and the roadmap; no functional code yet.

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

Probably not: [crossbeam](https://github.com/crossbeam-rs/crossbeam) is older, faster in places and far more widely tested. kilitsiz exists to show each algorithm in a small amount of code where every atomic ordering and every `unsafe` block carries its reason.

## Licence

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
