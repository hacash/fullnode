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
use crate::rt::ItrErrCode::CalcTrap;

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
        let blocks = (len + 63) / 64;
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
}

/// A verified calc function attached to a contract (built once at load).
#[derive(Debug, Clone)]
pub struct CalcFnObj {
    pub container: container::CalcContainer,
    pub module: verify::CalcModule,
}

impl CalcFnObj {
    /// Verify a container and build the executable object (deploy + load).
    pub fn build(c: container::CalcContainer) -> VmrtRes<Self> {
        let module = verify::verify_container(&c)?;
        Ok(Self { container: c, module })
    }

    /// Run with the HVM gas rules; never touches host state.
    pub fn run(&self, input: &[u8], cycle_budget: u64) -> CalcExec {
        let lay = self.container.layout();
        let in_res = ceil_div(input.len() as u64, 16) as i64;
        let exec = interp::run(&self.module, &self.container, input, cycle_budget);
        match exec {
            Ok(r) => {
                let out_res = ceil_div(r.output.len() as u64, 16) as i64;
                let mem_res = (lay.mem_len / 1024) as i64 * crate::calc::CALC_MEM_RESOURCE_PER_KIB;
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
                }),
                cycles_used: used,
                resource: in_res + (lay.mem_len / 1024) as i64 * crate::calc::CALC_MEM_RESOURCE_PER_KIB,
            },
        }
    }
}

pub(crate) fn trap(tip: &str) -> ItrErr {
    ItrErr::new(CalcTrap, tip)
}

/// ceil(a / b) for non-negative a and positive b.
#[inline(always)]
pub(crate) fn ceil_div(a: u64, b: u64) -> u64 {
    (a + b - 1) / b
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
    // reject layouts that do not fit before returning
    verify::verify_container(&c)?;
    Ok(c)
}

/// FitSH `calc fn` backend hook (phase 2): lower fitsh IR to a RV64IMC
/// image. The signature is fixed now so the wire/loader side cannot drift;
/// the implementation lands with the phase-2 backend.
pub fn lower_ir_to_rv64imc(_nodes: &[Box<dyn crate::ir::IRNode>]) -> VmrtRes<Vec<u8>> {
    Err(ItrErr::new(ItrErrCode::ContractError, "fitsh calc backend lands in phase 2"))
}
