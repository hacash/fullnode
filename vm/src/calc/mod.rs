//! HVM calc func: RV64IM(+C)(+B)(+Zicclsm) pure-compute kernels.
//!
//! v2 admits every B / RVC profile combination (profile bytes 0..=3);
//! profile 0 is the zk-standardized subset (eth-act `riscv64im_zicclsm`).
//! The HASH bit stays unadmitted. See `SPEC.md`.

pub mod config;
pub mod container;
pub mod decode;
pub mod interp;
pub mod verify;

pub use config::CalcHostConfig;
/// Re-exported so packers and tests can name the chain caps through the
/// public `calc` module (`vm::rt` is crate-private).
pub use crate::rt::SpaceCap;
// x86-64 AOT. Official builds turn `calc-aot` on and `CalcFnObj::run` executes
// it in lockstep with the interpreter. See `aot.rs`.
#[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
pub mod aot;

#[cfg(test)]
mod isa_tests;

use crate::rt::{ItrErr, ItrErrCode, VmrtRes};

pub use container::{
    profile_admitted, profile_has_bitmanip, profile_has_compressed, profile_has_hash, profile_known,
    CALC_PROFILE_BITMANIP, CALC_PROFILE_COMPRESSED, CALC_PROFILE_HASH,
    CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IM_B, CALC_PROFILE_RV64IMC, CALC_PROFILE_RV64IMC_B,
    CODE_BASE as CALC_CODE_BASE, HALT_STUB as CALC_HALT_STUB,
    IMAGE_ID_DOMAIN as CALC_IMAGE_ID_DOMAIN,
};

/// Exchange rate: 1 compute = 48 cycles. CONSENSUS CONSTANT: changing this
/// value changes gas results and is a hard fork — do not touch after
/// activation.
///
/// `48` is a chosen policy value, not a unit derivation.
///
/// Calibration (re-measured 2026-11-04 on an i7-8550U, release+LTO,
/// pinned, warm): HVM arithmetic ~33 ns/compute; calc interpreter
/// ~4.1 ns/cycle; x86-64 AOT ~0.38 ns/cycle; lockstep ~4.5 ns/cycle.
/// AOT parity is N ≈ 87, so N=48 prices calc at roughly +81% over AOT
/// hardware cost. It is still roughly 6x cheaper than the
/// interpreter/lockstep. The 200-diamond HIP5 kernel (200,863 cycles)
/// costs 4,185 compute at N=48. Decide the pricing engine and target
/// hardware before activation, then freeze N.
pub const CYCLES_PER_COMPUTE: u64 = 48;

/// Resource charged per declared KiB of calc memory (input bytes, output
/// bytes and declared memory are the only resource charges on the calc path).
/// 2 per KiB: a 64 KiB kernel costs 128 resource per call.
pub const CALC_MEM_RESOURCE_PER_KIB: i64 = 2;

/// Per-call fixed cycle charge, deducted before the first instruction runs.
/// Every invocation costs the host a layout computation, a fresh memory
/// allocation and a full zeroing of the declared memory; without this charge
/// an instant-return kernel could burn that host work for free (resource gas
/// alone does not cover it).
pub const CALC_SETUP_BASE_CYCLES: u64 = 64;

/// Additional setup cycles per declared KiB of calc memory (zeroing cost).
pub const CALC_SETUP_PER_KIB_CYCLES: u64 = 16;

/// Minimum stack that must remain above the end of the guest image. The
/// stack itself is the rest of the gap up to the output region, not a fixed
/// 2 KiB window. The stack region starts exactly at `guest_end`: there is
/// deliberately **no guard band** — a frame larger than the free span
/// overwrites guest data deterministically instead of trapping, which keeps
/// the permission map pure PT_LOAD semantics and zk-circuit minimal.
/// CONSENSUS CONSTANT: this is the single fit rule
/// [`CALC_MIN_FREE_RW_GAP`]; it is a floor that may only be **lowered**
/// after activation — raising it would reject containers that were legal
/// when deployed.
pub const CALC_STACK_SIZE: u64 = 2 * 1024;

/// Minimum free RW span the layout must leave between the end of the guest
/// image and `stack_top`: the minimum stack. The single fit rule used by
/// wire decode, the verifier and the packers.
pub const CALC_MIN_FREE_RW_GAP: u64 = CALC_STACK_SIZE;

/// ecall a7 = HALT. a0 = exit code (0 = success), a1 = output length.
pub const ECALL_HALT: u64 = 0;
/// Reserved HINT / private-input a7 range. L1 nodes cannot agree on hint
/// content, so these numbers trap until a later activation.
pub const ECALL_HINT_MIN: u64 = 16;
pub const ECALL_HINT_MAX: u64 = 31;

/// Hash builtin numbers behind `ecall` (a7), aligned with the native hash
/// catalog (`rt/native_catalog.rs`). HALT is 0; 1..=6 are hashes. v2 traps
/// them: [`CALC_PROFILE_HASH`] is not admitted (hash admission is its own
/// hard fork with its own pricing activation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashKind {
    Sha2 = 1,
    Sha3 = 2,
    Ripemd160 = 3,
    Keccak256 = 4,
    Blake2s256 = 5,
    Blake2b256 = 6,
}

impl HashKind {
    pub fn from_a7(v: u64) -> Option<Self> {
        Some(match v {
            1 => HashKind::Sha2,
            2 => HashKind::Sha3,
            3 => HashKind::Ripemd160,
            4 => HashKind::Keccak256,
            5 => HashKind::Blake2s256,
            6 => HashKind::Blake2b256,
            _ => return None,
        })
    }
    /// Digest length in bytes.
    pub fn digest_len(self) -> u64 {
        match self {
            HashKind::Ripemd160 => 20,
            _ => 32,
        }
    }
    /// Cycles: fixed overhead + per-64-byte block cost.
    ///
    /// Calibrated against portable (non-accelerated) RustCrypto throughput on
    /// consensus-class hardware, so a node without SHA-NI never burns native
    /// cycles far beyond what was charged: roughly 1.5-2 cycles/byte for the
    /// SHA family and RIPEMD-160 (~10-15 native cycles/byte portable) and
    /// ~1 cycle/byte for BLAKE2 (~5-8 native cycles/byte portable). RIPEMD-160
    /// is priced at/above SHA-256, matching its real relative cost. Builtins
    /// stay an order of magnitude cheaper than computing a hash in-kernel
    /// with RV64IM instructions (~700-1300 cycles per SHA-256 block), so the
    /// incentive to use them is preserved.
    pub fn cycles(self, len: u64) -> u64 {
        // Saturating: `u64::MAX` bytes must not panic (runtime lengths are
        // bounded by the 64 KiB memory, but the pricing helper stays total).
        let blocks = len.div_ceil(64);
        let per_block = match self {
            HashKind::Sha2 => 96,
            HashKind::Sha3 => 128,
            HashKind::Ripemd160 => 128,
            HashKind::Keccak256 => 128,
            HashKind::Blake2s256 => 64,
            HashKind::Blake2b256 => 64,
        };
        64u64.saturating_add(blocks.saturating_mul(per_block))
    }
}

/// Run a hash builtin over `input`, appending nothing else: digest written
/// into `out` (must be exactly `digest_len` bytes).
pub fn run_hash(kind: HashKind, input: &[u8], out: &mut [u8]) {
    use blake2::digest::consts::U32;
    use sha3::Digest as _;
    use tiny_keccak::Hasher as _;
    let dlen = kind.digest_len() as usize;
    debug_assert_eq!(out.len(), dlen);
    match kind {
        HashKind::Sha2 => out.copy_from_slice(&sha2::Sha256::digest(input)),
        HashKind::Sha3 => out.copy_from_slice(&sha3::Sha3_256::digest(input)),
        HashKind::Ripemd160 => out.copy_from_slice(&ripemd::Ripemd160::digest(input)),
        HashKind::Keccak256 => {
            let mut k = tiny_keccak::Keccak::v256();
            k.update(input);
            let mut buf = [0u8; 32];
            k.finalize(&mut buf);
            out.copy_from_slice(&buf);
        }
        HashKind::Blake2s256 => out.copy_from_slice(&blake2::Blake2s256::digest(input)),
        HashKind::Blake2b256 => out.copy_from_slice(&blake2::Blake2b::<U32>::digest(input)),
    }
}

/// Canonical call receipt: the future proof statement. Deterministic and
/// identical between interpreter and AOT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcReceipt {
    pub image_id: [u8; 32],
    pub input_hash: [u8; 32],
    /// SHA-256 of the output bytes; SHA-256 of empty on any abort.
    pub output_hash: [u8; 32],
    /// 0 on success or architectural abort; guest-reported nonzero halt
    /// code otherwise.
    pub exit_code: u64,
    /// `None` on a successful halt (exit_code = 0). Guest nonzero halt is
    /// classified as [`CalcAbort2::Trap`].
    pub abort: Option<CalcAbort2>,
    pub cycles_used: u64,
}

/// Result of one calc invocation through the HVM call layer, carrying the
/// gas actually consumed so the caller can charge it on both the success
/// and the abort path (failures charge up to the stop point).
///
/// The SHA-256 proof statement is **not** computed here: hashing input and
/// output on every consensus call would be uncharged host work. Callers
/// that need a receipt use [`CalcFnObj::receipt`].
#[derive(Debug, Clone)]
pub struct CalcExec {
    /// Output bytes on success; abort classification otherwise.
    pub result: Result<Vec<u8>, CalcAbort2>,
    /// Cycles consumed up to the stop point (exact on success).
    pub cycles_used: u64,
    /// Resource charge: ceil(in/16) + ceil(out/16) + 2 per declared KiB.
    pub resource: i64,
    /// 0 on success or architectural abort; guest HALT nonzero `a0`
    /// otherwise (mapped to [`CalcAbort2::Trap`] in `result`).
    pub exit_code: u64,
}

/// Abort classification mirroring `interp::CalcAbort` without the message
/// payload (mapped to `ItrErr` by the caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcAbort2 {
    Trap,
    OutOfGas,
    /// The host machine's wall-clock deadline elapsed mid-run. Only the
    /// interpreter observes a deadline (see [`CalcFnObj::run`]).
    Deadline,
    /// The pre-run resource budget cannot cover the minimum charge.
    OutOfResource,
    /// Interpreter and AOT disagreed on gas or the result. Fatal for the
    /// transaction; not a guest-visible abort.
    EngineMismatch,
}

/// Per-call execution limits. `resource < 0` means the resource budget is
/// unlimited; `deadline` is only set by non-consensus builders.
#[derive(Debug, Clone, Copy)]
pub struct CalcRunLimits {
    pub cycles: u64,
    pub resource: i64,
    pub deadline: Option<std::time::Instant>,
}

/// A verified calc function attached to a contract (built once at load).
pub struct CalcFnObj {
    container: container::CalcContainer,
    module: verify::CalcModule,
    /// Lazily hashed canonical image ID. `OnceLock` keeps the build path
    /// free of uncharged hashing while avoiding a second code path.
    image_id: std::sync::OnceLock<[u8; 32]>,
    /// Mounted x86-64 image. `None` means this object runs on the interpreter
    /// only (feature off, or the host turned AOT off). The consensus call
    /// path holds an `Arc`, so the image stays mounted there.
    #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
    aot: Option<aot::AotFunc>,
}

impl std::fmt::Debug for CalcFnObj {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalcFnObj")
            .field("container", &self.container)
            .field("image_id", &self.image_id())
            .field("module", &"<verified>")
            .finish()
    }
}

impl CalcFnObj {
    /// Verify a container under the chain's current `SpaceCap` and build the
    /// executable object. This is the production deploy/load entry: the only
    /// chain input is `cap.calc_mem_kib_max`, and `host.aot_mount` selects
    /// the lockstep check.
    pub fn build(
        c: container::CalcContainer,
        cap: &SpaceCap,
        host: CalcHostConfig,
    ) -> VmrtRes<Self> {
        let module = verify::verify_container_with_cap(&c, cap)?;
        #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
        let aot = if host.aot_mount {
            Some(aot::aot_compile(&module).ok_or_else(|| {
                ItrErr::new(ItrErrCode::ContractError, "calc aot compile failed")
            })?)
        } else {
            None
        };
        #[cfg(not(all(feature = "calc-aot", target_arch = "x86_64")))]
        let _ = host;
        Ok(Self {
            container: c,
            module,
            image_id: std::sync::OnceLock::new(),
            #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
            aot,
        })
    }

    /// [`Self::build`] under the default chain caps. Tests and the testkit
    /// only: production deploy/load pass the live `SpaceCap`.
    #[cfg(any(test, feature = "calc-testkit"))]
    pub fn build_v1(c: container::CalcContainer) -> VmrtRes<Self> {
        Self::build(c, &SpaceCap::new(0), CalcHostConfig::new())
    }

    /// [`Self::build_v1`] with host-only knobs applied. Tests only.
    #[cfg(any(test, feature = "calc-testkit"))]
    pub fn build_with_host(
        c: container::CalcContainer,
        host: config::CalcHostConfig,
    ) -> VmrtRes<Self> {
        Self::build(c, &SpaceCap::new(0), host)
    }

    /// [`Self::build_v1`] with AOT mounting requested. A compile failure
    /// fails the build; the lockstep check must not silently disappear.
    /// Tests and the testkit only.
    #[cfg(all(
        feature = "calc-aot",
        target_arch = "x86_64",
        any(test, feature = "calc-testkit")
    ))]
    pub fn build_with_aot(c: container::CalcContainer) -> VmrtRes<Self> {
        Self::build_with_host(c, CalcHostConfig::new().with_aot_mount(true))
    }

    /// Test/testkit compatibility shim for builds without an x86-64 AOT
    /// engine: the object is built on the interpreter, and `aot_mounted()`
    /// stays false. Production code must use [`Self::build`] plus an explicit
    /// [`CalcHostConfig`].
    #[cfg(any(test, feature = "calc-testkit"))]
    #[cfg(not(all(feature = "calc-aot", target_arch = "x86_64")))]
    pub fn build_with_aot(c: container::CalcContainer) -> VmrtRes<Self> {
        Self::build(c, &SpaceCap::new(0), CalcHostConfig::new())
    }

    /// Declared usable memory in KiB (resource/setup charge input).
    pub(crate) fn mem_kib(&self) -> u32 {
        self.container.mem_kib
    }

    pub(crate) fn output_max(&self) -> u16 {
        self.container.output_max
    }

    /// Canonical image ID of the attached container. Hashed on first use and
    /// cached; the value is fully determined by the container bytes.
    pub fn image_id(&self) -> [u8; 32] {
        *self.image_id.get_or_init(|| self.container.image_id())
    }

    /// Whether an AOT image is mounted on this object.
    #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
    pub fn aot_mounted(&self) -> bool {
        self.aot.is_some()
    }

    /// Whether an AOT image is mounted on this object. Always false on
    /// targets without the x86-64 backend.
    #[cfg(not(all(feature = "calc-aot", target_arch = "x86_64")))]
    pub fn aot_mounted(&self) -> bool {
        false
    }

    /// Derived proof statement for a completed [`CalcExec`]. SHA-256 of
    /// input/output is charged to the caller that asks, not to consensus
    /// execution.
    pub fn receipt(&self, input: &[u8], exec: &CalcExec) -> CalcReceipt {
        let abort = match &exec.result {
            Ok(_) => None,
            Err(a) => Some(*a),
        };
        let output: &[u8] = match &exec.result {
            Ok(o) => o,
            Err(_) => &[],
        };
        CalcReceipt {
            image_id: self.image_id(),
            input_hash: sha256_bytes(input),
            output_hash: if abort.is_none() {
                sha256_bytes(output)
            } else {
                sha256_bytes(&[])
            },
            exit_code: exec.exit_code,
            abort,
            cycles_used: exec.cycles_used,
        }
    }

    /// Run with the HVM gas rules; never touches host state.
    pub fn run(&self, input: &[u8], limits: CalcRunLimits) -> CalcExec {
        let lay = self.container.layout();
        let in_res = ceil_div(input.len() as u64, 16) as i64;
        // Resource/setup charges use declared usable memory (mem_kib), not
        // the unmapped [0, CODE_BASE) page.
        let mem_res = (lay.usable_len / 1024) as i64 * CALC_MEM_RESOURCE_PER_KIB;
        // The whole resource charge is known before the run: input bytes,
        // declared memory, and `output_max` (the container's declared
        // maximum, even when a smaller output is returned). Admission caps
        // reserve the same maximum, so a call that passes this pre-flight can
        // never exceed the resource budget after the run. There is no
        // post-run resource check.
        let out_res_max = ceil_div(lay.output_max, 16) as i64;
        // Pre-flight checks charge base gas only: they run before any cycle.
        if input.len() as u64 > lay.input_max {
            return calc_exec(Err(CalcAbort2::Trap), 0, 0, 0, 0);
        }
        if limits.resource >= 0
            && in_res
                .saturating_add(mem_res)
                .saturating_add(out_res_max)
                > limits.resource
        {
            return calc_exec(Err(CalcAbort2::OutOfResource), 0, 0, 0, 0);
        }
        // The per-call setup charge (memory allocation + zeroing + image
        // copies) is deducted up front and always kept, even on traps.
        let setup =
            CALC_SETUP_BASE_CYCLES + (lay.usable_len / 1024) * CALC_SETUP_PER_KIB_CYCLES;
        if limits.cycles < setup {
            // Terminal OutOfGas: report the attempted setup charge only, not
            // the whole remaining budget. A tx reaching this abort cannot be
            // included, so nothing beyond setup is burned; if ceil(setup/48)
            // then exceeds the compute bucket, settle fails with the same
            // ItrErrCode::OutOfGas — the failure class never changes.
            return calc_exec(Err(CalcAbort2::OutOfGas), setup, 0, in_res, mem_res);
        }
        // Early lockstep insurance. Official x86-64 binaries compile
        // `calc-aot` and mount an image, so every call runs the interpreter
        // and then AOT in order on this thread and keeps the result only
        // when the engines agree on the FULL stop state: gas (`cycles_used`),
        // the consensus-observable outcome (output bytes, abort class, halt
        // code), all 32 registers, and the whole memory image. A mismatch in
        // any of these is `EngineMismatch`: the caller rejects the
        // transaction and it does not enter a block. The register/memory
        // compare is deliberate: an engine bug that happens to agree on the
        // consensus surface must still be surfaced while both engines run.
        // A panic inside either engine is reported as `EngineMismatch` the
        // same way — BUT only under unwind builds (the test profiles). The
        // workspace release profile sets `panic = "abort"`, where a panic
        // terminates the process before any handler runs (the supervisor
        // restarts from durable state); `catch_unwind` here is effective
        // exactly where tests run. Builds without the feature, or with the
        // image unmounted, take the interpreter alone.
        //
        // A host wall-clock deadline is observed by the interpreter only. The
        // AOT image **still runs** (the lockstep is never skipped, so preflight
        // and consensus exercise the same engines), but a deadline stop is
        // host policy rather than guest semantics: the AOT verdict is not
        // compared and the call reports `Deadline`. Once production has
        // stayed quiet, the interpreter side of this pair is removed and AOT
        // is the only engine.
        #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
        if let Some(aot) = self.aot.as_ref() {
            let budget = limits.cycles - setup;
            let (interp_out, interp_regs, interp_mem) = match std::panic::catch_unwind(
                std::panic::AssertUnwindSafe(|| {
                    let (r, regs, mem) = interp::run_lockstep(
                        &self.module,
                        &self.container,
                        input,
                        budget,
                        limits.deadline,
                    );
                    (engine_from_interp(setup, r), regs, mem)
                }),
            ) {
                Ok(out) => out,
                Err(_) => return calc_exec(Err(CalcAbort2::EngineMismatch), 0, 0, 0, 0),
            };
            let (aot_out, aot_regs, aot_mem) = match std::panic::catch_unwind(
                std::panic::AssertUnwindSafe(|| {
                    let (out, regs, mem) =
                        aot.run_with_state(&self.module, &self.container, input, budget);
                    (engine_from_aot(setup, out), regs, mem)
                }),
            ) {
                Ok(out) => out,
                Err(_) => return calc_exec(Err(CalcAbort2::EngineMismatch), 0, 0, 0, 0),
            };
            if matches!(interp_out.result, Err(CalcAbort2::Deadline)) {
                return calc_exec(
                    Err(CalcAbort2::Deadline),
                    interp_out.cycles_used,
                    0,
                    in_res,
                    mem_res,
                );
            }
            if interp_out != aot_out || interp_regs != aot_regs || interp_mem != aot_mem {
                return calc_exec(Err(CalcAbort2::EngineMismatch), 0, 0, 0, 0);
            }
            return calc_exec(
                interp_out.result,
                interp_out.cycles_used,
                interp_out.exit_code,
                in_res,
                mem_res,
            );
        }
        let out = engine_from_interp(
            setup,
            interp::run(
                &self.module,
                &self.container,
                input,
                limits.cycles - setup,
                limits.deadline,
            ),
        );
        calc_exec(out.result, out.cycles_used, out.exit_code, in_res, mem_res)
    }
}

/// One engine's gas and consensus-observable outcome, compared by the
/// lockstep check.
#[derive(Clone, PartialEq, Eq)]
struct EngineOut {
    result: Result<Vec<u8>, CalcAbort2>,
    cycles_used: u64,
    exit_code: u64,
}

fn engine_from_interp(
    setup: u64,
    exec: Result<interp::CalcRun, (interp::CalcAbort, u64)>,
) -> EngineOut {
    match exec {
        Ok(r) => EngineOut {
            result: Ok(r.output),
            cycles_used: setup + r.cycles_used,
            exit_code: 0,
        },
        Err((abort, used)) => {
            let (cls, exit_code) = match abort {
                interp::CalcAbort::Trap(_) => (CalcAbort2::Trap, 0),
                interp::CalcAbort::GuestExit(code) => (CalcAbort2::Trap, code),
                interp::CalcAbort::OutOfGas => (CalcAbort2::OutOfGas, 0),
                interp::CalcAbort::Deadline => (CalcAbort2::Deadline, 0),
            };
            EngineOut { result: Err(cls), cycles_used: setup + used, exit_code }
        }
    }
}

#[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
fn engine_from_aot(setup: u64, out: aot::AotOutcome) -> EngineOut {
    EngineOut {
        result: match out.abort {
            None => Ok(out.output.unwrap_or_default()),
            Some(a) => Err(a),
        },
        cycles_used: setup + out.cycles_used,
        exit_code: out.exit_code,
    }
}

/// Assemble one [`CalcExec`]: the resource charge is the input, actual output
/// and declared-memory terms (an abort has no output term).
fn calc_exec(
    result: Result<Vec<u8>, CalcAbort2>,
    cycles_used: u64,
    exit_code: u64,
    in_res: i64,
    mem_res: i64,
) -> CalcExec {
    let out_res = match &result {
        Ok(o) => ceil_div(o.len() as u64, 16) as i64,
        Err(_) => 0,
    };
    CalcExec {
        result,
        cycles_used,
        resource: in_res.saturating_add(out_res).saturating_add(mem_res),
        exit_code,
    }
}

/// ceil(a / b) for non-negative a and positive b.
#[inline(always)]
pub(crate) fn ceil_div(a: u64, b: u64) -> u64 {
    a / b + u64::from(a % b != 0)
}


/// SHA-256 of `data` (receipt / image-id helper).
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}

/// Reject an encoded container that exceeds the chain's `calc_image_max`
/// (also capped by the `u16` body prefix the contract code uses).
pub(crate) fn reject_image_len(len: usize, cap: &SpaceCap) -> VmrtRes<()> {
    let max = cap.calc_image_max.min(u16::MAX as usize);
    if len > max {
        return Err(ItrErr::new(
            ItrErrCode::ContractError,
            &format!("calcfunc size overflow, max {}", max),
        ));
    }
    Ok(())
}

/// Build a calc container from load segments, admitted against `cap`.
#[allow(clippy::too_many_arguments)]
pub fn pack_container(
    cap: &SpaceCap,
    profile: u8,
    mem_kib: u32,
    entry: u32,
    input_max: u16,
    output_max: u16,
    segs: Vec<container::CalcSegment>,
) -> VmrtRes<container::CalcContainer> {
    let c = container::CalcContainer {
        profile,
        mem_kib,
        entry,
        segs,
        input_max,
        output_max,
    };
    let bytes = c.encode()?;
    reject_image_len(bytes.len(), cap)?;
    let (decoded, used) = container::CalcContainer::decode(&bytes)?;
    if used != bytes.len() {
        return Err(ItrErr::new(
            ItrErrCode::ContractError,
            "calc pack: container round-trip did not consume all bytes",
        ));
    }
    verify::verify_container_with_cap(&decoded, cap)?;
    Ok(decoded)
}

/// Pack a single RX blob at [`container::CODE_BASE`].
#[allow(clippy::too_many_arguments)]
pub fn pack_code(
    cap: &SpaceCap,
    profile: u8,
    mem_kib: u32,
    entry: u32,
    input_max: u16,
    output_max: u16,
    code: Vec<u8>,
) -> VmrtRes<container::CalcContainer> {
    pack_container(
        cap,
        profile,
        mem_kib,
        entry,
        input_max,
        output_max,
        vec![container::CalcSegment::rx(container::CODE_BASE as u32, code)],
    )
}

/// Pack adjacent RX / R / RW blobs at [`container::CODE_BASE`]. Tests and
/// fixtures that still emit three regions use this; production packing is
/// [`pack_elf`].
#[allow(clippy::too_many_arguments)]
pub fn pack_regions(
    cap: &SpaceCap,
    profile: u8,
    mem_kib: u32,
    entry: u32,
    input_max: u16,
    output_max: u16,
    code: Vec<u8>,
    rodata: Vec<u8>,
    init: Vec<u8>,
) -> VmrtRes<container::CalcContainer> {
    let c = container::CalcContainer::from_regions(
        profile,
        mem_kib,
        entry,
        code,
        rodata,
        init,
        input_max,
        output_max,
    );
    pack_container(
        cap,
        profile,
        mem_kib,
        entry,
        input_max,
        output_max,
        c.segs,
    )
}

/// Pack an ELF64 little-endian RISC-V (`e_machine` 243) image into a calc
/// container with an explicit `mem_kib`. Entry is the `calc_entry` symbol
/// when present, otherwise `e_entry`. `PT_LOAD` segments are stored as-is
/// (`memsz - filesz` is `.bss`).
pub fn pack_elf(
    cap: &SpaceCap,
    elf: &[u8],
    profile: u8,
    mem_kib: u32,
    input_max: u16,
    output_max: u16,
) -> VmrtRes<container::CalcContainer> {
    let parsed = parse_elf64_riscv(elf, profile)?;
    pack_container(
        cap,
        profile,
        mem_kib,
        parsed.entry,
        input_max,
        output_max,
        parsed.segs,
    )
}

/// [`pack_elf`] with the smallest `mem_kib` that fits the image,
/// [`CALC_STACK_SIZE`] and the declared IO regions.
pub fn pack_elf_min_mem(
    cap: &SpaceCap,
    elf: &[u8],
    profile: u8,
    input_max: u16,
    output_max: u16,
) -> VmrtRes<container::CalcContainer> {
    let parsed = parse_elf64_riscv(elf, profile)?;
    let guest_end = parsed
        .segs
        .iter()
        .map(|s| s.end())
        .max()
        .unwrap_or(container::CODE_BASE);
    let mem_kib = container::min_mem_kib_for(guest_end, input_max as u64, output_max as u64)
        .ok_or_else(|| {
            ItrErr::new(
                ItrErrCode::ContractError,
                "calc elf: image cannot fit in the engine memory ceiling",
            )
        })?;
    pack_container(
        cap,
        profile,
        mem_kib,
        parsed.entry,
        input_max,
        output_max,
        parsed.segs,
    )
}

struct ParsedElf {
    entry: u32,
    segs: Vec<container::CalcSegment>,
}

struct ElfSec {
    name: String,
    typ: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    entsize: u64,
}

fn elf_err(t: &str) -> ItrErr {
    ItrErr::new(ItrErrCode::ContractError, t)
}

fn ru16(b: &[u8], o: usize) -> VmrtRes<u16> {
    let s = b.get(o..o + 2).ok_or_else(|| elf_err("calc elf: truncated"))?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn ru32(b: &[u8], o: usize) -> VmrtRes<u32> {
    let s = b.get(o..o + 4).ok_or_else(|| elf_err("calc elf: truncated"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn ru64(b: &[u8], o: usize) -> VmrtRes<u64> {
    let s = b.get(o..o + 8).ok_or_else(|| elf_err("calc elf: truncated"))?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn cstr_at(buf: &[u8], off: u64) -> VmrtRes<String> {
    let off = usize::try_from(off).map_err(|_| elf_err("calc elf: name offset"))?;
    if off >= buf.len() {
        return Err(elf_err("calc elf: name offset"));
    }
    let end = buf[off..]
        .iter()
        .position(|&c| c == 0)
        .ok_or_else(|| elf_err("calc elf: unterminated name"))?;
    let bytes = &buf[off..off + end];
    Ok(std::str::from_utf8(bytes)
        .map_err(|_| elf_err("calc elf: name is not utf-8"))?
        .to_string())
}

fn sec_bytes<'a>(elf: &'a [u8], sec: &ElfSec) -> VmrtRes<&'a [u8]> {
    let off = usize::try_from(sec.offset).map_err(|_| elf_err("calc elf: section offset"))?;
    let len = usize::try_from(sec.size).map_err(|_| elf_err("calc elf: section size"))?;
    elf.get(off..off + len).ok_or_else(|| elf_err("calc elf: section out of file"))
}

fn parse_elf64_riscv(elf: &[u8], profile: u8) -> VmrtRes<ParsedElf> {
    if elf.len() < 64 || elf.get(..4) != Some(b"\x7fELF") {
        return Err(elf_err("calc elf: bad magic"));
    }
    if elf[4] != 2 {
        return Err(elf_err("calc elf: not ELF64"));
    }
    if elf[5] != 1 {
        return Err(elf_err("calc elf: not little-endian"));
    }
    let e_type = ru16(elf, 16)?;
    if e_type != 2 {
        return Err(elf_err("calc elf: not ET_EXEC"));
    }
    let e_machine = ru16(elf, 18)?;
    if e_machine != 243 {
        return Err(elf_err("calc elf: e_machine is not EM_RISCV"));
    }
    const EF_RISCV_RVC: u32 = 0x0001;
    let e_flags = ru32(elf, 48)?;
    if e_flags & EF_RISCV_RVC != 0 && !container::profile_has_compressed(profile) {
        return Err(elf_err("calc elf: EF_RISCV_RVC set but profile has no C"));
    }
    let e_entry = ru64(elf, 24)?;
    let e_shoff = ru64(elf, 40)?;
    let e_shentsize = ru16(elf, 58)? as u64;
    let e_shnum = ru16(elf, 60)? as usize;
    let e_shstrndx = ru16(elf, 62)? as usize;
    // Section headers are optional. A stripped PT_LOAD-only ELF packs via
    // `e_entry`. Relocs and `calc_entry` are read when the table is present.
    let mut sections: Vec<ElfSec> = Vec::new();
    if e_shoff != 0 && e_shnum != 0 {
        if e_shentsize < 64 {
            return Err(elf_err("calc elf: truncated section headers"));
        }
        if e_shstrndx >= e_shnum {
            return Err(elf_err("calc elf: bad shstrndx"));
        }
        sections.reserve(e_shnum);
        for i in 0..e_shnum {
            let base = e_shoff
                .checked_add((i as u64).checked_mul(e_shentsize).ok_or_else(|| elf_err("calc elf: shoff"))?)
                .ok_or_else(|| elf_err("calc elf: shoff"))?;
            let base = usize::try_from(base).map_err(|_| elf_err("calc elf: shoff"))?;
            let hdr = elf.get(base..base + 64).ok_or_else(|| elf_err("calc elf: truncated section header"))?;
            sections.push(ElfSec {
                name: String::new(),
                typ: ru32(hdr, 4)?,
                flags: ru64(hdr, 8)?,
                addr: ru64(hdr, 16)?,
                offset: ru64(hdr, 24)?,
                size: ru64(hdr, 32)?,
                link: ru32(hdr, 40)?,
                entsize: ru64(hdr, 56)?,
            });
        }
        let shstr = sec_bytes(elf, &sections[e_shstrndx])?.to_vec();
        for (i, sec) in sections.iter_mut().enumerate() {
            let base = e_shoff
                .checked_add((i as u64).checked_mul(e_shentsize).ok_or_else(|| elf_err("calc elf: shoff"))?)
                .ok_or_else(|| elf_err("calc elf: shoff"))?;
            let base = usize::try_from(base).map_err(|_| elf_err("calc elf: shoff"))?;
            let name_off = ru32(elf, base)?;
            sec.name = cstr_at(&shstr, name_off as u64)?;
        }
        const SHT_RELA: u32 = 4;
        const SHT_REL: u32 = 9;
        for sec in &sections {
            if sec.typ == SHT_REL || sec.typ == SHT_RELA {
                return Err(elf_err("calc elf: relocation section not allowed"));
            }
        }
    }

    let e_phoff = ru64(elf, 32)?;
    let e_phentsize = ru16(elf, 54)? as u64;
    let e_phnum = ru16(elf, 56)? as usize;
    if e_phoff == 0 || e_phnum == 0 || e_phentsize < 56 {
        return Err(elf_err("calc elf: missing program headers"));
    }
    const PT_LOAD: u32 = 1;
    const PT_DYNAMIC: u32 = 2;
    const PT_INTERP: u32 = 3;
    const PT_TLS: u32 = 7;
    let mut segs = Vec::new();
    for i in 0..e_phnum {
        let base = e_phoff
            .checked_add((i as u64).checked_mul(e_phentsize).ok_or_else(|| elf_err("calc elf: phoff"))?)
            .ok_or_else(|| elf_err("calc elf: phoff"))?;
        let base = usize::try_from(base).map_err(|_| elf_err("calc elf: phoff"))?;
        let hdr = elf.get(base..base + 56).ok_or_else(|| elf_err("calc elf: truncated phdr"))?;
        let p_type = ru32(hdr, 0)?;
        let p_flags = ru32(hdr, 4)?;
        let p_offset = ru64(hdr, 8)?;
        let p_vaddr = ru64(hdr, 16)?;
        let p_filesz = ru64(hdr, 32)?;
        let p_memsz = ru64(hdr, 40)?;
        match p_type {
            PT_LOAD => {}
            PT_DYNAMIC | PT_INTERP | PT_TLS => {
                return Err(elf_err("calc elf: dynamic/interp/tls not allowed"));
            }
            _ => continue,
        }
        if p_memsz == 0 {
            continue;
        }
        let vaddr = u32::try_from(p_vaddr).map_err(|_| elf_err("calc elf: p_vaddr exceeds u32"))?;
        let memsz = u32::try_from(p_memsz).map_err(|_| elf_err("calc elf: p_memsz exceeds u32"))?;
        let filesz = usize::try_from(p_filesz).map_err(|_| elf_err("calc elf: p_filesz exceeds usize"))?;
        if filesz as u64 > p_memsz {
            return Err(elf_err("calc elf: p_filesz exceeds p_memsz"));
        }
        let off = usize::try_from(p_offset).map_err(|_| elf_err("calc elf: p_offset"))?;
        let data = if filesz == 0 {
            Vec::new()
        } else {
            elf.get(off..off + filesz)
                .ok_or_else(|| elf_err("calc elf: PT_LOAD out of file"))?
                .to_vec()
        };
        let mut flags = (p_flags & 0x7) as u8;
        flags |= container::SEG_R;
        if flags & container::SEG_X != 0 && filesz as u32 != memsz {
            return Err(elf_err("calc elf: executable PT_LOAD cannot have bss"));
        }
        if flags & container::SEG_W != 0 && flags & container::SEG_X != 0 {
            return Err(elf_err("calc elf: W^X violated"));
        }
        segs.push(container::CalcSegment {
            vaddr,
            memsz,
            flags,
            data,
        });
    }
    segs.sort_by_key(|s| s.vaddr);
    if segs.is_empty() {
        return Err(elf_err("calc elf: no PT_LOAD segments"));
    }

    let mut entry = u32::try_from(e_entry).map_err(|_| elf_err("calc elf: e_entry exceeds u32"))?;
    if let Some(symtab) = sections.iter().find(|s| s.name == ".symtab") {
        let str_idx = symtab.link as usize;
        let strtab = sections.get(str_idx).ok_or_else(|| elf_err("calc elf: bad strtab"))?;
        let strbytes = sec_bytes(elf, strtab)?;
        let syms = sec_bytes(elf, symtab)?;
        let entsize = if symtab.entsize == 0 { 24 } else { symtab.entsize as usize };
        if entsize >= 24 {
            let mut off = 0;
            while off + entsize <= syms.len() {
                let name_off = ru32(syms, off)?;
                let value = ru64(syms, off + 8)?;
                if let Ok(name) = cstr_at(strbytes, name_off as u64) {
                    if name == "calc_entry" {
                        entry = u32::try_from(value)
                            .map_err(|_| elf_err("calc elf: calc_entry exceeds u32"))?;
                        break;
                    }
                }
                off += entsize;
            }
        }
    }

    Ok(ParsedElf { entry, segs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
    #[test]
    fn dual_run_halt_agrees() {
        let c = pack_code(
            &SpaceCap::new(0),
            container::CALC_PROFILE_RV64IM,
            8,
            container::CODE_BASE as u32,
            64,
            64,
            halt_text(),
        )
        .unwrap();
        let obj = CalcFnObj::build_with_host(c, CalcHostConfig::new()).unwrap();
        assert!(obj.aot_mounted());
        let exec = obj.run(
            &[],
            CalcRunLimits {
                cycles: 10_000,
                resource: i64::MAX,
                deadline: None,
            },
        );
        assert!(exec.result.is_ok(), "{exec:?}");
        assert!(exec.cycles_used > 0);
    }

    #[test]
    fn hash_cycles_max_len_does_not_overflow() {
        // `u64::MAX` bytes must round up to a block count without wrapping
        // the `len + 63` pre-addition.
        for kind in [
            HashKind::Sha2,
            HashKind::Sha3,
            HashKind::Ripemd160,
            HashKind::Keccak256,
            HashKind::Blake2s256,
            HashKind::Blake2b256,
        ] {
            let cycles = kind.cycles(u64::MAX);
            assert!(cycles > 0);
        }
        // u64::MAX bytes: pricing must saturate, not wrap or panic.
        for kind in [HashKind::Sha2, HashKind::Sha3, HashKind::Ripemd160] {
            assert_eq!(kind.cycles(u64::MAX), u64::MAX);
        }
        // Exact block rounding on a non-saturating length: 2^40 bytes.
        assert_eq!(HashKind::Sha2.cycles(1 << 40), 64 + 96 * (1u64 << 34));
        // RIPEMD-160 must never be cheaper than SHA-256 (real relative cost).
        for blocks in [0u64, 1, 100, 1 << 40] {
            assert!(HashKind::Ripemd160.cycles(blocks * 64) >= HashKind::Sha2.cycles(blocks * 64));
        }
    }

    /// Known-answer vectors: an oracle independent of the `run_hash`
    /// implementation (the digests are published constants, not computed by
    /// the code under test).
    #[test]
    fn run_hash_known_answers() {
        fn hex(s: &str) -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        }
        let abc = b"abc";
        for (kind, want) in [
            (
                HashKind::Sha2,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                HashKind::Sha3,
                "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532",
            ),
            (
                HashKind::Ripemd160,
                "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc",
            ),
            (
                HashKind::Keccak256,
                "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45",
            ),
            (
                HashKind::Blake2s256,
                "508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982",
            ),
            (
                HashKind::Blake2b256,
                "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319",
            ),
        ] {
            let mut out = vec![0u8; kind.digest_len() as usize];
            run_hash(kind, abc, &mut out);
            assert_eq!(out, hex(want), "{kind:?}");
        }
    }

    #[test]
    fn ceil_div_max_numerator_does_not_overflow() {
        assert_eq!(ceil_div(u64::MAX, 4), 1u64 << 62);
        assert_eq!(ceil_div(u64::MAX, 1), u64::MAX);
        assert_eq!(ceil_div(0, 16), 0);
        assert_eq!(ceil_div(16, 16), 1);
        assert_eq!(ceil_div(17, 16), 2);
    }

    fn halt_text() -> Vec<u8> {
        let mut c = Vec::new();
        for w in [0x0000_0513u32, 0x0000_0593, 0x0000_0893, 0x0000_0073] {
            c.extend_from_slice(&w.to_le_bytes());
        }
        c
    }

    /// Minimal ELF64 LE RISC-V with PT_LOAD for `.text` and optional `.bss`.
    fn synth_elf(bss_addr: u64, bss_size: u64, symbol: bool) -> Vec<u8> {
        let text = halt_text();
        let shstr = b"\0.text\0.bss\0.symtab\0.strtab\0.shstrtab\0";
        let strtab = if symbol { b"\0calc_entry\0".to_vec() } else { b"\0other\0".to_vec() };
        let mut sym = vec![0u8; 24];
        sym.extend_from_slice(&1u32.to_le_bytes());
        sym.extend_from_slice(&[0x12, 0]);
        sym.extend_from_slice(&1u16.to_le_bytes());
        sym.extend_from_slice(&container::CODE_BASE.to_le_bytes());
        sym.extend_from_slice(&0u64.to_le_bytes());

        let phnum: u16 = if bss_size > 0 { 2 } else { 1 };
        let mut body = vec![0u8; 64 + 56 * phnum as usize];
        let text_off = body.len() as u64;
        body.extend_from_slice(&text);
        let str_off = body.len() as u64;
        body.extend_from_slice(&strtab);
        let sym_off = body.len() as u64;
        body.extend_from_slice(&sym);
        let shstr_off = body.len() as u64;
        body.extend_from_slice(shstr);
        let shoff = body.len() as u64;

        let mut ph = |i: usize, typ: u32, flags: u32, off: u64, vaddr: u64, filesz: u64, memsz: u64| {
            let at = 64 + i * 56;
            body[at..at + 4].copy_from_slice(&typ.to_le_bytes());
            body[at + 4..at + 8].copy_from_slice(&flags.to_le_bytes());
            body[at + 8..at + 16].copy_from_slice(&off.to_le_bytes());
            body[at + 16..at + 24].copy_from_slice(&vaddr.to_le_bytes());
            body[at + 24..at + 32].copy_from_slice(&vaddr.to_le_bytes());
            body[at + 32..at + 40].copy_from_slice(&filesz.to_le_bytes());
            body[at + 40..at + 48].copy_from_slice(&memsz.to_le_bytes());
            body[at + 48..at + 56].copy_from_slice(&1u64.to_le_bytes());
        };
        ph(0, 1, 5, text_off, container::CODE_BASE, text.len() as u64, text.len() as u64);
        if bss_size > 0 {
            ph(1, 1, 6, 0, bss_addr, 0, bss_size);
        }

        let mut sh = |name: u32, typ: u32, flags: u64, addr: u64, off: u64, size: u64, link: u32, entsize: u64| {
            let mut h = Vec::new();
            h.extend_from_slice(&name.to_le_bytes());
            h.extend_from_slice(&typ.to_le_bytes());
            h.extend_from_slice(&flags.to_le_bytes());
            h.extend_from_slice(&addr.to_le_bytes());
            h.extend_from_slice(&off.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&link.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
            h.extend_from_slice(&4u64.to_le_bytes());
            h.extend_from_slice(&entsize.to_le_bytes());
            body.extend_from_slice(&h);
        };
        sh(0, 0, 0, 0, 0, 0, 0, 0);
        sh(1, 1, 6, container::CODE_BASE, text_off, text.len() as u64, 0, 0);
        sh(7, 8, 3, bss_addr, 0, bss_size, 0, 0);
        sh(12, 2, 0, 0, sym_off, sym.len() as u64, 4, 24);
        sh(20, 3, 0, 0, str_off, strtab.len() as u64, 0, 0);
        sh(28, 3, 0, 0, shstr_off, shstr.len() as u64, 0, 0);

        body[0..4].copy_from_slice(b"\x7fELF");
        body[4] = 2;
        body[5] = 1;
        body[6] = 1;
        body[16..18].copy_from_slice(&2u16.to_le_bytes());
        body[18..20].copy_from_slice(&243u16.to_le_bytes());
        body[20..24].copy_from_slice(&1u32.to_le_bytes());
        let entry = if symbol { 0 } else { container::CODE_BASE };
        body[24..32].copy_from_slice(&entry.to_le_bytes());
        body[32..40].copy_from_slice(&64u64.to_le_bytes());
        body[40..48].copy_from_slice(&shoff.to_le_bytes());
        body[52..54].copy_from_slice(&64u16.to_le_bytes());
        body[54..56].copy_from_slice(&56u16.to_le_bytes());
        body[56..58].copy_from_slice(&phnum.to_le_bytes());
        body[58..60].copy_from_slice(&64u16.to_le_bytes());
        body[60..62].copy_from_slice(&6u16.to_le_bytes());
        body[62..64].copy_from_slice(&5u16.to_le_bytes());
        body
    }

    #[test]
    fn layout_leaves_min_stack_above_guest_end() {
        let c = pack_code(
            &SpaceCap::new(0),
            container::CALC_PROFILE_RV64IM,
            4,
            container::CODE_BASE as u32,
            64,
            64,
            halt_text(),
        )
        .unwrap();
        let lay = c.layout();
        assert!(lay.fits());
        assert_eq!(lay.stack_top, lay.output_base);
        // No guard band: the stack starts exactly at the end of the image.
        assert_eq!(lay.stack_bottom, c.guest_end());
        assert!(lay.stack_bottom + CALC_STACK_SIZE <= lay.stack_top);
    }

    #[test]
    fn min_mem_kib_is_the_smallest_fitting_value() {
        let guest_end = container::CODE_BASE + halt_text().len() as u64;
        let kib = container::min_mem_kib_for(guest_end, 64, 64).expect("fits");
        assert!(kib >= container::CALC_MEM_KIB_MIN);
        let lay = container::compute_layout(guest_end, 64, 64, kib as u64 * 1024);
        assert!(lay.fits());
        if kib > container::CALC_MEM_KIB_MIN {
            let smaller = container::compute_layout(guest_end, 64, 64, (kib as u64 - 1) * 1024);
            assert!(!smaller.fits(), "kib-1 must not fit");
        }
    }

    #[test]
    fn wide_mem_roundtrip_up_to_the_engine_ceiling() {
        let cap = SpaceCap {
            calc_mem_kib_max: 70_000,
            ..SpaceCap::new(0)
        };
        let c = pack_code(
            &cap,
            container::CALC_PROFILE_RV64IM,
            70_000,
            container::CODE_BASE as u32,
            16,
            16,
            halt_text(),
        )
        .unwrap();
        assert_eq!(c.mem_kib, 70_000);
        assert_eq!(c.layout().usable_len, 70_000 * 1024);
        let (decoded, used) = container::CalcContainer::decode(&c.encode().unwrap()).unwrap();
        assert_eq!(
            used,
            container::HEADER_LEN + container::SEG_HDR_LEN + halt_text().len()
        );
        assert_eq!(decoded.mem_kib, 70_000);
        let mut over = c.clone();
        over.mem_kib = container::CALC_MEM_KIB_ENGINE_MAX + 1;
        assert!(container::CalcContainer::decode(&over.encode().unwrap()).is_err());
    }

    fn with_extra_section(mut elf: Vec<u8>, typ: u32, flags: u64, size: u64) -> Vec<u8> {
        let shnum = u16::from_le_bytes(elf[60..62].try_into().unwrap());
        let mut hdr = vec![0u8; 64];
        hdr[4..8].copy_from_slice(&typ.to_le_bytes());
        hdr[8..16].copy_from_slice(&flags.to_le_bytes());
        hdr[32..40].copy_from_slice(&size.to_le_bytes());
        elf.extend_from_slice(&hdr);
        elf[60..62].copy_from_slice(&(shnum + 1).to_le_bytes());
        elf
    }

    #[test]
    fn pack_elf_rejects_rela() {
        let data_base = container::CODE_BASE + halt_text().len() as u64;
        let base = synth_elf(data_base, 16, true);
        let rela = with_extra_section(base.clone(), 4, 0, 24);
        let err = pack_elf(&SpaceCap::new(0), &rela, 0, 4, 64, 64).unwrap_err();
        assert!(
            err.to_string().contains("relocation section not allowed"),
            "{err}"
        );
        let rel = with_extra_section(base, 9, 0, 16);
        let err = pack_elf(&SpaceCap::new(0), &rel, 0, 4, 64, 64).unwrap_err();
        assert!(
            err.to_string().contains("relocation section not allowed"),
            "{err}"
        );
    }

    #[test]
    fn pack_elf_keeps_bss_off_chain_and_uses_calc_entry() {
        let data_base = container::CODE_BASE + halt_text().len() as u64;
        let ok = pack_elf(&SpaceCap::new(0), &synth_elf(data_base, 16, true), 0, 4, 64, 64)
            .unwrap();
        assert_eq!(ok.entry, container::CODE_BASE as u32);
        assert_eq!(ok.first_rx_data(), halt_text());
        assert_eq!(ok.rw_filesz(), 0);
        assert_eq!(ok.layout().stack_bottom, data_base + 16);
        let min = pack_elf_min_mem(&SpaceCap::new(0), &synth_elf(data_base, 16, true), 0, 64, 64)
            .unwrap();
        assert!(min.layout().fits());
        assert!(min.mem_kib <= 8);
        let err = pack_elf(
            &SpaceCap::new(0),
            &synth_elf(data_base, 0x800, true),
            0,
            4,
            64,
            64,
        )
        .unwrap_err();
        assert!(err.to_string().contains("memory overflows"), "{err}");
        let from_ehdr =
            pack_elf(&SpaceCap::new(0), &synth_elf(data_base, 16, false), 0, 4, 64, 64).unwrap();
        assert_eq!(from_ehdr.entry, container::CODE_BASE as u32);
    }

    #[test]
    fn pack_elf_accepts_phdr_only_and_rejects_et_dyn() {
        let data_base = container::CODE_BASE + halt_text().len() as u64;
        let mut stripped = synth_elf(data_base, 16, false);
        stripped[40..48].copy_from_slice(&0u64.to_le_bytes());
        stripped[60..62].copy_from_slice(&0u16.to_le_bytes());
        stripped[62..64].copy_from_slice(&0u16.to_le_bytes());
        let ok = pack_elf(&SpaceCap::new(0), &stripped, 0, 4, 64, 64).unwrap();
        assert_eq!(ok.entry, container::CODE_BASE as u32);
        let mut dyn_elf = synth_elf(data_base, 16, false);
        dyn_elf[16..18].copy_from_slice(&3u16.to_le_bytes());
        let err = pack_elf(&SpaceCap::new(0), &dyn_elf, 0, 4, 64, 64).unwrap_err();
        assert!(err.to_string().contains("ET_EXEC"), "{err}");
    }
}
