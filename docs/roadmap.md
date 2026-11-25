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

## Progress

- **M0**, 2026-11-17: done.
- **M1**, 2026-11-25: done, except that miri, ThreadSanitizer and the aarch64 job have so far only been configured, not run locally (see below). `spsc::RingBuffer`, `CachePadded`, the `sync` module that switches to loom, and drop-counting element types. Three loom scenarios (three values through one slot and through two, and a ring dropped with values still in it, whichever handle goes last) pass in about 4 seconds in release mode. The ring with its publishing store weakened to Relaxed is built in CI behind a cfg and has to be rejected by loom ([ADR 0003](adr/0003-known-wrong-variants.md)); locally it is, with a causality violation on the slot. Eleven mutations were tried by hand, each with the plain tests and with loom, each under a ten-minute timeout. loom caught all eleven. The plain tests, including a threaded run of 200,000 values, caught none of the four weakened orderings on x86, which is the reason for the loom job and the aarch64 one. Dropping the empty check in `pop` is caught by both only as a crash reading an uninitialised value, not as a clean failure. Two problems in the test setup itself were found on the way: a failed assertion in the threaded test left the producer spinning on a full ring, so two mutations showed up as a timeout instead of a failure; and the loom test binary aborts while unwinding from loom's panic, so the CI check could not rely on exit code 101 and reads the output instead (`scripts/expect-loom-failure.sh`). miri and ThreadSanitizer need nightly, which is not installed where this was written; their CI jobs have not run yet.

Review round after M1:

- `RingBuffer::new` refuses a capacity above `usize::MAX / 2` with an assert, because `2 * capacity` is what the positions count up to and a zero-sized `T` does not bound it.
- Dropping the ring keeps dropping the remaining values when one destructor panics, through a guard whose own `Drop` finishes the loop, as `Vec` does.
- `compile_fail` doctests show that `Producer` and `Consumer` need `T: Send` and are not `Sync`.
- `scripts/expect-loom-failure.sh` builds the variant first and fails if the build fails, then wants a test shown running and failing, so a compile error no longer counts as loom rejecting it.

| Mutation | Plain tests | loom |
| --- | --- | --- |
| `push`: `tail` store Release to Relaxed (the cfg variant kept in CI) | pass | caught |
| `push`: the same by hand | pass | caught |
| `pop`: `tail` load Acquire to Relaxed | pass | caught |
| `pop`: `head` store Release to Relaxed | pass | caught |
| `push`: `head` load Acquire to Relaxed | pass | caught |
| `push`: no full check after reloading `head` | caught | caught |
| `pop`: no empty check after reloading `tail` | caught (crash) | caught (crash) |
| `push`: store `tail` before writing the slot | caught | caught |
| `pop`: store `head` before reading the slot | caught | caught |
| `Drop`: values left in the ring are not dropped | caught | caught |
| Positions wrap at the capacity instead of twice it | caught | caught |

## Order of work inside a milestone

1. Write the loom scenario and the sequential model first.
2. Implement with the strongest orderings, get everything green.
3. Weaken one ordering at a time, each with its written argument, re-running loom after each.
4. Add the known-wrong variant if the milestone has one, and see loom reject it.

## After 0.1

Not planned, listed so they are not forgotten: hazard pointers as a second reclamation scheme behind the same interface, a concurrent hash map, a segmented unbounded queue that allocates per block instead of per node.
