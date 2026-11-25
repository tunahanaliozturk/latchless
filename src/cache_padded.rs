//! Keeps a value on its own cache line.

use core::ops::Deref;

/// Aligns and pads `T` to a cache line, so two values written by different
/// threads do not share one and invalidate each other on every write.
///
/// 128 bytes on x86-64 and aarch64, where the prefetcher pulls lines in pairs
/// (x86-64) or the line itself is 128 bytes (some aarch64 cores); 64 elsewhere.
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), repr(align(128)))]
#[cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    repr(align(64))
)]
#[derive(Debug, Default)]
pub(crate) struct CachePadded<T>(T);

impl<T> CachePadded<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T> Deref for CachePadded<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::CachePadded;
    use core::mem::{align_of, size_of};

    #[test]
    fn two_padded_values_never_share_a_cache_line() {
        let pair = [CachePadded::new(0_u8), CachePadded::new(0_u8)];
        let first = core::ptr::from_ref(&*pair[0]).addr();
        let second = core::ptr::from_ref(&*pair[1]).addr();
        assert!(second - first >= 64);
        assert!(align_of::<CachePadded<u8>>() >= 64);
        assert_eq!(size_of::<CachePadded<u8>>(), align_of::<CachePadded<u8>>());
    }
}
