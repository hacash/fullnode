//! Bounded ordered handoff from parallel block decoders to the sync applier.

use std::sync::{Condvar, Mutex};

use base::BlkPkg;
use sys::Ret;

/// One decoded block waiting to be applied, or the error that stopped its slot.
pub(crate) type Slot = Ret<BlkPkg>;

/// Producers may finish out of order; `take` waits for the requested sequence.
pub(crate) struct Ring {
    slots: Mutex<RingState>,
    not_empty: Condvar,
    not_full: Condvar,
    capacity: usize,
}

struct RingState {
    items: Vec<Option<Slot>>,
    /// Reservations include work still being decoded, ensuring the earliest
    /// unfinished sequence always retains capacity to publish.
    in_flight: usize,
    next: u64,
    end: Option<u64>,
    stopped: bool,
}

impl Ring {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            slots: Mutex::new(RingState {
                items: (0..capacity).map(|_| None).collect(),
                in_flight: 0,
                next: 0,
                end: None,
                stopped: false,
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            capacity,
        }
    }

    /// Poison-tolerant lock. Every pipeline stage shares this mutex, so one
    /// stage panicking must not wedge the others: the state each method
    /// mutates is fully updated before its guard is released, so a poisoned
    /// lock only ever reflects a completed transition, never a torn one. Any
    /// panic behind a lock here is a programming error the sync join reports.
    fn lock(&self) -> std::sync::MutexGuard<'_, RingState> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn reserve(&self) -> bool {
        let mut state = self.lock();
        while !state.stopped && state.in_flight >= self.capacity {
            state = self.not_full.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        if state.stopped {
            return false;
        }
        state.in_flight += 1;
        true
    }

    pub(crate) fn release(&self) {
        let mut state = self.lock();
        debug_assert!(state.in_flight > 0);
        state.in_flight -= 1;
        self.not_full.notify_one();
    }

    pub(crate) fn publish_reserved(&self, seq: u64, slot: Slot) -> bool {
        let idx = (seq % self.capacity as u64) as usize;
        let mut state = self.lock();
        if state.stopped {
            debug_assert!(state.in_flight > 0);
            state.in_flight -= 1;
            self.not_full.notify_one();
            return false;
        }
        debug_assert!(seq < state.next + self.capacity as u64);
        debug_assert!(state.items[idx].is_none());
        state.items[idx] = Some(slot);
        self.not_empty.notify_one();
        true
    }

    pub(crate) fn take(&self, seq: u64) -> Option<Slot> {
        let idx = (seq % self.capacity as u64) as usize;
        let mut state = self.lock();
        loop {
            if let Some(slot) = state.items[idx].take() {
                state.next = seq + 1;
                debug_assert!(state.in_flight > 0);
                state.in_flight -= 1;
                self.not_full.notify_one();
                return Some(slot);
            }
            if state.stopped {
                return None;
            }
            if state.end.is_some_and(|end| seq >= end) {
                return None;
            }
            state = self
                .not_empty
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    pub(crate) fn close(&self, total: u64) {
        let mut state = self.lock();
        state.end = Some(total);
        self.not_empty.notify_one();
    }

    pub(crate) fn stop(&self) {
        let mut state = self.lock();
        state.stopped = true;
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }

    #[cfg(test)]
    fn publish(&self, seq: u64, slot: Slot) -> bool {
        self.reserve() && self.publish_reserved(seq, slot)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    fn slot(message: &'static str) -> Slot {
        Err(sys::Error::fault(message))
    }

    #[test]
    fn wakes_one_blocked_producer_when_space_opens() {
        let ring = Arc::new(Ring::new(1));
        assert!(ring.publish(0, slot("zero")));
        thread::scope(|scope| {
            let producer_ring = ring.clone();
            let producer = scope.spawn(move || producer_ring.publish(1, slot("one")));
            assert!(ring.take(0).is_some());
            assert!(producer.join().unwrap());
        });
        assert!(ring.take(1).is_some());
    }

    #[test]
    fn reserves_space_for_the_earliest_slow_decode() {
        let ring = Arc::new(Ring::new(2));
        assert!(ring.reserve(), "sequence zero starts decoding first");
        assert!(ring.publish(1, slot("one")));
        thread::scope(|scope| {
            let producer_ring = ring.clone();
            let producer = scope.spawn(move || producer_ring.publish(2, slot("two")));

            assert!(ring.publish_reserved(0, slot("zero")));
            assert!(ring.take(0).is_some());
            assert!(ring.take(1).is_some());
            assert!(producer.join().unwrap());
        });
        assert!(ring.take(2).is_some());
    }

    #[test]
    fn stop_releases_blocked_producers() {
        let ring = Arc::new(Ring::new(1));
        assert!(ring.publish(0, slot("zero")));
        thread::scope(|scope| {
            let producer_ring = ring.clone();
            let producer = scope.spawn(move || producer_ring.publish(1, slot("one")));
            ring.stop();
            assert!(!producer.join().unwrap());
        });
    }
}
