//! Calc container v1 binary layout: fixed 32-byte big-endian
//! header + code + rodata + initialized data.
//!
//! Memory image carried by the container:
//! - `code`: RV64IMC(+Zbb/Zba/Zbs/Zbc) instruction stream — decoded and priced
//!   verifier, executable, read-only at runtime.
//! - `rodata`: constant data (lookup tables etc.), kept adjacent to code so
//!   position-relative addressing (`auipc`) stays valid. Readable, but
//!   non-executable (jump targets must stay inside `code`) and read-only.
//! - `init`: initialized writable data, loaded at the 16-byte-aligned
//!   `data_base`.
//!
//! There is deliberately no checksum field: the container bytes are already
//! committed by consensus once deployed (EVM/CKB-VM carry no container
//! checksums either), and a CRC would only add deploy-time cost without
//! authenticity. The two trailing header bytes are reserved-must-be-zero for
//! future minor fields.

use crate::rt::{ItrErr, ItrErrCode, VmrtRes};

pub const CALC_MAGIC: [u8; 4] = *b"HCF1";
pub const CALC_VERSION: u8 = 1;
/// ABI 1: bytes -> bytes (single input byte string, single output byte string).
pub const CALC_ABI_BYTES_TO_BYTES: u8 = 1;

pub const CALC_MEM_KIB_MIN: u16 = 4;
pub const CALC_MEM_KIB_MAX: u16 = 64;
pub const CALC_IO_MAX: u16 = 4096;
pub const HEADER_LEN: usize = 32;

/// Runtime memory map derived deterministically from the header:
/// code | rodata | data | free | input | output | guard | stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcLayout {
    pub mem_len: u64,
    /// Executable region `[0, code_len)`; the only valid jump-target area.
    pub code_len: u64,
    /// Read-only constant region `[rodata_base, rodata_base + rodata_len)`.
    pub rodata_base: u64,
    pub rodata_len: u64,
    /// Initialized data base (16-byte aligned past code+rodata). Stores
    /// below this address trap (code and rodata are both read-only).
    pub data_base: u64,
    pub init_len: u64,
    /// Input region base (16-byte aligned).
    pub input_base: u64,
    pub input_max: u64,
    /// Output region base (16-byte aligned).
    pub output_base: u64,
    pub output_max: u64,
    /// Stack-overflow guard band `[guard_base, stack_bottom)`: stores that
    /// intersect it trap (see `CALC_STACK_GUARD`).
    pub guard_base: u64,
    /// Bottom of the stack region (16-byte aligned).
    pub stack_bottom: u64,
    /// Initial stack pointer (top of the reserved stack, 16-byte aligned).
    pub stack_top: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcContainer {
    pub abi: u8,
    pub mem_kib: u16,
    pub entry: u32,
    pub code: Vec<u8>,
    pub rodata: Vec<u8>,
    pub init: Vec<u8>,
    pub input_max: u16,
    pub output_max: u16,
}

impl CalcContainer {
    /// Header invariants shared by the wire decoder, the packer and the
    /// deploy-time verifier. `verify_container` calls this so a directly
    /// constructed container cannot bypass the header checks that `decode`
    /// performs on wire input.
    pub(crate) fn validate_header(&self) -> VmrtRes<()> {
        let err = |t: &str| ItrErr::new(ItrErrCode::ContractError, t);
        if self.abi != CALC_ABI_BYTES_TO_BYTES {
            return Err(err("calc container abi unsupported"));
        }
        if !(CALC_MEM_KIB_MIN..=CALC_MEM_KIB_MAX).contains(&self.mem_kib) {
            return Err(err("calc container mem_kib out of range"));
        }
        if self.output_max == 0 || self.output_max > CALC_IO_MAX || self.input_max > CALC_IO_MAX {
            return Err(err("calc container io size invalid"));
        }
        if self.code.is_empty() || self.code.len() % 2 != 0 {
            return Err(err("calc container code_len invalid"));
        }
        Ok(())
    }

    /// Deterministic memory layout; must be identical in verifier, loader
    /// and pack tool.
    pub fn layout(&self) -> CalcLayout {
        // Saturating math keeps `layout` total; the decode-side io-overlap
        // check rejects anything that does not fit.
        let mem_len = self.mem_kib as u64 * 1024;
        // rodata sits at a 16-byte boundary past the code stream, mirroring
        // the pack tool's link script (`. = ALIGN(16)` before `.rodata`) so
        // position-relative constant addressing stays valid.
        let rodata_base = (self.code.len() as u64 + 15) & !15;
        let data_base = (rodata_base + self.rodata.len() as u64 + 15) & !15;
        let stack_top = mem_len & !15;
        let stack_bottom = stack_top.saturating_sub(crate::calc::CALC_STACK_SIZE);
        let guard_base = stack_bottom.saturating_sub(crate::calc::CALC_STACK_GUARD);
        let output_base = guard_base.saturating_sub(self.output_max as u64) & !15;
        let input_base = output_base.saturating_sub(self.input_max as u64) & !15;
        CalcLayout {
            mem_len,
            code_len: self.code.len() as u64,
            rodata_base,
            rodata_len: self.rodata.len() as u64,
            data_base,
            init_len: self.init.len() as u64,
            input_base,
            input_max: self.input_max as u64,
            output_base,
            output_max: self.output_max as u64,
            guard_base,
            stack_bottom,
            stack_top,
        }
    }

    pub fn encode(&self) -> VmrtRes<Vec<u8>> {
        let err = |t: &str| ItrErr::new(ItrErrCode::ContractError, t);
        let code_len = u32::try_from(self.code.len())
            .map_err(|_| err("calc container code length exceeds u32"))?;
        let init_len = u32::try_from(self.init.len())
            .map_err(|_| err("calc container init length exceeds u32"))?;
        let rodata_len = u32::try_from(self.rodata.len())
            .map_err(|_| err("calc container rodata length exceeds u32"))?;
        let capacity = HEADER_LEN
            .checked_add(self.code.len())
            .and_then(|n| n.checked_add(self.rodata.len()))
            .and_then(|n| n.checked_add(self.init.len()))
            .ok_or_else(|| err("calc container length overflow"))?;
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&CALC_MAGIC);
        out.push(CALC_VERSION);
        out.push(0); // reserved (former isa-profile byte; single ISA now)
        out.push(self.abi);
        out.push(0); // reserved (former fncnf byte)
        out.extend_from_slice(&self.mem_kib.to_be_bytes());
        out.extend_from_slice(&self.entry.to_be_bytes());
        out.extend_from_slice(&code_len.to_be_bytes());
        out.extend_from_slice(&init_len.to_be_bytes());
        out.extend_from_slice(&self.input_max.to_be_bytes());
        out.extend_from_slice(&self.output_max.to_be_bytes());
        out.extend_from_slice(&rodata_len.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes()); // reserved, must stay zero
        out.extend_from_slice(&self.code);
        out.extend_from_slice(&self.rodata);
        out.extend_from_slice(&self.init);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), ItrErr> {
        let err = |t: &str| ItrErr::new(ItrErrCode::ContractError, t);
        if bytes.len() < HEADER_LEN {
            return Err(err("calc container too short"));
        }
        if bytes[0..4] != CALC_MAGIC {
            return Err(err("calc container magic mismatch"));
        }
        if bytes[4] != CALC_VERSION {
            return Err(err("calc container version mismatch"));
        }
        if bytes[5] != 0 {
            return Err(err("calc container reserved header byte 5 must be zero"));
        }
        if bytes[6] != CALC_ABI_BYTES_TO_BYTES {
            return Err(err("calc container abi unsupported"));
        }
        if bytes[7] != 0 {
            return Err(err("calc container reserved header byte 7 must be zero"));
        }
        let mem_kib = u16::from_be_bytes([bytes[8], bytes[9]]);
        if !(CALC_MEM_KIB_MIN..=CALC_MEM_KIB_MAX).contains(&mem_kib) {
            return Err(err("calc container mem_kib out of range"));
        }
        let entry = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        let code_len = u32::from_be_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]) as usize;
        let init_len = u32::from_be_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize;
        let input_max = u16::from_be_bytes([bytes[22], bytes[23]]);
        let output_max = u16::from_be_bytes([bytes[24], bytes[25]]);
        let rodata_len = u32::from_be_bytes([bytes[26], bytes[27], bytes[28], bytes[29]]) as usize;
        if bytes[30] != 0 || bytes[31] != 0 {
            return Err(err("calc container reserved header bytes must be zero"));
        }
        let total = HEADER_LEN
            .checked_add(code_len)
            .and_then(|t| t.checked_add(rodata_len))
            .and_then(|t| t.checked_add(init_len))
            .ok_or_else(|| err("calc container length overflow"))?;
        if bytes.len() < total {
            return Err(err("calc container truncated payload"));
        }
        if bytes.len() != total {
            return Err(err("calc container trailing bytes"));
        }
        let payload = &bytes[HEADER_LEN..total];
        if code_len == 0 || code_len % 2 != 0 {
            return Err(err("calc container code_len invalid"));
        }
        if output_max == 0 || output_max > CALC_IO_MAX || input_max > CALC_IO_MAX {
            return Err(err("calc container io size invalid"));
        }
        let c = CalcContainer {
            abi: bytes[6],
            mem_kib,
            entry,
            code: payload[..code_len].to_vec(),
            rodata: payload[code_len..code_len + rodata_len].to_vec(),
            init: payload[code_len + rodata_len..].to_vec(),
            input_max,
            output_max,
        };
        // Layout sanity: everything must fit without overlap.
        let lay = c.layout();
        if lay.input_base < lay.data_base + lay.init_len {
            return Err(err("calc container memory overflows (io overlaps data)"));
        }
        if entry as u64 >= lay.code_len {
            return Err(err("calc container entry outside code"));
        }
        Ok((c, total))
    }
}
