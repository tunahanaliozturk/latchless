# 0005: `ArrayQueue` follows crossbeam's layout, with a wider full check and loom-sized scenarios

Status: accepted, 2026-11-25

## Context

`mpmc::ArrayQueue` is Dmitry Vyukov's bounded queue. crossbeam's `ArrayQueue` is the best-reviewed Rust version of it: `head` and `tail` are stamps with the slot index in the low bits and a lap count above it, each slot carries a stamp of its own, the compare-and-swap that claims a slot is `SeqCst`, and the full and empty checks put a `SeqCst` fence before reading the other index. latchless takes that layout and those orderings, because the argument for each is already worked out and the design rule is to keep the stronger ordering when the argument for a weaker one is not short.

Four things came up while building it.

1. In crossbeam, a push that finds its slot still waiting for the previous lap's push to finish writing (the slot's stamp is exactly one lap behind the tail) spins until that push publishes. At that point the queue is full: the slow push has already claimed the last free position. The push could say so instead of waiting.
2. loom does not finish a model in which two threads can wait at the same time. A pop waits for a push that has claimed a slot but not published it; a push waits for a pop that has claimed a value but not freed the slot. While the thread they wait for is not scheduled, two waiting threads can yield to each other for ever, and loom explores that. A preemption bound does not stop it: switching away from a thread that has just yielded is not counted as a preemption. With two producers and two consumers on a one-slot queue, loom ran past its limit of 1,000 branches in one execution, and with the limit raised to 20,000 it had not finished after fifteen minutes.
3. In the first version of the scenarios the main thread made one of the pops. A probe showed that pop never returned a value in any interleaving loom explored, so the scenario was weaker than its name. Moved to a spawned thread, the same pop did return values. The cause inside loom was not found.
4. loom treats `SeqCst` loads, stores and compare-and-swaps as `AcqRel` (its README says so); only `SeqCst` fences are modelled in full. So loom checks the compare-and-swaps more weakly than the C11 model: it can report a failure that cannot happen, but cannot miss one because of this. It also means a compare-and-swap weakened from `SeqCst` to `AcqRel` looks the same to loom.

## Decision

- The push full check runs when the slot's stamp is one lap behind the tail in either way: holding the previous lap's value, or still being written by the previous lap's push. In both cases the push fences, loads `head`, and reports full only if `head` is exactly one lap behind the tail, the same condition crossbeam uses. The answer rests on that condition, not on the stamp, so it is as sound as crossbeam's; the change is only that a push on a full queue no longer waits for a slow push. A pop still waits for a claimed push to publish, because reporting empty there would not be linearizable.
- Zero capacity is allowed, as in `spsc::RingBuffer`: every push fails and every pop finds the queue empty. crossbeam panics instead. A capacity above `usize::MAX / 2` is refused with an assert, because `capacity + 1` rounded up to a power of two has to fit in a `usize`.
- Every loom caller is a spawned thread, each scenario fails unless some explored interleaving has a pop return a value a spawned thread pushed, and no scenario allows two threads to wait at once:
  - two producers and two consumers (the exit test): three slots and one value pushed before the threads start, so the producers never wrap and only the consumer that loses the race for that value can wait;
  - slot reuse across a lap: one slot, two producers and one consumer;
  - each of two threads pushes, then pops (the fence in the empty check);
  - each of two threads pops, then pushes, on a full queue of two slots (the mirror of the last one; the case the Release half of the compare-and-swap on `tail` is for);
  - one thread pops twice while the other pushes twice into two slots holding one value (a stale full answer meeting a stale empty one, the case the fence in the full check is for).
- The first two are explored with at most three preemptions per interleaving, the bound loom's documentation suggests and tokio's CI uses. Without a bound neither finished in twenty minutes on the development laptop; with it the four-thread scenario takes two to nine minutes, depending on what else the machine is doing, and the rest of the suite under a minute. `LOOM_MAX_PREEMPTIONS` overrides the bound for a longer run. The two-thread scenarios, and the drop scenario, are exhaustive.
- The queue gets a known-wrong variant in CI, as [ADR 0003](0003-known-wrong-variants.md) describes, although the M2 exit test does not name one: `mpmc_relaxed_publish` weakens the store that publishes a written slot to Relaxed, and the `loom-rejects-known-wrong-mpmc` job requires loom to fail on it.
- Under loom, `sync::spin` yields instead of issuing a spin-loop hint, so a waiting thread lets the others run.

## Consequences

- The queue differs from crossbeam's in one branch of `push`. The proptest model test, the loom scenarios and the stress histories all go through it.
- loom does not cover two consumers waiting on the same unpublished slot, or a push and a pop waiting at the same time. The threaded stress tests do, without the guarantee of every interleaving.
- The bounded scenarios do not cover interleavings with four or more preemptions.
- The `SeqCst` compare-and-swaps are justified by the C11 argument in their comments, not by loom. The mutation table in the roadmap records which weakenings loom catches and which it cannot see.
- The M1 ring's loom scenarios use the main thread as the consumer. A probe on the drop scenario showed that main-thread pop does see values, so the problem in point 3 did not show up there, but the ring's scenarios were not rewritten to the spawned-only form.
