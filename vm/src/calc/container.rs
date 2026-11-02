//! Calc container v1 binary layout: fixed 36-byte big-endian
//! header + code + rodata + initialized data.
//!
//! Memory image carried by the container:
//! - `code`: RV64IM instruction stream (optionally +C and/or
//!   Zba/Zbb/Zbs/Zbc, selected by the profile byte) — decoded and priced by
//!   the verifier, executable, read-only at runtime. Mapped at [`CODE_BASE`].
//! - `rodata`: constant data (lookup tables etc.), kept adjacent to code so
//!   position-relative addressing (`auipc`) stays valid. Readable, but
//!   non-executable (jump targets must stay inside `code`) and read-only.
//! - `init`: initialized writable data, loaded at the 16-byte-aligned
//!   `data_base`.
//!
//! There is deliberately no checksum field: the container bytes are already
//! committed by consensus once deployed (EVM/CKB-VM carry no container
//! checksums either), and a CRC would only add deploy-time cost without
//! authenticity. Header bytes 32..36 are `bss_len` (u32 big-endian). Zero
//! means no extra `.bss`.
//! Header byte 5 is the ISA profile bitfield: bit 0 = Zba/Zbb/Zbs/Zbc,
//! bit 1 = RVC. See [`CALC_PROFILE_RV64IM`], [`CALC_PROFILE_RV64IM_B`],
//! [`CALC_PROFILE_RV64IMC`], [`CALC_PROFILE_RV64IMC_B`].

use crate::rt::{ItrErr, ItrErrCode, VmrtRes};

pub const CALC_MAGIC: [u8; 4] = *b"HCF1";
pub const CALC_VERSION: u8 = 1;
/// ABI 1: bytes -> bytes (single input byte string, single output byte string).
pub const CALC_ABI_BYTES_TO_BYTES: u8 = 1;

/// Profile byte bit 0: Zba/Zbb/Zbs/Zbc. Orthogonal to [`CALC_PROFILE_COMPRESSED`].
pub const CALC_PROFILE_BITMANIP: u8 = 0b01;
/// Profile byte bit 1: RVC (16-bit) encodings. Orthogonal to [`CALC_PROFILE_BITMANIP`].
pub const CALC_PROFILE_COMPRESSED: u8 = 0b10;

/// Profile 0: RV64IM + Zicclsm. No C, no Zba/Zbb/Zbs/Zbc. zkVM baseline.
pub const CALC_PROFILE_RV64IM: u8 = 0;
/// Profile 1: RV64IM + Zicclsm + Zba/Zbb/Zbs/Zbc. No C.
pub const CALC_PROFILE_RV64IM_B: u8 = CALC_PROFILE_BITMANIP;
/// Profile 2: RV64IMC + Zicclsm. No Zba/Zbb/Zbs/Zbc.
pub const CALC_PROFILE_RV64IMC: u8 = CALC_PROFILE_COMPRESSED;
/// Profile 3: RV64IMC + Zicclsm + Zba/Zbb/Zbs/Zbc.
pub const CALC_PROFILE_RV64IMC_B: u8 = CALC_PROFILE_BITMANIP | CALC_PROFILE_COMPRESSED;

/// Bits outside the two-axis matrix are not a profile.
pub const fn profile_known(profile: u8) -> bool {
    profile & !(CALC_PROFILE_BITMANIP | CALC_PROFILE_COMPRESSED) == 0
}

pub const fn profile_has_bitmanip(profile: u8) -> bool {
    profile & CALC_PROFILE_BITMANIP != 0
}

pub const fn profile_has_compressed(profile: u8) -> bool {
    profile & CALC_PROFILE_COMPRESSED != 0
}

/// Guest address of the first code byte. Addresses in `[0, CODE_BASE)` are
/// unmapped. CONSENSUS CONSTANT.
pub const CODE_BASE: u64 = 0x1000;

/// Domain tag for the canonical image ID (SHA-256 over this prefix plus
/// the encoded container). CONSENSUS CONSTANT.
pub const IMAGE_ID_DOMAIN: &[u8] = b"hacash-calc-image-v1";

pub const CALC_MEM_KIB_MIN: u32 = 4;
/// Consensus v1 verify cap (`CALC_CONFIG_V1.mem_kib_max`, and the default
/// of `SpaceCap::calc_mem_kib_max`). Wire `decode` only requires
/// [`CALC_MEM_KIB_MIN`] and rejects above [`CALC_MEM_KIB_ENGINE_MAX`].
pub const CALC_MEM_KIB_MAX: u32 = 64;
/// Largest `mem_kib` either engine can address. AOT compares guest
/// addresses with a signed 32-bit immediate, so
/// `CODE_BASE + mem_kib * 1024` must be `<= i32::MAX`. Technical ceiling,
/// not an admission policy: a sidechain raises `SpaceCap::calc_mem_kib_max`
/// up to this value and does not change the header again.
pub const CALC_MEM_KIB_ENGINE_MAX: u32 = (((i32::MAX as u64) - CODE_BASE) / 1024) as u32;
pub const HEADER_LEN: usize = 36;

/// Runtime memory map derived deterministically from the header.
///
/// `mem_kib` sizes the usable region *above* [`CODE_BASE`]:
/// `usable_len = mem_kib * 1024`, `mem_len = CODE_BASE + usable_len`
/// (exclusive end of the guest address space). Resource and setup charges
/// use `mem_kib` / `usable_len`, not the unmapped low page.
///
/// Layout (all addresses absolute), low to high:
/// unmapped | code (RX) | rodata (R) | init (RW) | bss (RW) |
/// guard (store-trap) | stack (RW, grows down from `stack_top`) |
/// output (RW) | input through `mem_len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcLayout {
    /// Exclusive end of the guest address space (`CODE_BASE + usable_len`).
    pub mem_len: u64,
    /// Declared usable bytes above [`CODE_BASE`] (`mem_kib * 1024`).
    pub usable_len: u64,
    /// Executable region `[CODE_BASE, CODE_BASE + code_len)`.
    pub code_len: u64,
    /// Read-only constant region `[rodata_base, rodata_base + rodata_len)`.
    pub rodata_base: u64,
    pub rodata_len: u64,
    /// First writable address: `align16(rodata_end)`. Stores below this
    /// trap (code, alignment pad before rodata, rodata, and the
    /// `[rodata_end, data_base)` hole). Init is copied here. Linker
    /// scripts must `ALIGN(16)` before `.data` so `.data`/`.bss` VMAs
    /// equal this address (nothing is linked into the hole).
    pub data_base: u64,
    pub init_len: u64,
    /// Zero-filled `[data_base + init_len, data_base + init_len + bss_len)`.
    pub bss_len: u64,
    /// Input region base (16-byte aligned). The region runs through `mem_len`.
    pub input_base: u64,
    pub input_max: u64,
    /// Output region base (16-byte aligned), just below the input region.
    pub output_base: u64,
    pub output_max: u64,
    /// Stack-overflow guard `[guard_base, stack_bottom)`, between `.bss`
    /// and the downward-growing stack.
    pub guard_base: u64,
    /// Lowest stack address. Stores that intersect the guard below it trap.
    pub stack_bottom: u64,
    /// Initial stack pointer (16-byte aligned). Stack grows down from here
    /// and occupies every byte above the guard up to the output region.
    pub stack_top: u64,
}

impl CalcLayout {
    /// Extra `.bss` bytes that still leave the guard and the minimum stack
    /// below `stack_top`. `stack_top` does not move with `bss_len`.
    pub const fn free_rw_bytes(&self) -> u64 {
        let used = self.data_base.saturating_add(self.init_len).saturating_add(self.bss_len);
        let reserved = crate::calc::CALC_STACK_GUARD.saturating_add(crate::calc::CALC_STACK_SIZE);
        self.stack_top.saturating_sub(used).saturating_sub(reserved)
    }

    /// `.bss` occupies `[data_base + init_len, data_base + init_len + bss_len)`.
    /// After it, the 256-byte guard and [`crate::calc::CALC_STACK_SIZE`]
    /// minimum stack must still end at or before `stack_top`. The header
    /// stores `bss_len` as `u32`; a larger value is rejected at pack.
    pub fn check_bss_fit(&self, bss_len: u64) -> VmrtRes<()> {
        let err = || {
            ItrErr::new(
                ItrErrCode::ContractError,
                "calc container memory overflows (io overlaps bss)",
            )
        };
        if bss_len > u32::MAX as u64 {
            return Err(ItrErr::new(
                ItrErrCode::ContractError,
                "calc container bss_len exceeds u32",
            ));
        }
        let need = self
            .data_base
            .checked_add(self.init_len)
            .and_then(|end| end.checked_add(bss_len))
            .and_then(|end| end.checked_add(crate::calc::CALC_STACK_GUARD))
            .and_then(|end| end.checked_add(crate::calc::CALC_STACK_SIZE));
        match need {
            Some(end) if end <= self.stack_top => Ok(()),
            _ => Err(err()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcContainer {
    pub profile: u8,
    pub abi: u8,
    pub mem_kib: u32,
    /// Absolute guest PC of the entry instruction (not a code-relative
    /// offset). Must satisfy `CODE_BASE <= entry < CODE_BASE + code_len`
    /// and be 4-byte aligned on an instruction boundary.
    pub entry: u32,
    pub code: Vec<u8>,
    pub rodata: Vec<u8>,
    pub init: Vec<u8>,
    /// Header bytes 32..36. Zero means the image has no `.bss`.
    pub bss_len: u32,
    pub input_max: u16,
    pub output_max: u16,
}

/// Pure memory map. Saturating math keeps this total; callers reject
/// a layout whose io region overlaps data or the unmapped low page.
/// `usable_len` is `mem_kib * 1024`.
pub(crate) fn compute_layout(
    code_len: u64,
    rodata_len: u64,
    init_len: u64,
    bss_len: u64,
    input_max: u64,
    output_max: u64,
    usable_len: u64,
) -> CalcLayout {
    let mem_len = CODE_BASE.saturating_add(usable_len);
    // rodata sits at a 16-byte boundary past the code stream, mirroring
    // the pack tool's link script (`. = ALIGN(16)` before `.rodata`) so
    // position-relative constant addressing stays valid. Addresses are
    // absolute (code is mapped at CODE_BASE).
    let rodata_base = (CODE_BASE.saturating_add(code_len).saturating_add(15)) & !15;
    // 16-byte aligned past rodata. The hole `[rodata_end, data_base)`
    // is read-only (addr < data_base) and empty: linkers ALIGN(16)
    // before `.data` so `.data`/`.bss` start here, never in the hole.
    let data_base = (rodata_base.saturating_add(rodata_len).saturating_add(15)) & !15;
    let bss_end = data_base.saturating_add(init_len).saturating_add(bss_len);
    let guard_base = bss_end;
    let stack_bottom = guard_base.saturating_add(crate::calc::CALC_STACK_GUARD);
    // Input occupies the top of the guest space through `mem_len`.
    // Output sits immediately below it. The stack is whatever remains
    // between the guard and the output region; `sp` starts at `stack_top`.
    let input_base = mem_len.saturating_sub(input_max) & !15;
    let output_base = input_base.saturating_sub(output_max) & !15;
    let stack_top = output_base;
    CalcLayout {
        mem_len,
        usable_len,
        code_len,
        rodata_base,
        rodata_len,
        data_base,
        init_len,
        bss_len,
        input_base,
        input_max,
        output_base,
        output_max,
        guard_base,
        stack_bottom,
        stack_top,
    }
}

impl CalcContainer {
    /// Header, layout and entry checks shared by wire `decode`,
    /// [`Self::validate_header`] and the verifier. Does not apply
    /// `mem_kib_max` (that is verify-time) or config feature gates.
    fn validate_invariants(&self) -> VmrtRes<()> {
        let err = |t: &str| ItrErr::new(ItrErrCode::ContractError, t);
        if !profile_known(self.profile) {
            return Err(err("calc container unknown isa profile"));
        }
        if self.abi != CALC_ABI_BYTES_TO_BYTES {
            return Err(err("calc container abi unsupported"));
        }
        if self.mem_kib < CALC_MEM_KIB_MIN || self.mem_kib > CALC_MEM_KIB_ENGINE_MAX {
            return Err(err("calc container mem_kib out of range"));
        }
        if self.output_max == 0 {
            return Err(err("calc container io size invalid"));
        }
        let compressed = profile_has_compressed(self.profile);
        let code_align = if compressed { 2 } else { 4 };
        if self.code.is_empty() || self.code.len() % code_align != 0 {
            return Err(err("calc container code_len invalid"));
        }
        let lay = self.layout();
        let reserved_end = lay
            .stack_bottom
            .checked_add(crate::calc::CALC_STACK_SIZE);
        match reserved_end {
            Some(end) if end <= lay.stack_top && lay.stack_top == lay.output_base => {}
            _ => return Err(err("calc container memory overflows (io overlaps data)")),
        }
        if lay.input_base < CODE_BASE || lay.output_base < CODE_BASE {
            return Err(err("calc container memory overflows (io overlaps unmapped)"));
        }
        if lay.input_base < lay.output_base {
            return Err(err("calc container memory overflows (io overlaps data)"));
        }
        let entry_u = self.entry as u64;
        if entry_u < CODE_BASE || entry_u >= CODE_BASE + lay.code_len {
            return Err(err("calc container entry outside code"));
        }
        if entry_u % code_align as u64 != 0 {
            return Err(err(if compressed {
                "calc container entry not 2-byte aligned"
            } else {
                "calc container entry not 4-byte aligned"
            }));
        }
        Ok(())
    }

    /// Header invariants shared by the wire decoder, the packer and the
    /// deploy-time verifier. `verify_container` calls this so a directly
    /// constructed container cannot bypass the header checks that `decode`
    /// performs on wire input. The upper memory cap is `cfg.mem_kib_max`
    /// (v1: 64); wire `decode` does not apply it.
    pub(crate) fn validate_header(&self, cfg: &crate::calc::config::CalcConfig) -> VmrtRes<()> {
        self.validate_invariants()?;
        if self.mem_kib > cfg.mem_kib_max {
            return Err(ItrErr::new(
                ItrErrCode::ContractError,
                "calc verify: mem_kib exceeds config cap",
            ));
        }
        Ok(())
    }

    /// Deterministic memory layout; must be identical in verifier, loader
    /// and pack tool. Usable length is `mem_kib * 1024`.
    pub fn layout(&self) -> CalcLayout {
        compute_layout(
            self.code.len() as u64,
            self.rodata.len() as u64,
            self.init.len() as u64,
            self.bss_len as u64,
            self.input_max as u64,
            self.output_max as u64,
            self.mem_kib as u64 * 1024,
        )
    }

    /// Canonical image ID: SHA-256 over `IMAGE_ID_DOMAIN || encode()`.
    /// Fully determined by the wire bytes; identical for every node.
    pub fn image_id(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(IMAGE_ID_DOMAIN);
        match self.encode() {
            Ok(bytes) => h.update(&bytes),
            Err(_) => {
                // Lengths that do not fit the u32 wire fields cannot be
                // deployed; the hash is still total (no panic) and
                // domain-separated from a successful encode.
                h.update(b"\xffencode-overflow");
                h.update((self.code.len() as u64).to_be_bytes());
                h.update((self.rodata.len() as u64).to_be_bytes());
                h.update((self.init.len() as u64).to_be_bytes());
            }
        }
        h.finalize().into()
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
        out.push(self.profile);
        out.push(self.abi);
        out.push(0); // reserved (former fncnf byte)
        out.extend_from_slice(&self.mem_kib.to_be_bytes());
        out.extend_from_slice(&self.entry.to_be_bytes());
        out.extend_from_slice(&code_len.to_be_bytes());
        out.extend_from_slice(&init_len.to_be_bytes());
        out.extend_from_slice(&self.input_max.to_be_bytes());
        out.extend_from_slice(&self.output_max.to_be_bytes());
        out.extend_from_slice(&rodata_len.to_be_bytes());
        out.extend_from_slice(&self.bss_len.to_be_bytes());
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
        let profile = bytes[5];
        if bytes[7] != 0 {
            return Err(err("calc container reserved header byte 7 must be zero"));
        }
        let mem_kib = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let entry = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let code_len = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
        let init_len = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as usize;
        let input_max = u16::from_be_bytes([bytes[24], bytes[25]]);
        let output_max = u16::from_be_bytes([bytes[26], bytes[27]]);
        let rodata_len = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]) as usize;
        let bss_len = u32::from_be_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
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
        let c = CalcContainer {
            profile,
            abi: bytes[6],
            mem_kib,
            entry,
            code: payload[..code_len].to_vec(),
            rodata: payload[code_len..code_len + rodata_len].to_vec(),
            init: payload[code_len + rodata_len..].to_vec(),
            bss_len,
            input_max,
            output_max,
        };
        c.validate_invariants()?;
        Ok((c, total))
    }
}
