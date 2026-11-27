# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows [Semantic Versioning](https://semver.org/).

## Unreleased

- Repository scaffold, design document and roadmap.
- `spsc::RingBuffer`: a bounded wait-free ring with one producer and one consumer handle.
- Test harness: atomics and cells switch to loom under `--cfg loom`, drop-counting element types, CI jobs for loom, miri, ThreadSanitizer and aarch64, and a CI check that loom rejects a deliberately weakened ring.
- `RingBuffer::new` panics on a capacity above `usize::MAX / 2` instead of risking an overflowing position range.
- Dropping a ring whose value panics in its destructor still drops the other values.
- Compile-fail doctests for the handles not being `Sync` and needing `T: Send`.
- The loom rejection check fails when the known-wrong variant does not build.
- `mpmc::ArrayQueue`: a bounded lock-free queue for any number of producers and consumers, after Vyukov.
- A linearizability checker for queue histories, used by the threaded stress test and, in an unordered mode, by the loom scenarios.
- proptest as a dev-dependency, for the sequential model tests.
- A second CI check that loom rejects a deliberately weakened queue.
- Two loom scenarios for the queue's full check: a stale full answer meeting a stale empty one, and each thread popping, then pushing, on a full queue. Without them loom missed the fence in the full check being removed and the compare-and-swap on `tail` being weakened to Relaxed.
