use super::{Consumer, Producer, RingBuffer};
use crate::test_support::{DropCounter, Tracked};
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::VecDeque;
use std::thread;
use std::vec::Vec;

/// A small xorshift generator, so the model test needs no dependency and every
/// run sees the same sequence.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn the_ring_behaves_like_a_bounded_vecdeque() {
    let steps = if cfg!(miri) { 500 } else { 20_000 };
    for capacity in 0..=5 {
        let (mut producer, mut consumer) = RingBuffer::new(capacity);
        let mut model = VecDeque::new();
        let mut random = Xorshift(0x9E37_79B9_7F4A_7C15 + capacity as u64);
        for step in 0..steps {
            if random.next() % 2 == 0 {
                let pushed = producer.push(step);
                if model.len() < capacity {
                    assert_eq!(pushed, Ok(()));
                    model.push_back(step);
                } else {
                    assert_eq!(pushed, Err(step));
                }
            } else {
                assert_eq!(consumer.pop(), model.pop_front());
            }
        }
    }
}

#[test]
fn a_full_ring_hands_the_value_back() {
    let (mut producer, mut consumer) = RingBuffer::new(2);
    assert_eq!(producer.push('a'), Ok(()));
    assert_eq!(producer.push('b'), Ok(()));
    assert_eq!(producer.push('c'), Err('c'));
    assert_eq!(consumer.pop(), Some('a'));
    assert_eq!(producer.push('c'), Ok(()));
    assert_eq!(consumer.pop(), Some('b'));
    assert_eq!(consumer.pop(), Some('c'));
    assert_eq!(consumer.pop(), None);
}

#[test]
fn a_zero_capacity_ring_is_always_full_and_always_empty() {
    let (mut producer, mut consumer) = RingBuffer::new(0);
    assert_eq!(producer.capacity(), 0);
    assert_eq!(producer.push(1), Err(1));
    assert_eq!(consumer.pop(), None);
}

#[test]
#[should_panic(expected = "capacity must be at most")]
fn a_capacity_whose_doubled_range_overflows_is_refused() {
    let _ = RingBuffer::<()>::new(usize::MAX / 2 + 1);
}

#[test]
fn zero_sized_values_are_counted_like_any_other() {
    let (mut producer, mut consumer) = RingBuffer::new(3);
    for _ in 0..3 {
        assert_eq!(producer.push(()), Ok(()));
    }
    assert_eq!(producer.push(()), Err(()));
    assert_eq!(consumer.pop(), Some(()));
    assert_eq!(producer.push(()), Ok(()));
}

fn fill_and_half_drain(
    capacity: usize,
    counter: &DropCounter,
) -> (Producer<Tracked>, Consumer<Tracked>) {
    let (mut producer, mut consumer) = RingBuffer::new(capacity);
    // Two laps first, so the values left behind straddle the wrap.
    for value in 0..=2 * capacity {
        assert!(producer.push(counter.track(value)).is_ok());
        assert_eq!(consumer.pop().map(|tracked| tracked.value), Some(value));
    }
    for value in 0..capacity {
        assert!(producer.push(counter.track(value)).is_ok());
    }
    drop(consumer.pop());
    (producer, consumer)
}

#[test]
fn values_left_in_the_ring_are_dropped_once_with_the_last_handle() {
    let capacity = 4;
    let total = 2 * capacity + 1 + capacity;

    let counter = DropCounter::new();
    let (producer, consumer) = fill_and_half_drain(capacity, &counter);
    drop(producer);
    assert_eq!(counter.drops(), 2 * capacity + 2);
    drop(consumer);
    assert_eq!(counter.drops(), total);

    let counter = DropCounter::new();
    let (producer, consumer) = fill_and_half_drain(capacity, &counter);
    drop(consumer);
    assert_eq!(counter.drops(), 2 * capacity + 2);
    drop(producer);
    assert_eq!(counter.drops(), total);
}

/// Sets its flag when dropped, also while unwinding from a failed assertion.
struct SetOnDrop<'a>(&'a AtomicBool);

impl Drop for SetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn values_cross_threads_in_order_and_are_each_dropped_once() {
    let count = if cfg!(miri) { 200 } else { 200_000 };
    let counter = DropCounter::new();
    let (mut producer, mut consumer) = RingBuffer::new(16);
    let values: Vec<Tracked> = (0..count).map(|value| counter.track(value)).collect();
    // Set when the consumer stops, also by a failed assertion, so the producer
    // does not spin on a full ring forever and turn a failure into a hang.
    let consumer_gone = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            for value in values {
                let mut value = value;
                while let Err(back) = producer.push(value) {
                    if consumer_gone.load(Ordering::SeqCst) {
                        return;
                    }
                    value = back;
                    thread::yield_now();
                }
            }
        });
        let _stop_producer = SetOnDrop(&consumer_gone);
        let mut expected = 0;
        while expected < count {
            match consumer.pop() {
                Some(tracked) => {
                    assert_eq!(tracked.value, expected);
                    expected += 1;
                }
                None => thread::yield_now(),
            }
        }
    });
    assert_eq!(counter.drops(), count);
}

#[test]
fn handles_can_be_sent_to_other_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Producer<Tracked>>();
    assert_send::<Consumer<Tracked>>();
}
