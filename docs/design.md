# kilitsiz design

This document is the contract. Code that disagrees with it is a bug in one of the two, and the fix says which.

## Goals

1. A small set of lock-free structures that are correct in every interleaving, with the evidence in the repository.
2. Every `unsafe` block and every atomic ordering explained where it is written.
3. Works without the standard library, given an allocator.
4. Honest benchmarks against the crates people actually use.

## Non-goals

- Beating crossbeam. Matching it within a stated margin is the aim; where kilitsiz is slower, the benchmarks say so.
- A concurrent hash map, skip list or channel with blocking and `select`. They are listed for later.
- Wait-freedom, except for the single-producer single-consumer ring where it comes for free.
- Async wakers. The structures are non-blocking primitives; a `try_push` that fails returns the value.

## Contents

### `spsc::RingBuffer<T>`

A bounded ring with one producer handle and one consumer handle. The split into two handles is what makes single-producer and single-consumer a compile-time fact, not a comment. Each side caches the other side's index and reloads it only when the cache says the ring is full or empty, which keeps the two sides off each other's cache line in the common case.

### `mpmc::ArrayQueue<T>`

A bounded queue for any number of producers and consumers, after Dmitry Vyukov's design. Each slot carries a sequence number that says whether it is ready for a push or a pop at a given lap. A push claims a slot by compare-and-swap on the tail, writes the value, then publishes the slot by advancing its sequence number. No allocation after construction.

### `epoch`

The unbounded structures free nodes that other threads may still be reading. `epoch` decides when that is safe.

- A global epoch counter and a list of participants.
- A thread pins itself before touching shared pointers; the `Guard` it gets back unpins on drop. Pointers loaded under a guard borrow from it, so the compiler rejects a pointer that outlives the pin.
- A retired node goes into the retiring thread's local bag, stamped with the current epoch.
- The global epoch advances when every pinned participant has seen the current one. A bag stamped two epochs ago can be freed.

Known weakness, stated up front: a thread that stays pinned stops reclamation for everyone, and memory grows until it unpins. Hazard pointers bound that at a cost per read. The choice is recorded in [ADR 0002](adr/0002-epoch-based-reclamation.md).

### `Stack<T>` and `Queue<T>`

A Treiber stack and a Michael-Scott queue, both on `epoch`. These are the standard algorithms; the value is in the reclamation being right and tested. Both face the ABA problem in their compare-and-swap loops, and `epoch` is what rules it out: a node's address cannot be reused while any thread that could have seen it is still pinned.

## Rules for unsafe

Recorded in [ADR 0001](adr/0001-unsafe-with-rules.md). In short:

- Every `unsafe` block has a `// SAFETY:` comment naming the invariant that makes it sound. clippy's `undocumented_unsafe_blocks` is set to deny.
- One unsafe operation per block, so each comment answers for exactly one thing.
- Every `unsafe fn` and `unsafe impl` documents what the caller must guarantee.
- `Send` and `Sync` impls are written by hand with their bounds justified, and there are compile-fail tests for the cases that must not compile (for example a queue of `Rc<T>` shared between threads).
- Unsafe code stays inside the module that owns the invariant. No `pub` function can be used to break one.

## Rules for atomic orderings

- No `SeqCst` by default. Each load, store and compare-and-swap uses the weakest ordering that is correct, with a comment that says which other operation it must be ordered against.
- If the argument for a weaker ordering does not fit in a short comment, the stronger one is used and the benchmark decides whether it matters.
- Indices that different threads write live on separate cache lines, with a `CachePadded` wrapper defined in the crate.

## Testing

| Layer | Tool | What it finds |
| --- | --- | --- |
| Model checking | loom, under `--cfg loom` | Wrong orderings, lost updates, use after free, in every interleaving of small scenarios (two or three threads, a handful of operations). |
| Undefined behaviour | miri | Aliasing violations, uninitialised reads, leaks, in single-threaded and small multi-threaded tests. |
| Data races at scale | ThreadSanitizer on nightly, Linux | Races in stress tests too large for loom. |
| Sequential behaviour | proptest against `VecDeque` and `Vec` | Wrong results, off-by-one at wrap-around and capacity. |
| Linearizability | Recorded concurrent histories checked against the sequential model | Results that no sequential order explains. |
| Drop behaviour | Drop-counting and panicking element types | Leaks, double drops, broken state after a panic in `T::drop`. |
| Weak memory hardware | Stress tests on aarch64 in CI | Ordering bugs that x86's strong memory model hides. |

loom cannot explore large scenarios, and stress tests cannot explore every interleaving. Neither is enough alone, which is why both run.

Atomics and cells are imported through one internal module that resolves to `core` or to loom, so the code under test is the code that ships.

## `no_std`

The crate is `#![no_std]` and needs `alloc`. `epoch` needs a per-thread participant handle; with the default `std` feature it lives in a thread-local, and without it the caller registers and passes a handle explicitly.

## Benchmarks

Published in `docs/benchmarks.md` at M5, with the hardware and the commands. criterion, 1 to 16 threads:

- `spsc::RingBuffer` against `rtrb` and `std::sync::mpsc`.
- `mpmc::ArrayQueue` against `crossbeam_queue::ArrayQueue` and `Mutex<VecDeque>`.
- `Queue` against `crossbeam_queue::SegQueue`.
- `Stack` against `Mutex<Vec>`.

No target numbers are claimed before they are measured.

## Dependencies

None at runtime. Development only: loom, proptest, criterion, trybuild for the compile-fail tests, and the crates benchmarked against. `cargo deny` checks licences and advisories in CI.
