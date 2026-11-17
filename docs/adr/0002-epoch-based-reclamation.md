# 0002: Epoch-based reclamation, written in the crate

Status: accepted, 2026-11-17

## Context

A thread that pops a node from a lock-free stack cannot free it at once: another thread may have loaded the same pointer a moment earlier and be about to read through it. Something has to delay the free until no such thread can exist. The same mechanism prevents the ABA problem, because an address cannot be reused while a thread that saw it is still active.

Options:

1. **Epoch-based reclamation.** Threads announce when they are inside a critical section. Memory retired in one epoch is freed once every active thread has moved past it. Cheap reads: one store to pin, one to unpin. A stalled pinned thread blocks all reclamation.
2. **Hazard pointers.** A thread publishes each pointer it is about to dereference. Bounded unreclaimed memory. Every read costs a store and a re-validation, with a full memory barrier.
3. **Reference counting** on nodes. Simple to reason about, slow: every read writes to shared memory.
4. Depend on `crossbeam-epoch`.

## Decision

Option 1, implemented in this crate as the `epoch` module.

## Consequences

- Read paths stay cheap, which suits a queue and a stack where every operation is short.
- Unreclaimed memory is not bounded. A thread that pins and then blocks holds back every other thread's garbage. This is documented on `pin`, and guards are not `Send`, so a guard cannot be parked in another thread or held across an await point in a multi-threaded executor.
- Writing the scheme here, not depending on crossbeam-epoch, is most of the crate's difficulty and most of its purpose. It also means the benchmarks compare two independent implementations.
- Structures talk to `epoch` through a small interface (pin, load under guard, defer destroy). Hazard pointers could be added behind the same interface later; that is listed after 0.1 and not designed for now.
