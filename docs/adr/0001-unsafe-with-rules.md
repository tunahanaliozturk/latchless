# 0001: Unsafe code is allowed, under written rules

Status: accepted, 2026-11-17

## Context

The sibling project katman forbids `unsafe` outright. That is not possible here. A lock-free queue stores values in memory that several threads reach through shared references, hands out ownership of a slot based on an atomic counter, and frees nodes by raw pointer. None of that can be expressed in safe Rust without putting a lock or a reference count back in, which is the thing the crate exists to avoid.

So the question is not whether to use `unsafe` but how to keep each use small enough to check.

## Decision

- `unsafe` is allowed, and every use follows the rules below. The lints that can enforce a rule are set to deny in `Cargo.toml`.
- Every `unsafe` block has a `// SAFETY:` comment that names the invariant it relies on and where that invariant is established (`clippy::undocumented_unsafe_blocks`).
- One unsafe operation per block (`clippy::multiple_unsafe_ops_per_block`).
- The body of an `unsafe fn` is not an unsafe block by itself (`unsafe_op_in_unsafe_fn`).
- Every `unsafe fn`, `unsafe trait` and `unsafe impl` documents the caller's obligations (`clippy::missing_safety_doc`).
- An invariant is owned by one module. Fields that take part in it are private to that module, so safe code outside cannot break it.
- No `unsafe` to save a bounds check or a branch without a benchmark that shows it matters.
- Each structure has loom scenarios and runs under miri before it is merged.

## Consequences

- A reviewer can audit the crate by reading the `SAFETY` comments and checking each against its module, without holding the whole crate in mind.
- The code is longer. One operation per block and a comment per block roughly doubles the line count of the hot paths.
- The lints cannot tell whether a `SAFETY` comment is true. loom and miri are the check on that, within the scenarios they are given. A bug in an interleaving no test models remains possible, and the README does not claim otherwise.
