# 0003: Known-wrong variants are cfg switches in the shipped code

Status: accepted, 2026-11-25

## Context

Several exit tests in the roadmap say that a deliberately broken copy of a structure must be rejected: the ring with one ordering weakened (M1), reclamation that frees one epoch early (M3). The point is to show the tests can tell right from wrong. A loom suite that passes everything proves nothing.

The broken copy has to be built and run in CI on every change, or it stops meaning anything the first time the real code is edited. There are three ways to keep one:

1. A second copy of the module under `tests/` or a test module, with the one change.
2. A mutation tool run in CI that edits the source.
3. A `cfg` switch next to the line in question, off unless CI turns it on.

A copy drifts from the code it copies: after a refactor it is testing a structure that no longer exists. A mutation tool is another dependency, and the useful mutations here are specific (one ordering, one check), not what a generic tool picks.

A second constraint shapes where the loom tests live. loom is a dev-dependency, built only under `--cfg loom`. Dev-dependencies are visible to the crate's own unit tests but not to the library as linked by integration tests, so the internal `sync` module can switch to loom only in `cfg(all(loom, test))`, and the loom scenarios are unit tests inside the crate (`src/*/loom_tests.rs`).

## Decision

Option 3. The ordering or check stays where it is, written as a private constant or a small branch with two `cfg` arms:

```rust
#[cfg(not(latchless_mutation = "spsc_relaxed_publish"))]
const PUBLISH: Ordering = Ordering::Release;
#[cfg(latchless_mutation = "spsc_relaxed_publish")]
const PUBLISH: Ordering = Ordering::Relaxed;
```

The allowed values are listed under `check-cfg` in `Cargo.toml`, so a typo in the name fails the build. CI builds each variant, checks that it compiles, runs the loom tests on it under `timeout`, and fails unless loom fails. The check is `scripts/expect-loom-failure.sh`. It reads the test output, not only the exit code, because a loom panic can make the test binary abort while unwinding instead of exiting with 101.

Other mutations are tried by hand during a milestone and recorded in the roadmap's Progress section. Only the one the exit test names is kept in CI.

## Consequences

- The variant under test is the shipped code with exactly one change, and it cannot drift.
- Each kept variant adds a few lines and a `cfg` to the shipped source. Nobody gets it by accident: the cfg is not a Cargo feature and has to be passed in `RUSTFLAGS`.
- A CI job whose success is a failing test is easy to misread. The job prints which outcome it expected, and a hang counts as a failure, not a rejection.
- Integration tests in `tests/` cannot use loom. If one ever needs to, loom has to become an optional normal dependency behind the same cfg, which needs this ADR revisited.
