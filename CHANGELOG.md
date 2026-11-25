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
