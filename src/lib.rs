//! latchless: queues and stacks that take no locks.
//!
//! Implemented so far: [`spsc::RingBuffer`], a bounded ring with one producer and
//! one consumer, and [`mpmc::ArrayQueue`], a bounded queue for any number of
//! producers and consumers. The rest of the plan is in `docs/design.md` and
//! `docs/roadmap.md`.
//!
//! `mpmc` needs 64-bit atomics and is left out on targets without them
//! (docs/adr/0006-array-queue-64-bit-stamps.md).
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod cache_padded;
#[cfg(target_has_atomic = "64")]
pub mod mpmc;
pub mod spsc;
mod sync;
#[cfg(test)]
mod test_support;
