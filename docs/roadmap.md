# Roadmap

Each milestone ends with a test that has to pass. A milestone is not done because the code exists.

| # | Milestone | Scope | Exit test |
| --- | --- | --- | --- |
| M0 | Scaffold | Repository, lints, CI, design, roadmap. | CI green on an empty crate. |
| M1 | Test harness and SPSC ring | `spsc::RingBuffer`, `CachePadded`, the internal atomics module that switches to loom. CI jobs for loom, miri, ThreadSanitizer and aarch64. Drop-counting test types. | loom passes on push/pop scenarios. A copy of the ring with one ordering weakened on purpose is rejected by loom. miri clean. |
| M2 | Bounded MPMC queue | `mpmc::ArrayQueue`. Linearizability checker for queue histories. | loom passes with two producers and two consumers. Recorded stress histories are linearizable. Wrap-around and full/empty edges covered by proptest. |
| M3 | Epoch reclamation | `epoch`: participants, pin and guard, deferred destruction, collection. `std` and no-`std` participant handling. | loom finds no use after free with a reader pinned across a retire. A variant that frees one epoch early is rejected. Every retired node is freed by the time all handles are dropped. |
| M4 | Unbounded structures | `Stack` (Treiber) and `Queue` (Michael-Scott) on `epoch`. Compile-fail tests for `Send` and `Sync` bounds. | loom, miri and ThreadSanitizer clean. Histories linearizable. No leak and no double drop with panicking element types. |
| M5 | Numbers | criterion benchmarks against crossbeam, rtrb and mutex baselines, 1 to 16 threads. One round of tuning from the results. | `docs/benchmarks.md` reproducible from the listed commands, including the cases where latchless loses. |
| M6 | Release | Rustdoc with an ordering argument per structure, examples, README with measured results. | `cargo doc` has no missing item. Tag v0.1.0 and publish to crates.io. |

## Order of work inside a milestone

1. Write the loom scenario and the sequential model first.
2. Implement with the strongest orderings, get everything green.
3. Weaken one ordering at a time, each with its written argument, re-running loom after each.
4. Add the known-wrong variant if the milestone has one, and see loom reject it.

## After 0.1

Not planned, listed so they are not forgotten: hazard pointers as a second reclamation scheme behind the same interface, a concurrent hash map, a segmented unbounded queue that allocates per block instead of per node.
