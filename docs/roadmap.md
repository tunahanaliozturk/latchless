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

- **M2**, 2026-11-25: done, with two limits on the loom part stated below; miri, ThreadSanitizer and aarch64 again only configured. `mpmc::ArrayQueue` in crossbeam's stamp layout, with one change to `push`: a push whose slot is still being written by the previous lap's push reports a full queue instead of waiting for it ([ADR 0005](adr/0005-array-queue-full-check.md)). A linearizability checker for queue histories, a Wing and Gong search with Lowe's memo, with its own tests of histories it must reject ([ADR 0004](adr/0004-linearizability-checking.md)). proptest as a dev-dependency.
  - Exit test, loom: `two_producers_and_two_consumers_agree_with_a_sequential_queue` passes, together with slot reuse across a lap on a one-slot queue, push-then-pop on two threads, its mirror pop-then-push on a full queue, a stale full answer meeting a stale empty one, and a queue dropped with values in it. The whole loom suite, ring included, took 143 seconds in release mode on the development laptop (559 in an earlier run while other builds shared the machine), nearly all of it the four-thread scenario; the rest finishes in under a minute. Limits: the two largest scenarios are explored with at most three preemptions per interleaving, because without a bound neither finished in twenty minutes; and no scenario lets two threads wait at once, because loom then explores the waiting threads yielding to each other without end and runs past its branch limit. The four-thread scenario therefore starts with one value in a three-slot queue, so only one consumer can end up waiting.
  - Exit test, histories: `recorded_histories_are_linearizable` checks 500 rounds of four threads making 24 random pushes and pops each; in a sample run 477 of the 500 rounds had calls from different threads overlapping, and the test fails if none do. The timestamps come from a `SeqCst` counter, which orders the calls more than the queue does, so these histories check the logic, not the orderings.
  - Exit test, proptest: `the_queue_behaves_like_a_bounded_vecdeque`, 512 cases of up to 64 steps on capacities 0 to 4, half of them starting on the last lap before the stamps wrap around `usize`.
  - The queue with its publishing store weakened to Relaxed is kept as a cfg variant and CI requires loom to reject it (`loom-rejects-known-wrong-mpmc`); all six queue scenarios report a causality violation on it locally. The plain tests pass on it.
  - Found on the way: in the first version of the loom scenarios the main thread made one of the pops, and a probe showed that pop never returned a value in any interleaving loom explored. Every caller is now a spawned thread, and each scenario fails unless some interleaving has a pop return a value a spawned thread pushed. The same probe on the ring's drop scenario showed its main-thread pop does see values.

  - Mutations: 21 by hand plus the cfg variant, each with the plain tests (five-minute timeout) and with every loom scenario but the four-thread one (five minutes); when those passed, the four-thread scenario ran on its own (twenty minutes). The first pass left two orderings uncaught: the fence in `push`'s full check removed, and the compare-and-swap on `tail` weakened to Relaxed. Two scenarios were added for them, `a_stale_full_and_a_stale_empty_answer_cannot_meet` and `a_push_after_a_pop_on_each_thread_cannot_find_the_queue_full`, and the table below is the second pass against the final suite. As in M1, the plain tests catch none of the weakened orderings on x86. Two weakenings are not caught and cannot be by these tools: the compare-and-swaps on `head` and `tail` from `SeqCst` to `AcqRel`. loom models a `SeqCst` compare-and-swap as `AcqRel` (ADR 0005, point 4), so for loom the mutated queue is the same program, and on x86 both compile to the same locked instruction. They stay `SeqCst` on the C11 argument in their comments: a `SeqCst` fence only orders against `SeqCst` operations.

| Mutation | Plain tests | loom (scenarios that fail) |
| --- | --- | --- |
| `push`: publishing store Release to Relaxed (the cfg variant kept in CI) | pass | caught: all six queue scenarios |
| `push`: slot stamp load Acquire to Relaxed | pass | caught: slot reuse, pop-then-push, stale full and empty |
| `pop`: slot stamp load Acquire to Relaxed | pass | caught: every scenario that ran (the four-thread one was not needed) |
| `pop`: store that frees the slot Release to Relaxed | pass | caught: slot reuse, pop-then-push, stale full and empty |
| `push`: no fence in the full check | pass | caught: stale full and empty, pop-then-push |
| `pop`: no fence in the empty check | pass | caught: push-then-pop, pop-then-push, stale full and empty |
| `push`: fence in the full check `SeqCst` to `AcqRel` | pass | caught: stale full and empty |
| `pop`: fence in the empty check `SeqCst` to `AcqRel` | pass | caught: stale full and empty |
| `push`: compare-and-swap on `tail` `SeqCst` to Relaxed | pass | caught: pop-then-push |
| `pop`: compare-and-swap on `head` `SeqCst` to Relaxed | pass | caught: push-then-pop, pop-then-push |
| `push`: compare-and-swap on `tail` `SeqCst` to `AcqRel` | pass | not caught; not catchable, see above |
| `pop`: compare-and-swap on `head` `SeqCst` to `AcqRel` | pass | not caught; not catchable, see above |
| `pop`: reports empty without reading `tail` | caught: `recorded_histories_are_linearizable` | caught: push-then-pop, pop-then-push, stale full and empty |
| `push`: reports full without reading `head` | caught: `recorded_histories_are_linearizable` | caught: stale full and empty, pop-then-push |
| `next` keeps the index bits when it moves to the next lap | caught: two failures, and four tests hang until the timeout | caught: pop-then-push, stale full and empty |
| `pop`: frees the slot with `+` instead of `wrapping_add` | caught: the proptest and the seeded run across the wrap overflow | pass (no scenario reaches the wrap) |
| `push`: publishes before writing the value | caught: `recorded_histories_are_linearizable` (and a hang) | caught: every scenario that ran, causality violation |
| `pop`: frees the slot before reading the value | caught: `recorded_histories_are_linearizable` (and a hang) | caught: slot reuse, pop-then-push, stale full and empty |
| `Drop`: drops nothing past `head` | caught: both drop tests | caught: the drop scenario |
| `Drop`: the guard does not finish after a panic | caught: `a_panicking_destructor_does_not_stop_the_other_values_being_dropped` | pass (no scenario panics in a destructor) |
| `push`: no check for zero capacity | caught: the zero-capacity test, the proptest, the seeded run | pass (no zero-capacity scenario) |
| `new`: no assert on the capacity | caught: `a_capacity_without_room_for_a_lap_is_refused` | pass |

## Order of work inside a milestone

1. Write the loom scenario and the sequential model first.
2. Implement with the strongest orderings, get everything green.
3. Weaken one ordering at a time, each with its written argument, re-running loom after each.
4. Add the known-wrong variant if the milestone has one, and see loom reject it.

## After 0.1

Not planned, listed so they are not forgotten: hazard pointers as a second reclamation scheme behind the same interface, a concurrent hash map, a segmented unbounded queue that allocates per block instead of per node.
