//! Shutdown notification and in-flight work tracking.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[derive(Clone, Default)]
pub struct Waiter {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    shutdown: AtomicBool,
    state: Mutex<State>,
    cvar: Condvar,
}

#[derive(Default)]
struct State {
    pending: usize,
    next_waker_id: u64,
    wakers: HashMap<u64, Waker>,
}

impl Waiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trigger(&self) {
        if self.inner.shutdown.swap(true, Ordering::Release) {
            return;
        }
        let wakers = {
            let mut state = self.inner.state.lock().unwrap();
            self.inner.cvar.notify_all();
            std::mem::take(&mut state.wakers)
        };
        for (_, w) in wakers {
            w.wake();
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.shutdown.load(Ordering::Acquire)
    }

    pub fn sleep_or_quit(&self, d: Duration) -> bool {
        if self.is_shutdown() {
            return true;
        }
        let state = self.inner.state.lock().unwrap();
        let (state, _) = self
            .inner
            .cvar
            .wait_timeout_while(state, d, |_| !self.is_shutdown())
            .unwrap();
        drop(state);
        self.is_shutdown()
    }

    pub fn wait_complete(&self) {
        let mut state = self.inner.state.lock().unwrap();
        while !(self.is_shutdown() && state.pending == 0) {
            state = self.inner.cvar.wait(state).unwrap();
        }
    }

    /// Register work only while shutdown has not started. The flag is checked under
    /// the same mutex `wait_complete` uses, so a registered hold is always observed.
    pub fn try_hold(&self) -> Option<HoldGuard> {
        let mut state = self.inner.state.lock().unwrap();
        if self.is_shutdown() {
            return None;
        }
        state.pending += 1;
        Some(HoldGuard {
            inner: self.inner.clone(),
        })
    }

    pub async fn cancelled(&self) {
        let id = {
            let mut state = self.inner.state.lock().unwrap();
            let id = state.next_waker_id;
            state.next_waker_id = state.next_waker_id.wrapping_add(1);
            id
        };
        CancelFuture {
            inner: self.inner.clone(),
            id,
        }
        .await
    }
}

pub struct HoldGuard {
    inner: Arc<Inner>,
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        let became_zero = {
            let mut state = self.inner.state.lock().unwrap();
            debug_assert!(state.pending > 0);
            state.pending -= 1;
            state.pending == 0
        };
        if became_zero {
            self.inner.cvar.notify_all();
        }
    }
}

struct CancelFuture {
    inner: Arc<Inner>,
    id: u64,
}

impl Future for CancelFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.inner.shutdown.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        let mut state = self.inner.state.lock().unwrap();
        if self.inner.shutdown.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        let waker = cx.waker();
        match state.wakers.get_mut(&self.id) {
            Some(slot) if !slot.will_wake(waker) => *slot = waker.clone(),
            Some(_) => {}
            None => {
                state.wakers.insert(self.id, waker.clone());
            }
        }
        drop(state);
        Poll::Pending
    }
}

impl Drop for CancelFuture {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.wakers.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    use super::*;

    struct CountWake(AtomicUsize);

    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn count_waker() -> (Arc<CountWake>, Waker) {
        let count = Arc::new(CountWake(AtomicUsize::new(0)));
        (count.clone(), Waker::from(count))
    }

    fn poll_once<F: Future<Output = ()>>(future: Pin<&mut F>, waker: &Waker) -> Poll<()> {
        Future::poll(future, &mut Context::from_waker(waker))
    }

    #[test]
    fn cancelled_futures_with_the_same_waker_are_independent() {
        let waiter = Waiter::new();
        let (count, waker) = count_waker();
        let mut dropped = Box::pin(waiter.cancelled());
        let mut live = Box::pin(waiter.cancelled());
        assert_eq!(poll_once(dropped.as_mut(), &waker), Poll::Pending);
        assert_eq!(poll_once(live.as_mut(), &waker), Poll::Pending);

        drop(dropped);
        waiter.trigger();

        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        assert_eq!(poll_once(live.as_mut(), &waker), Poll::Ready(()));
    }

    #[test]
    fn cancelled_future_updates_its_waker_when_repolled() {
        let waiter = Waiter::new();
        let (old_count, old_waker) = count_waker();
        let (new_count, new_waker) = count_waker();
        let mut future = Box::pin(waiter.cancelled());
        assert_eq!(poll_once(future.as_mut(), &old_waker), Poll::Pending);
        assert_eq!(poll_once(future.as_mut(), &new_waker), Poll::Pending);

        waiter.trigger();

        assert_eq!(old_count.0.load(Ordering::Relaxed), 0);
        assert_eq!(new_count.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn wait_complete_waits_for_registered_work() {
        let waiter = Waiter::new();
        let hold = waiter.try_hold().unwrap();
        let waiting = waiter.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            waiting.wait_complete();
            done_tx.send(()).unwrap();
        });

        waiter.trigger();
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        assert!(waiter.try_hold().is_none());
        drop(hold);
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn sleep_ignores_hold_completion_notifications() {
        let waiter = Waiter::new();
        let hold = waiter.try_hold().unwrap();
        let sleeping = waiter.clone();
        let started = Instant::now();
        let thread = std::thread::spawn(move || {
            assert!(!sleeping.sleep_or_quit(Duration::from_millis(80)));
        });

        std::thread::sleep(Duration::from_millis(10));
        drop(hold);
        thread.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(60));
    }
}
