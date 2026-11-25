//! latchless: lock-free queues and stacks.
//!
//! Implemented so far: [`spsc::RingBuffer`], a bounded ring with one producer and
//! one consumer, and [`mpmc::ArrayQueue`], a bounded queue for any number of
//! producers and consumers. The rest of the plan is in `docs/design.md` and
//! `docs/roadmap.md`.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod cache_padded;
pub mod mpmc;
pub mod spsc;
mod sync;
#[cfg(test)]
mod test_support;
