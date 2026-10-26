//! HVM calc func: RV64IMC pure-compute kernels.
//!
//! Container (`container.rs`) + whitelist decoder (`decode.rs`) + deploy-time
//! verifier with basic-block pricing (`verify.rs`) + pre-decoded interpreter
//! (`interp.rs`). The whole pipeline is deterministic and free of host
//! access: a calc kernel can only see its own flat memory and the hash
//! builtins behind `ecall`.

pub mod container;
pub mod decode;
pub mod interp;
pub mod verify;

use crate::rt::{ItrErr, ItrErrCode, VmrtRes};

/// Exchange rate: 1 compute = N cycles. Provisional 4; to be re-measured
/// and locked (e.g. 32 for the AOT-era equivalence point) before activation.
pub const CYCLES_PER_COMPUTE: u64 = 4;

/// Resource charged per declared KiB of calc memory (input bytes, output
/// bytes and declared memory are the only resource charges on the calc path).
pub const CALC_MEM_RESOURCE_PER_KIB: i64 = 8;

/// Fixed interpreter stack reservation (bytes), carved out of the declared
/// memory, 16-byte aligned top.
pub const CALC_STACK_SIZE: u64 = 2 * 1024;

/// Initial `ra` value: `jalr`/`ret` targeting this sentinel is the canonical
/// return from the entry function (never a valid code address).
pub const CALC_RETURN_SENTINEL: u64 = 0xFFFF_FFFF_FFFF_FFF0;

/// Hash builtin numbers behind `ecall` (a7), aligned with the native hash
/// catalog (`rt/native_catalog.rs`).
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
    pub fn cycles(self, len: u64) -> u64 {
        let blocks = len.div_ceil(64);
        match self {
            HashKind::Sha2 => 30 + 12 * blocks,
            HashKind::Sha3 => 30 + 20 * blocks,
            HashKind::Ripemd160 => 20 + 10 * blocks,
            HashKind::Keccak256 => 30 + 20 * blocks,
            HashKind::Blake2s256 => 30 + 8 * blocks,
            HashKind::Blake2b256 => 30 + 8 * blocks,
        }
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

/// Result of one calc invocation through the HVM call layer, carrying the
/// gas actually consumed so the caller can charge it on both the success
/// and the abort path (failures charge up to the stop point).
#[derive(Debug, Clone)]
pub struct CalcExec {
    /// Output bytes on success; abort classification otherwise.
    pub result: Result<Vec<u8>, CalcAbort2>,
    /// Cycles consumed up to the stop point (exact on success).
    pub cycles_used: u64,
    /// Resource charge: ceil(in/16) + ceil(out/16) + 8 per declared KiB.
    pub resource: i64,
}

/// Abort classification mirroring `interp::CalcAbort` without the message
/// payload (mapped to `ItrErr` by the caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcAbort2 {
    Trap,
    OutOfGas,
    /// The host machine's wall-clock deadline elapsed mid-run.
    Deadline,
    /// The pre-run resource budget cannot cover the minimum charge.
    OutOfResource,
    /// The declared/actual value size exceeds the host `SpaceCap`.
    ValueTooLarge,
}

/// Per-call execution limits. `resource < 0` means the resource budget is
/// unlimited; `deadline` is only set by non-consensus builders.
#[derive(Debug, Clone, Copy)]
pub struct CalcRunLimits {
    pub cycles: u64,
    pub resource: i64,
    pub value_size: usize,
    pub deadline: Option<std::time::Instant>,
}

/// A verified calc function attached to a contract (built once at load).
#[derive(Debug, Clone)]
pub struct CalcFnObj {
    container: container::CalcContainer,
    module: verify::CalcModule,
}

impl CalcFnObj {
    /// Verify a container and build the executable object (deploy + load).
    pub fn build(c: container::CalcContainer) -> VmrtRes<Self> {
        let module = verify::verify_container(&c)?;
        Ok(Self { container: c, module })
    }

    /// Run with the HVM gas rules; never touches host state.
    pub fn run(&self, input: &[u8], limits: CalcRunLimits) -> CalcExec {
        let lay = self.container.layout();
        let in_res = ceil_div(input.len() as u64, 16) as i64;
        let mem_res = (lay.mem_len / 1024) as i64 * crate::calc::CALC_MEM_RESOURCE_PER_KIB;
        // Pre-flight checks charge base gas only: they run before any cycle.
        if input.len() as u64 > lay.input_max {
            return CalcExec {
                result: Err(CalcAbort2::Trap),
                cycles_used: 0,
                resource: 0,
            };
        }
        if input.len() > limits.value_size || lay.output_max > limits.value_size as u64 {
            return CalcExec {
                result: Err(CalcAbort2::ValueTooLarge),
                cycles_used: 0,
                resource: 0,
            };
        }
        if limits.resource >= 0 && in_res.saturating_add(mem_res) > limits.resource {
            return CalcExec {
                result: Err(CalcAbort2::OutOfResource),
                cycles_used: 0,
                resource: 0,
            };
        }
        let exec = interp::run(
            &self.module,
            &self.container,
            input,
            limits.cycles,
            limits.deadline,
        );
        match exec {
            Ok(r) => {
                let out_res = ceil_div(r.output.len() as u64, 16) as i64;
                CalcExec {
                    result: Ok(r.output),
                    cycles_used: r.cycles_used,
                    resource: in_res + out_res + mem_res,
                }
            }
            Err((abort, used)) => CalcExec {
                result: Err(match abort {
                    interp::CalcAbort::Trap(_) => CalcAbort2::Trap,
                    interp::CalcAbort::OutOfGas => CalcAbort2::OutOfGas,
                    interp::CalcAbort::Deadline => CalcAbort2::Deadline,
                }),
                cycles_used: used,
                resource: in_res + mem_res,
            },
        }
    }
}

/// ceil(a / b) for non-negative a and positive b.
#[inline(always)]
pub(crate) fn ceil_div(a: u64, b: u64) -> u64 {
    a / b + u64::from(a % b != 0)
}


/// Build a calc container from a linked bare-metal image produced by the
/// Rust cross-build pack step. `code` is the executable image
/// (.text), `rodata` the constant image placed at the next 16-byte boundary
/// (the fixture link scripts use `. = ALIGN(16)` before `.rodata` — keep
/// them in sync), `init` optional initialized writable data.
pub fn pack_container(
    mem_kib: u16,
    entry: u32,
    input_max: u16,
    output_max: u16,
    code: Vec<u8>,
    rodata: Vec<u8>,
    init: Vec<u8>,
) -> VmrtRes<container::CalcContainer> {
    let c = container::CalcContainer {
        isa_profile: container::CALC_ISA_RV64IMC,
        abi: container::CALC_ABI_BYTES_TO_BYTES,
        fncnf: 0b11,
        mem_kib,
        entry,
        code,
        rodata,
        init,
        input_max,
        output_max,
    };
    // Round-trip through the wire format so a directly constructed
    // container cannot bypass the decoder's header/length checks, then run
    // the full verifier on exactly the bytes that will be stored.
    let bytes = c.encode()?;
    let (decoded, used) = container::CalcContainer::decode(&bytes)?;
    if used != bytes.len() {
        return Err(ItrErr::new(
            ItrErrCode::ContractError,
            "calc pack: container round-trip did not consume all bytes",
        ));
    }
    verify::verify_container(&decoded)?;
    Ok(decoded)
}

/// FitSH `calc fn` backend hook (phase 2): lower fitsh IR to a RV64IMC
/// image. The signature is fixed now so the wire/loader side cannot drift;
/// the implementation lands with the phase-2 backend.
pub fn lower_ir_to_rv64imc(_nodes: &[Box<dyn crate::ir::IRNode>]) -> VmrtRes<Vec<u8>> {
    Err(ItrErr::new(ItrErrCode::ContractError, "fitsh calc backend lands in phase 2"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Exact block rounding: 2^64 / 64 = 2^58 blocks.
        assert_eq!(HashKind::Sha2.cycles(u64::MAX), 30 + 12 * (1u64 << 58));
    }

    #[test]
    fn ceil_div_max_numerator_does_not_overflow() {
        assert_eq!(ceil_div(u64::MAX, 4), 1u64 << 62);
        assert_eq!(ceil_div(u64::MAX, 1), u64::MAX);
        assert_eq!(ceil_div(0, 16), 0);
        assert_eq!(ceil_div(16, 16), 1);
        assert_eq!(ceil_div(17, 16), 2);
    }
}
