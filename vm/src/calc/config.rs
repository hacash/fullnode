//! Consensus feature gates for the calc micro-VM.
//!
//! [`CALC_CONFIG_V1`] is the consensus default: hash builtins, read-only
//! rodata, the stack guard, a free entry point, and both ISA axes
//! (bitmanip and RVC) are on. The container profile byte is the deployer's
//! choice among the four combinations. A later consensus upgrade must ship
//! a new named constant (for example `CALC_CONFIG_V2`), not
//! [`CalcConfig::all_enabled`].
//! `all_enabled`, the `with_*` builders and [`CalcConfig::testkit`] exist
//! only for tests and the `calc-testkit` feature. Changing a v1 execution
//! flag is a consensus upgrade and must not make a previously legal
//! execution fail. Space caps live on `SpaceCap` and only ever rise.
//! Deploy and load both use [`CALC_CONFIG_V1`] plus the current `SpaceCap`.
//! Host-only knobs live on [`CalcHostConfig`].

use crate::calc::container::{CALC_MEM_KIB_ENGINE_MAX, CALC_MEM_KIB_MAX};

/// Which optional calc capabilities are active.
///
/// Profile 0 (neither axis) is always allowed. `profile1_bitmanip` admits
/// bit 0 of the profile byte. `compressed` admits bit 1 (RVC). A container
/// whose declared bits are not admitted is rejected at verify.
///
/// Fields are crate-private. Production builds can only obtain
/// [`CALC_CONFIG_V1`] / [`CalcConfig::v1`] and read it through the getters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcConfig {
    /// Allow profile bit 0 (Zba/Zbb/Zbs/Zbc). On in v1.
    pub(crate) profile1_bitmanip: bool,
    /// Allow profile bit 1 (RVC). On in v1. zk images leave this bit clear
    /// in the container; they do not need the chain to forbid it.
    pub(crate) compressed: bool,
    /// Allow hash ecalls (`a7` = 1..=6). HALT (`a7` = 0) is always available.
    pub(crate) hash_builtins: bool,
    /// Trap stores and digest writes into rodata and `[rodata_end, data_base)`.
    /// Code and the pad below `rodata_base` stay non-writable either way.
    pub(crate) rodata_readonly: bool,
    /// Trap stores that intersect the 256-byte stack guard band.
    pub(crate) stack_guard: bool,
    /// Hash the container while building `CalcFnObj`. When false, `image_id`
    /// and `receipt` hash it on demand.
    pub(crate) compute_image_id_on_load: bool,
    /// Mount the AOT engine from `build_with_config` (`calc-aot` only).
    pub(crate) aot_mount: bool,
    /// Allow any instruction-boundary entry inside the code region.
    /// When false, verify requires `entry == CODE_BASE`.
    pub(crate) free_entry: bool,
    /// Largest `mem_kib` verify will accept. Wire decode does not apply this
    /// (it applies [`CALC_MEM_KIB_ENGINE_MAX`]). Chain deploy admission is
    /// `SpaceCap::calc_mem_kib_max`, which defaults to the same v1 value.
    pub(crate) mem_kib_max: u32,
}

/// Consensus default. Both ISA axes are admitted; the container profile
/// byte selects the combination. Hash builtins, read-only rodata, the
/// stack guard and a free entry point are on. Image-id hashing and AOT
/// mount are host choices ([`CalcHostConfig`]), not consensus switches.
pub const CALC_CONFIG_V1: CalcConfig = CalcConfig {
    profile1_bitmanip: true,
    compressed: true,
    hash_builtins: true,
    rodata_readonly: true,
    stack_guard: true,
    compute_image_id_on_load: false,
    aot_mount: false,
    free_entry: true,
    mem_kib_max: CALC_MEM_KIB_MAX,
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
/// [`CalcConfig::with_mem_kib_max`], which reaches [`CALC_MEM_KIB_ENGINE_MAX`].
#[cfg(any(test, feature = "calc-testkit"))]
pub const CALC_TESTKIT_MEM_KIB_CEILING: u32 = 16384;

impl CalcConfig {
    /// Consensus default. Same value as [`CALC_CONFIG_V1`].
    pub const fn v1() -> Self {
        CALC_CONFIG_V1
    }

    /// Overlay host-only knobs. Does not change guest-visible flags.
    pub const fn apply_host(self, host: CalcHostConfig) -> Self {
        Self {
            aot_mount: host.aot_mount,
            compute_image_id_on_load: host.compute_image_id_on_load,
            ..self
        }
    }

    pub const fn profile1_bitmanip(self) -> bool {
        self.profile1_bitmanip
    }
    pub const fn compressed(self) -> bool {
        self.compressed
    }
    pub const fn hash_builtins(self) -> bool {
        self.hash_builtins
    }
    pub const fn rodata_readonly(self) -> bool {
        self.rodata_readonly
    }
    pub const fn stack_guard(self) -> bool {
        self.stack_guard
    }
    pub const fn compute_image_id_on_load(self) -> bool {
        self.compute_image_id_on_load
    }
    pub const fn aot_mount(self) -> bool {
        self.aot_mount
    }
    pub const fn free_entry(self) -> bool {
        self.free_entry
    }
    pub const fn mem_kib_max(self) -> u32 {
        self.mem_kib_max
    }

    /// Copy with a caller-chosen memory cap, clamped to
    /// [`CALC_MEM_KIB_ENGINE_MAX`]. Sidechain packers pass the same number
    /// they put in `SpaceCap::calc_mem_kib_max`.
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
    /// Pre-gate behavior for tests: profile 1, hashes, rodata and guard
    /// checks, eager image id, AOT mount, and a free entry point.
    /// `mem_kib_max` stays at the v1 cap ([`CALC_MEM_KIB_MAX`]). Not a
    /// consensus config and not a large-memory preset.
    pub const fn all_enabled() -> Self {
        Self {
            profile1_bitmanip: true,
            compressed: true,
            hash_builtins: true,
            rodata_readonly: true,
            stack_guard: true,
            compute_image_id_on_load: true,
            aot_mount: true,
            free_entry: true,
            mem_kib_max: CALC_MEM_KIB_MAX,
        }
    }

    pub const fn with_profile1_bitmanip(self, on: bool) -> Self {
        Self { profile1_bitmanip: on, ..self }
    }

    pub const fn with_compressed(self, on: bool) -> Self {
        Self { compressed: on, ..self }
    }

    pub const fn with_hash_builtins(self, on: bool) -> Self {
        Self { hash_builtins: on, ..self }
    }

    pub const fn with_rodata_readonly(self, on: bool) -> Self {
        Self { rodata_readonly: on, ..self }
    }

    pub const fn with_stack_guard(self, on: bool) -> Self {
        Self { stack_guard: on, ..self }
    }

    pub const fn with_compute_image_id_on_load(self, on: bool) -> Self {
        Self { compute_image_id_on_load: on, ..self }
    }

    pub const fn with_aot_mount(self, on: bool) -> Self {
        Self { aot_mount: on, ..self }
    }

    pub const fn with_free_entry(self, on: bool) -> Self {
        Self { free_entry: on, ..self }
    }

    /// Test/conformance config: [`all_enabled`] capabilities with a caller
    /// chosen memory cap. `mem_kib_max` above 16384 (16 MiB) is rejected.
    /// Not compiled into production builds. Production sidechains use
    /// [`CalcConfig::with_mem_kib_max`], which clamps at the engine ceiling.
    pub fn testkit(mem_kib_max: u32) -> Result<Self, &'static str> {
        if mem_kib_max > CALC_TESTKIT_MEM_KIB_CEILING {
            return Err("calc testkit: mem_kib_max exceeds 16MiB ceiling");
        }
        Ok(Self { mem_kib_max, ..Self::all_enabled() })
    }
}
