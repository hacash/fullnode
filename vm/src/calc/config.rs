//! Host-only build knobs for the calc micro-VM.
//!
//! There is deliberately **no consensus configuration struct** here. Every
//! guest-visible execution capability — the ISA profile cells, HALT, the
//! memory layout and read-only regions — is unconditional
//! and frozen for v1: a later config must never be able to reject or
//! reinterpret a previously legal container, so no such switch exists.
//!
//! The only admission parameter is the chain's own
//! [`SpaceCap::calc_mem_kib_max`](crate::rt::SpaceCap::calc_mem_kib_max),
//! passed directly to the verifier and to the packers. Chain caps only rise,
//! so a kernel that passed admission still loads after an upgrade.
//!
//! [`CalcHostConfig`] carries host choices that do **not** change guest
//! semantics (currently only whether the x86-64 image is mounted for the
//! early lockstep check).

/// Host knobs applied when a container is built. These are not consensus
/// switches: `aot_mount` selects whether the early lockstep check runs, not a
/// different guest semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcHostConfig {
    /// Mount the x86-64 image beside the interpreter (`calc-aot` only).
    /// Official builds default this on; the two engines must agree or the
    /// call is rejected as [`CalcAbort2::EngineMismatch`](crate::calc::CalcAbort2::EngineMismatch).
    pub aot_mount: bool,
}

impl Default for CalcHostConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl CalcHostConfig {
    /// AOT mounts when this binary was built with `calc-aot` (the official
    /// x86-64 default). Builds that turn the feature off stay on the
    /// interpreter. `with_aot_mount(false)` is the explicit off switch.
    pub const fn new() -> Self {
        Self {
            aot_mount: cfg!(feature = "calc-aot"),
        }
    }

    pub const fn with_aot_mount(self, on: bool) -> Self {
        Self { aot_mount: on }
    }
}
