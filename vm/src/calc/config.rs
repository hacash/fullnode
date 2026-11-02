//! Build parameters for the calc micro-VM.
//!
//! [`CALC_CONFIG_V1`] is the consensus default. Every guest-visible
//! execution capability — the four ISA profile cells, hash builtins,
//! read-only rodata, the stack guard, a free entry point — is
//! **unconditional**: v1 admits all of them and no later configuration may
//! turn one off, because rejecting or reinterpreting a previously legal
//! container is a consensus break. A config therefore carries only the
//! admission memory cap, derived from `SpaceCap` (the single knob, and it
//! only rises) via [`CalcConfig::for_space_cap`], plus two host-only build
//! knobs applied through [`CalcHostConfig`]. A sidechain that needs more
//! admission room raises `SpaceCap::calc_mem_kib_max`; it does not ship a
//! new config constant. `CalcConfig::testkit` exists only for tests and
//! the `calc-testkit` feature.

use crate::calc::container::CALC_MEM_KIB_ENGINE_MAX;
use crate::rt::{SpaceCap, CALC_MEM_KIB_MAX};

/// Build parameters for a calc container: the admission memory cap plus two
/// host-only knobs.
///
/// Fields are crate-private. Production builds obtain [`CALC_CONFIG_V1`] /
/// [`CalcConfig::v1`] (or [`CalcConfig::for_space_cap`]) and read them
/// through the getters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcConfig {
    /// Largest `mem_kib` verify will accept. Wire decode does not apply this
    /// (it applies [`CALC_MEM_KIB_ENGINE_MAX`]). Chain deploy and load both
    /// pass `SpaceCap::calc_mem_kib_max` here through
    /// [`CalcConfig::for_space_cap`].
    pub(crate) mem_kib_max: u32,
    /// Hash the container while building `CalcFnObj`. When false, `image_id`
    /// and `receipt` hash it on demand.
    pub(crate) compute_image_id_on_load: bool,
    /// Mount the AOT engine from `build_with_config` (`calc-aot` only).
    pub(crate) aot_mount: bool,
}

/// Consensus default: admission memory cap at the [`CALC_MEM_KIB_MAX`]
/// default and both host knobs off. Image-id hashing and AOT mount are host
/// choices ([`CalcHostConfig`]), not consensus switches.
pub const CALC_CONFIG_V1: CalcConfig = CalcConfig {
    mem_kib_max: CALC_MEM_KIB_MAX,
    compute_image_id_on_load: false,
    aot_mount: false,
};

/// Host knobs applied on top of [`CALC_CONFIG_V1`] when a container is
/// built. `aot_mount` selects the early lockstep check, not a different
/// guest semantics: on means "run AOT beside the interpreter and reject a
/// disagreement".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcHostConfig {
    pub aot_mount: bool,
    pub compute_image_id_on_load: bool,
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
            compute_image_id_on_load: false,
        }
    }

    pub const fn with_aot_mount(self, on: bool) -> Self {
        Self { aot_mount: on, ..self }
    }

    pub const fn with_compute_image_id_on_load(self, on: bool) -> Self {
        Self { compute_image_id_on_load: on, ..self }
    }
}

/// Absolute ceiling for [`CalcConfig::testkit`]: 16384 KiB = 16 MiB.
/// The harness walks this range; sidechain admission uses
/// [`CalcConfig::for_space_cap`], which reaches [`CALC_MEM_KIB_ENGINE_MAX`].
#[cfg(any(test, feature = "calc-testkit"))]
pub const CALC_TESTKIT_MEM_KIB_CEILING: u32 = 16384;

impl CalcConfig {
    /// Consensus default. Same value as [`CALC_CONFIG_V1`].
    pub const fn v1() -> Self {
        CALC_CONFIG_V1
    }

    /// Consensus config for a chain's current caps: v1 with the admission
    /// memory cap taken from the `SpaceCap` knob, clamped to the engine
    /// ceiling. Deploy and load both build their config here, so a raised
    /// cap applies to newly deployed containers and to reloading stored
    /// ones alike.
    pub const fn for_space_cap(cap: &SpaceCap) -> Self {
        CALC_CONFIG_V1.with_mem_kib_max(cap.calc_mem_kib_max)
    }

    /// Overlay host-only knobs. Does not change guest-visible behavior.
    pub const fn apply_host(self, host: CalcHostConfig) -> Self {
        Self {
            aot_mount: host.aot_mount,
            compute_image_id_on_load: host.compute_image_id_on_load,
            ..self
        }
    }

    pub const fn mem_kib_max(self) -> u32 {
        self.mem_kib_max
    }

    pub const fn compute_image_id_on_load(self) -> bool {
        self.compute_image_id_on_load
    }

    pub const fn aot_mount(self) -> bool {
        self.aot_mount
    }

    /// Copy with a caller-chosen memory cap, clamped to
    /// [`CALC_MEM_KIB_ENGINE_MAX`]. Sidechain packers pass the same number
    /// they put in `SpaceCap::calc_mem_kib_max` (or call
    /// [`CalcConfig::for_space_cap`] directly).
    pub const fn with_mem_kib_max(self, mem_kib_max: u32) -> Self {
        let mem_kib_max = if mem_kib_max > CALC_MEM_KIB_ENGINE_MAX {
            CALC_MEM_KIB_ENGINE_MAX
        } else {
            mem_kib_max
        };
        Self { mem_kib_max, ..self }
    }
}

#[cfg(any(test, feature = "calc-testkit"))]
impl CalcConfig {
    /// Test/conformance config: both host knobs on (eager image id, AOT
    /// mount) with a caller-chosen memory cap. `mem_kib_max` above 16384
    /// (16 MiB) is rejected. Not compiled into production builds; production
    /// sidechains use [`CalcConfig::for_space_cap`] /
    /// [`CalcConfig::with_mem_kib_max`], which clamp at the engine ceiling.
    pub fn testkit(mem_kib_max: u32) -> Result<Self, &'static str> {
        if mem_kib_max > CALC_TESTKIT_MEM_KIB_CEILING {
            return Err("calc testkit: mem_kib_max exceeds 16MiB ceiling");
        }
        Ok(Self {
            mem_kib_max,
            compute_image_id_on_load: true,
            aot_mount: true,
        })
    }
}
