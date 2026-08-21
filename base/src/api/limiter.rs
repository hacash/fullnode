//! Concurrency and wall-clock limits for VM sandbox calls (§13.2): a global + per-IP
//! concurrent-call cap and a wall-clock deadline guard, so untrusted bytecode cannot starve writers or exhaust the worker pool.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default caps.  Tunable through `SandboxLimiter::with_*` if a deployment
/// needs different limits.
pub const DEFAULT_GLOBAL_CONCURRENCY: usize = 8;
pub const DEFAULT_PER_IP_CONCURRENCY: usize = 2;
/// Default wall-clock deadline for a single sandbox call: generous for legitimate
/// queries, far below any request timeout so a stuck call cannot pin a worker.
pub const DEFAULT_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// RAII permit returned by `SandboxLimiter::acquire`; `Drop` releases the global
/// + per-IP counters. `check_deadline` is best-effort — it cannot kill the VM thread.
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

    /// Non-blocking acquire from `peer_ip` (or `None`); `Err` immediately if either
    /// cap is exceeded, so the caller answers `sandbox_busy` without queueing (§13.2).
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
