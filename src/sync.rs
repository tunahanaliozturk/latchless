//! The one place atomics, cells and `Arc` are imported from.
//!
//! Everything resolves to loom's model types only when both `--cfg loom` is set
//! and the crate is built for its own tests, `cfg(all(loom, test))`, so the loom
//! tests check the same code that ships. In every other build, `--cfg loom`
//! included, it is `core` and `alloc`.
//! loom's `UnsafeCell` hands out raw pointers through closures instead of
//! `get()`, so the `core` side wraps its cell in the same shape.

#[cfg(all(loom, test))]
pub(crate) use loom::{
    cell::UnsafeCell,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
// Only `mpmc` uses these, and it is built only with 64-bit atomics.
#[cfg(all(loom, test, target_has_atomic = "64"))]
pub(crate) use loom::sync::atomic::{AtomicU64, fence};

#[cfg(not(all(loom, test)))]
pub(crate) use alloc::sync::Arc;
#[cfg(all(not(all(loom, test)), target_has_atomic = "64"))]
pub(crate) use core::sync::atomic::{AtomicU64, fence};
#[cfg(not(all(loom, test)))]
pub(crate) use core::sync::atomic::{AtomicUsize, Ordering};

/// Called in a loop that waits for another thread to finish a step.
///
/// A spin-loop hint normally. Under loom it yields, so the model runs the other
/// threads instead of exploring the same wait over and over.
#[cfg(target_has_atomic = "64")]
pub(crate) fn spin() {
    #[cfg(all(loom, test))]
    loom::thread::yield_now();
    #[cfg(not(all(loom, test)))]
    core::hint::spin_loop();
}

/// `core::cell::UnsafeCell` with loom's closure-based interface.
#[cfg(not(all(loom, test)))]
#[derive(Debug)]
pub(crate) struct UnsafeCell<T>(core::cell::UnsafeCell<T>);

#[cfg(not(all(loom, test)))]
impl<T> UnsafeCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(core::cell::UnsafeCell::new(value))
    }

    /// Runs `f` with a pointer the caller may only read through.
    pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
        f(self.0.get())
    }

    /// Runs `f` with a pointer the caller may read and write through.
    pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }
}
