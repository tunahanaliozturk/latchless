# 0006: `ArrayQueue` stamps are 64 bits on every target

Status: accepted, 2026-11-27

## Context

A push reads `tail`, checks the stamp of the slot it points at, and then claims the slot with a compare-and-swap that only compares `tail`. A pop does the same with `head`. If the thread stalls between the check and the compare-and-swap, and in the meantime the other threads move `tail` all the way round its range until it holds the same value again, the compare-and-swap succeeds on a check that belongs to an earlier lap. The review of M2 wrote out both executions: two pushes write the same slot at once, or a pop reads a value another pop already took and returns it from an empty queue, which for an owning `T` is a double free. Both are reachable through the safe API.

How far "all the way round" is depends on the width of the stamps. `tail` only takes values whose low bits are an index below the capacity, so its range is `capacity` values per lap times `2^bits / one_lap` laps, where `one_lap` is the smallest power of two above the capacity. In M2 the stamps were `usize`. On a 32-bit target with capacity 1 that is 2^31 claims: at a hundred million a second, about 21 seconds of other threads working while one thread is descheduled, which an overloaded machine or a debugger can produce.

## Decision

- `head`, `tail` and every slot stamp are `AtomicU64` on every target, imported through `src/sync.rs` so that loom models them like the other atomics. No new dependency.
- `mpmc` is compiled only where `target_has_atomic = "64"`. On a target without 64-bit atomics the module is left out and `spsc` still builds; a `compile_error!` would have taken the ring down with it. `fence` and `spin` in `sync.rs`, which only the queue uses, are gated the same way.
- The capacity limit stays `usize::MAX / 2`. On a 64-bit target that is what keeps `capacity + 1` rounded up to a power of two inside a `u64`. On a 32-bit target the limit is far below anything the stamps need, and allocating the slots fails long before it anyway.
- A slot index is the stamp's low bits, below `one_lap`, which is at most `usize::MAX / 2 + 1`; it is turned into a `usize` with a cast that cannot truncate on either width, behind an `#[expect(clippy::cast_possible_truncation)]` that says so.

## The bound that remains

The stamp still wraps, so the failure is not impossible, only far away. For a stalled push to be fooled, `tail` has to advance through its whole range, `capacity * 2^64 / one_lap` successful claims, during that one stall. Because `one_lap` is at most twice the capacity, that is at least 2^63 for every capacity:

| Capacity | `one_lap` | Claims during one stall |
| --- | --- | --- |
| 1 | 2 | 2^63 |
| any capacity from 1 up | at most `2 * capacity` | at least 2^63 |
| `usize::MAX / 2` on a 64-bit target (2^63 - 1) | 2^63 | 2^64 - 2 |
| `usize::MAX / 2` on a 32-bit target (2^31 - 1) | 2^31 | 2^64 - 2^33 |

The same holds for a stalled pop and `head`. Since the queue holds at most `capacity` values, a stalled push also needs nearly as many pops to complete in that time, and the other way round. At a billion claims a second, which a contended queue does not reach, 2^63 is about 292 years.

crossbeam's `ArrayQueue` uses `usize` stamps, so on a 64-bit target it accepts this same bound, and on a 32-bit target the shorter one described above. The SAFETY comments on the write in `push` and the read in `pop` state the assumption and point here.

## Consequences

- On a 64-bit target nothing changes in the generated atomics. On a 32-bit target with 64-bit atomics (i686, armv7) each stamp operation is a double-width one, which is slower; that is the price of the bound.
- Targets without 64-bit atomics (thumbv6m, thumbv7m, riscv32imac and others) have no `mpmc` module.
- No test holds a thread across 2^63 claims. The evidence is the argument above, plus the model tests (`the_queue_behaves_like_a_bounded_vecdeque`, `a_seeded_run_across_the_lap_wrap_matches_the_model`) that start `head` and `tail` on the last lap before the 64-bit stamps wrap and check every result across the wrap. A CI job runs the tests on i686 to cover a 32-bit `usize`.
