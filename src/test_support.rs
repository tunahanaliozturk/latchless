//! Shared test helpers: element types that count drops, a seeded generator, and
//! the linearizability checker.
//!
//! The drop counter is a plain `core` atomic behind an `alloc` `Arc`, also under loom:
//! loom runs its threads one at a time, and the count is read only after every
//! thread has been joined, so it needs no modelling.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

pub(crate) mod linearizability;

/// Hands out [`Tracked`] values and counts how many have been dropped.
#[derive(Debug, Default)]
pub(crate) struct DropCounter(Arc<AtomicUsize>);

impl DropCounter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn track(&self, value: usize) -> Tracked {
        Tracked {
            value,
            drops: Arc::clone(&self.0),
        }
    }

    pub(crate) fn drops(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// A value that bumps its counter once when dropped.
#[derive(Debug)]
pub(crate) struct Tracked {
    pub(crate) value: usize,
    drops: Arc<AtomicUsize>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

/// A small xorshift generator, so the model tests need no dependency and every
/// run sees the same sequence.
#[cfg(not(loom))]
#[derive(Debug)]
pub(crate) struct Xorshift(pub(crate) u64);

#[cfg(not(loom))]
impl Xorshift {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}
