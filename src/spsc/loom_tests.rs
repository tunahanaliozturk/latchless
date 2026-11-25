//! loom scenarios for the ring. Run with
//! `RUSTFLAGS="--cfg loom" cargo test --lib --release`.

use super::RingBuffer;
use crate::test_support::{DropCounter, Tracked};
use loom::thread;
use std::vec::Vec;

/// Pushes every value, yielding to the consumer while the ring is full.
fn push_all(producer: &mut super::Producer<Tracked>, values: Vec<Tracked>) {
    for value in values {
        let mut value = value;
        while let Err(back) = producer.push(value) {
            value = back;
            thread::yield_now();
        }
    }
}

#[test]
fn values_pass_through_a_one_slot_ring_in_order() {
    loom::model(|| {
        let counter = DropCounter::new();
        let (mut producer, mut consumer) = RingBuffer::new(1);
        // Three values through one slot: positions 0, 1, 0, so the counters wrap.
        let values: Vec<Tracked> = (0..3).map(|value| counter.track(value)).collect();
        let pusher = thread::spawn(move || push_all(&mut producer, values));
        let mut expected = 0;
        while expected < 3 {
            match consumer.pop() {
                Some(tracked) => {
                    assert_eq!(tracked.value, expected);
                    expected += 1;
                }
                None => thread::yield_now(),
            }
        }
        pusher.join().unwrap();
        drop(consumer);
        assert_eq!(counter.drops(), 3);
    });
}

#[test]
fn values_pass_through_a_two_slot_ring_in_order() {
    loom::model(|| {
        let counter = DropCounter::new();
        let (mut producer, mut consumer) = RingBuffer::new(2);
        let values: Vec<Tracked> = (0..3).map(|value| counter.track(value)).collect();
        let pusher = thread::spawn(move || push_all(&mut producer, values));
        let mut expected = 0;
        while expected < 3 {
            match consumer.pop() {
                Some(tracked) => {
                    assert_eq!(tracked.value, expected);
                    expected += 1;
                }
                None => thread::yield_now(),
            }
        }
        pusher.join().unwrap();
        drop(consumer);
        assert_eq!(counter.drops(), 3);
    });
}

#[test]
fn values_left_behind_are_dropped_once_whichever_handle_goes_last() {
    loom::model(|| {
        let counter = DropCounter::new();
        let (mut producer, mut consumer) = RingBuffer::new(2);
        let values: Vec<Tracked> = (0..2).map(|value| counter.track(value)).collect();
        let pusher = thread::spawn(move || {
            push_all(&mut producer, values);
            drop(producer);
        });
        if let Some(tracked) = consumer.pop() {
            assert_eq!(tracked.value, 0);
        }
        drop(consumer);
        pusher.join().unwrap();
        assert_eq!(counter.drops(), 2);
    });
}
