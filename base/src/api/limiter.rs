//! Concurrency and wall-clock limits for VM sandbox calls (§13.2).
//!
//! The VM sandbox runs untrusted contract bytecodes against an optimistic
//! snapshot.  Per §13.2 a single or a stream of sandbox requests must NOT be
//! able to:
//!   - starve root writers by pinning worker threads (gas-burning loops);
//!   - exhaust the server's thread pool via unbounded concurrent calls;
//!   - monopolise the API from one peer.
//!
//! This module exposes a `SandboxLimiter` providing:
//!   * a global concurrent-call cap (returns `Err` when exceeded);
//!   * a per-source-IP concurrent-call cap;
//!   * a wall-clock deadline guard that aborts a sandbox call even if the
//!     contract enters a low-gas-burning but non-terminating loop.
//!
//! All checks are non-blocking - the caller is responsible for returning a
//! `429 / sandbox busy` response when a permit cannot be acquired.  The server
//! never queues sandbox requests: §13.2 forbids unbounded queues behind the
//! gate.
//!
//! The limiter is shared via `ApiExecCtx` so that every route handler sees the
//! same counters regardless of which HTTP worker served the request.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default caps.  Tunable through `SandboxLimiter::with_*` if a deployment
/// needs different limits.
pub const DEFAULT_GLOBAL_CONCURRENCY: usize = 8;
pub const DEFAULT_PER_IP_CONCURRENCY: usize = 2;
/// Default wall-clock deadline for a single sandbox call.  Generous enough for
/// legitimate contract queries, well below any sensible request timeout so a
/// stuck call cannot pin a worker indefinitely.
pub const DEFAULT_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// RAII permit returned by `SandboxLimiter::acquire`.  Drops release the
/// global + per-IP counters even if the caller forgets to call
/// `check_deadline` (the deadline guard is best-effort - it does NOT forcibly
/// kill the running sandbox thread, since the VM has no native cancellation
/// hook; instead it makes the API handler return `state_changed` /
/// `sandbox_timeout` to the peer, and the limiter slot stays held until the
/// sandbox_call function actually returns).
pub struct SandboxPermit {
    limiter: Arc<Inner>,
    ip: Option<String>,
    /// Wall-clock deadline polled by the caller and injected into the VM for
    /// cooperative checks at instruction boundaries.
    deadline: Instant,
    /// Instant the permit was acquired, for `elapsed()`.
    acquired_at: Instant,
}

impl SandboxPermit {
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Returns `Ok(())` while the call is within its wall-clock budget and an
    /// error once the deadline elapsed.
    pub fn check_deadline(&self) -> Result<(), &'static str> {
        if Instant::now() > self.deadline {
            return Err("sandbox_wall_clock_timeout");
        }
        Ok(())
    }

    /// Wall-clock elapsed since the permit was acquired.  Used by callers that
    /// want to log slow sandbox calls.
    pub fn elapsed(&self) -> Duration {
        self.acquired_at.elapsed()
    }
}

impl Drop for SandboxPermit {
    fn drop(&mut self) {
        let mut g = self.limiter.state.lock().expect("sandbox limiter poisoned");
        g.global_inflight = g.global_inflight.saturating_sub(1);
        if let Some(ip) = &self.ip
            && let Some(count) = g.per_ip_inflight.get_mut(ip)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                g.per_ip_inflight.remove(ip);
            }
        }
    }
}

#[derive(Default)]
struct LimiterState {
    global_inflight: usize,
    per_ip_inflight: HashMap<String, usize>,
}

struct Inner {
    state: Mutex<LimiterState>,
    global_cap: usize,
    per_ip_cap: usize,
    wall_clock_timeout: Duration,
}

/// Shared concurrency + wall-clock limiter for sandbox calls.
#[derive(Clone)]
pub struct SandboxLimiter {
    inner: Arc<Inner>,
}

impl Default for SandboxLimiter {
    fn default() -> Self {
        Self::new(
            DEFAULT_GLOBAL_CONCURRENCY,
            DEFAULT_PER_IP_CONCURRENCY,
            DEFAULT_WALL_CLOCK_TIMEOUT,
        )
    }
}

impl SandboxLimiter {
    pub fn new(global_cap: usize, per_ip_cap: usize, wall_clock_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(LimiterState::default()),
                global_cap,
                per_ip_cap,
                wall_clock_timeout,
            }),
        }
    }

    /// Try to acquire a permit for a sandbox call originating from `peer_ip`
    /// (or `None` if the caller has no peer - e.g. localhost).  Non-blocking:
    /// returns `Err` immediately if either the global or per-IP cap is
    /// exceeded, so the caller can return a `sandbox_busy` response without
    /// queueing the request (§13.2 forbids unbounded queues).
    ///
    /// Returns a `SandboxPermit` whose `Drop` releases the counters, and
    /// whose `check_deadline` enforces the wall-clock budget.
    pub fn acquire(&self, peer_ip: Option<String>) -> Result<SandboxPermit, &'static str> {
        let mut g = self.inner.state.lock().expect("sandbox limiter poisoned");
        if g.global_inflight >= self.inner.global_cap {
            return Err("sandbox_global_concurrency_exceeded");
        }
        if let Some(ip) = peer_ip.as_ref() {
            let count = g.per_ip_inflight.get(ip).copied().unwrap_or(0);
            if count >= self.inner.per_ip_cap {
                return Err("sandbox_per_ip_concurrency_exceeded");
            }
        }
        // Commit the permit.
        g.global_inflight = g.global_inflight.saturating_add(1);
        if let Some(ip) = peer_ip.as_ref() {
            *g.per_ip_inflight.entry(ip.clone()).or_insert(0) += 1;
        }
        let acquired_at = Instant::now();
        let deadline = acquired_at + self.inner.wall_clock_timeout;
        Ok(SandboxPermit {
            limiter: self.inner.clone(),
            ip: peer_ip,
            deadline,
            acquired_at,
        })
    }
}
