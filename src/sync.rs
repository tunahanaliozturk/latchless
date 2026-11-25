//! The one place atomics, cells and `Arc` are imported from.
//!
//! Under `cfg(all(loom, test))` everything resolves to loom's model types, so the
//! loom tests check the same code that ships. Otherwise it is `core` and `alloc`.
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

#[cfg(not(all(loom, test)))]
pub(crate) use alloc::sync::Arc;
#[cfg(not(all(loom, test)))]
pub(crate) use core::sync::atomic::{AtomicUsize, Ordering};

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
