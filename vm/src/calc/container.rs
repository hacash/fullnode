//! Calc container v2: 20-byte header + PT_LOAD-like segment table.
//!
//! Header byte 5 is a profile id — three additive bits: bit 0 bitmanip (B),
//! bit 1 RVC, bit 2 hash ecalls. v2 admits every B / RVC combination
//! (profiles 0..=3); the HASH bit is not admitted. The profile byte is
//! itself the zk declaration: profile 0 is the zk-standardized subset
//! (eth-act `riscv64im_zicclsm`); B / C profiles execute on the same
//! engines but are outside that zk subset. Every guest-visible capability
//! a profile needs (decoder whitelist, gas weights, code alignment) is
//! attached to the profile id, so admission only ever grows.
//!
//! Segments are a lossless projection of ELF `PT_LOAD`. `.bss` is
//! `memsz - filesz` (zero-filled at load, not stored). Host IO/stack sit
//! at the top of `mem_kib`; guest segments must stay below that window.

use crate::calc::CALC_MIN_FREE_RW_GAP;
use crate::rt::{ItrErr, ItrErrCode, SpaceCap, VmrtRes};

pub const CALC_MAGIC: [u8; 4] = *b"HCF1";
pub const CALC_VERSION: u8 = 2;

/// ELF `PF_X`.
pub const SEG_X: u8 = 0x1;
/// ELF `PF_W`.
pub const SEG_W: u8 = 0x2;
/// ELF `PF_R`.
pub const SEG_R: u8 = 0x4;

/// Profile byte bit 0: Zba/Zbb/Zbs/Zbc.
pub const CALC_PROFILE_BITMANIP: u8 = 0b001;
/// Profile byte bit 1: RVC (16-bit) encodings.
pub const CALC_PROFILE_COMPRESSED: u8 = 0b010;
/// Profile byte bit 2: hash ecalls (`a7` = 1..=6) are part of this image.
/// Not admitted in v2: the number stays reserved so hash admission is a
/// distinct hard fork with its own pricing activation.
pub const CALC_PROFILE_HASH: u8 = 0b100;

/// Profile 0: RV64IM + Zicclsm. No C, no B, no hash. zkVM baseline
/// (eth-act `riscv64im_zicclsm`). **admitted.**
pub const CALC_PROFILE_RV64IM: u8 = 0;
/// RV64IM + Zba/Zbb/Zbs/Zbc. **admitted.**
pub const CALC_PROFILE_RV64IM_B: u8 = CALC_PROFILE_BITMANIP;
/// RV64IMC. **admitted.**
pub const CALC_PROFILE_RV64IMC: u8 = CALC_PROFILE_COMPRESSED;
/// RV64IMC + bitmanip. **admitted.**
pub const CALC_PROFILE_RV64IMC_B: u8 = CALC_PROFILE_BITMANIP | CALC_PROFILE_COMPRESSED;

/// Profile bits v2 admits. The HASH bit is deliberately outside: a hash
/// activation must bring its own pricing table on a new profile value.
pub const CALC_PROFILE_ADMITTED_MASK: u8 = CALC_PROFILE_BITMANIP | CALC_PROFILE_COMPRESSED;

const PROFILE_KNOWN_MASK: u8 =
    CALC_PROFILE_BITMANIP | CALC_PROFILE_COMPRESSED | CALC_PROFILE_HASH;

/// Decoder whitelist (B / C / HASH encodings exist in the tree). Bits
/// 3..=7 are reserved and disabled per SPEC §2: a future fork defines
/// them first, then grows admission to accept them.
pub const fn profile_known(profile: u8) -> bool {
    profile & !PROFILE_KNOWN_MASK == 0
}

/// Wire admission. Every B / RVC combination is admitted; the HASH bit is
/// rejected. Only profiles whose ISA subset is zk-standardized carry zk
/// portability — that property is per-profile, declared by the byte itself.
pub const fn profile_admitted(profile: u8) -> bool {
    profile & !CALC_PROFILE_ADMITTED_MASK == 0
}

pub const fn profile_has_bitmanip(profile: u8) -> bool {
    profile & CALC_PROFILE_BITMANIP != 0
}

pub const fn profile_has_compressed(profile: u8) -> bool {
    profile & CALC_PROFILE_COMPRESSED != 0
}

pub const fn profile_has_hash(profile: u8) -> bool {
    profile & CALC_PROFILE_HASH != 0
}

/// First mappable guest byte. `[0, HALT_STUB)` is unmapped.
/// `HALT_STUB + HALT_STUB_LEN == CODE_BASE` (no gap).
pub const CODE_BASE: u64 = 0x1000;

/// Injected HALT trampoline. `ra` points here at entry so a standard
/// `extern "C"` return becomes HALT with `a0 = 0`, `a1 = returned length`.
pub const HALT_STUB: u64 = 0x0FF0;
pub const HALT_STUB_LEN: u64 = 16;
pub const HALT_STUB_INSTS: usize = 4;
/// `mv a1,a0; li a0,0; li a7,0; ecall`
pub const HALT_STUB_BYTES: [u8; 16] = [
    0x93, 0x05, 0x05, 0x00, // addi a1, a0, 0
    0x13, 0x05, 0x00, 0x00, // addi a0, x0, 0
    0x93, 0x08, 0x00, 0x00, // addi a7, x0, 0
    0x73, 0x00, 0x00, 0x00, // ecall
];

/// Domain tag for the canonical image ID.
pub const IMAGE_ID_DOMAIN: &[u8] = b"hacash-calc-image-v2";

pub const CALC_MEM_KIB_MIN: u32 = 4;
pub use crate::rt::CALC_MEM_KIB_MAX;
/// Frozen consensus ceiling (SPEC §3): the whole guest memory stays
/// within the machine's signed 32-bit address space,
/// `CODE_BASE + mem_kib * 1024 <= 0x7FFF_FFFF`.
pub const CALC_MEM_KIB_ENGINE_MAX: u32 = (((i32::MAX as u64) - CODE_BASE) / 1024) as u32;

pub const HEADER_LEN: usize = 20;
pub const SEG_HDR_LEN: usize = 16;
pub const MAX_SEGS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcSegment {
    pub vaddr: u32,
    pub memsz: u32,
    pub flags: u8,
    pub data: Vec<u8>,
}

impl CalcSegment {
    pub fn filesz(&self) -> u32 {
        self.data.len() as u32
    }

    pub fn end(&self) -> u64 {
        self.vaddr as u64 + self.memsz as u64
    }

    pub fn rx(vaddr: u32, data: Vec<u8>) -> Self {
        let n = data.len() as u32;
        Self {
            vaddr,
            memsz: n,
            flags: SEG_R | SEG_X,
            data,
        }
    }

    pub fn ro(vaddr: u32, data: Vec<u8>) -> Self {
        let n = data.len() as u32;
        Self {
            vaddr,
            memsz: n,
            flags: SEG_R,
            data,
        }
    }

    pub fn rw(vaddr: u32, data: Vec<u8>, memsz: u32) -> Self {
        Self {
            vaddr,
            memsz,
            flags: SEG_R | SEG_W,
            data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcLayout {
    pub mem_len: u64,
    pub usable_len: u64,
    pub guest_end: u64,
    pub input_base: u64,
    pub input_max: u64,
    pub output_base: u64,
    pub output_max: u64,
    /// First stack byte. There is no guard band: the stack region starts
    /// exactly at `guest_end` and a deep call frame may legally write over
    /// the end of guest data — that is deterministic guest behavior, not a
    /// trap.
    pub stack_bottom: u64,
    pub stack_top: u64,
}

impl CalcLayout {
    pub const fn fits(&self) -> bool {
        let reserved_end = self
            .guest_end
            .saturating_add(crate::calc::CALC_MIN_FREE_RW_GAP);
        reserved_end <= self.stack_top
            && self.stack_top == self.output_base
            && self.input_base >= CODE_BASE
            && self.output_base >= CODE_BASE
            && self.guest_end >= CODE_BASE
            && self.stack_bottom == self.guest_end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcContainer {
    pub profile: u8,
    pub mem_kib: u32,
    pub entry: u32,
    pub segs: Vec<CalcSegment>,
    pub input_max: u16,
    pub output_max: u16,
}

impl CalcContainer {
    /// Single RX segment at [`CODE_BASE`]. Used by tests and hand-assembled kernels.
    pub fn from_code(
        profile: u8,
        mem_kib: u32,
        entry: u32,
        code: Vec<u8>,
        input_max: u16,
        output_max: u16,
    ) -> Self {
        Self {
            profile,
            mem_kib,
            entry,
            segs: vec![CalcSegment::rx(CODE_BASE as u32, code)],
            input_max,
            output_max,
        }
    }

    /// Adjacent RX / R / RW segments starting at [`CODE_BASE`], matching the
    /// historical three-region pack for tests that still pass separate blobs.
    pub fn from_regions(
        profile: u8,
        mem_kib: u32,
        entry: u32,
        code: Vec<u8>,
        rodata: Vec<u8>,
        init: Vec<u8>,
        input_max: u16,
        output_max: u16,
    ) -> Self {
        let mut segs = Vec::new();
        let mut v = CODE_BASE;
        if !code.is_empty() {
            let n = code.len() as u64;
            segs.push(CalcSegment::rx(v as u32, code));
            v += n;
        }
        if !rodata.is_empty() {
            v = (v + 15) & !15;
            let n = rodata.len() as u64;
            segs.push(CalcSegment::ro(v as u32, rodata));
            v += n;
        }
        if !init.is_empty() {
            v = (v + 15) & !15;
            let n = init.len() as u32;
            segs.push(CalcSegment::rw(v as u32, init, n));
        }
        Self {
            profile,
            mem_kib,
            entry,
            segs,
            input_max,
            output_max,
        }
    }

    /// Grow the last writable segment's `memsz` by `bss_len` (zero-filled at
    /// load, not stored). Grows the DECLARED size — it must never shrink a
    /// segment that already reserves BSS (`memsz > filesz`). If there is no
    /// writable segment, append a RW BSS at `guest_end`.
    pub fn with_bss(mut self, bss_len: u32) -> Self {
        if bss_len == 0 {
            return self;
        }
        match self.segs.iter_mut().rev().find(|s| s.flags & SEG_W != 0) {
            Some(s) => {
                s.memsz = s
                    .memsz
                    .checked_add(bss_len)
                    .expect("calc container: with_bss overflows memsz");
            }
            None => self.segs.push(CalcSegment::rw(self.guest_end() as u32, vec![], bss_len)),
        }
        self
    }

    pub fn exec_filesz(&self) -> usize {
        self.segs
            .iter()
            .filter(|s| s.flags & SEG_X != 0)
            .map(|s| s.data.len())
            .sum()
    }

    pub fn ro_filesz(&self) -> usize {
        self.segs
            .iter()
            .filter(|s| s.flags & SEG_X == 0 && s.flags & SEG_W == 0)
            .map(|s| s.data.len())
            .sum()
    }

    pub fn rw_filesz(&self) -> usize {
        self.segs
            .iter()
            .filter(|s| s.flags & SEG_W != 0)
            .map(|s| s.data.len())
            .sum()
    }

    pub fn first_rx_data(&self) -> &[u8] {
        self.segs
            .iter()
            .find(|s| s.flags & SEG_X != 0)
            .map(|s| s.data.as_slice())
            .unwrap_or(&[])
    }

    pub fn guest_end(&self) -> u64 {
        self.segs.iter().map(|s| s.end()).max().unwrap_or(CODE_BASE)
    }

    pub fn load_into(&self, mem: &mut [u8]) {
        let stub = HALT_STUB as usize;
        mem[stub..stub + HALT_STUB_LEN as usize].copy_from_slice(&HALT_STUB_BYTES);
        for s in &self.segs {
            let v = s.vaddr as usize;
            let n = s.data.len();
            mem[v..v + n].copy_from_slice(&s.data);
        }
    }

    pub fn can_read(&self, lay: &CalcLayout, addr: u64, end: u64) -> bool {
        perm_check(lay, &self.seg_recs(), addr, end, false)
    }

    pub fn can_write(&self, lay: &CalcLayout, addr: u64, end: u64) -> bool {
        perm_check(lay, &self.seg_recs(), addr, end, true)
    }

    pub fn seg_recs(&self) -> Vec<(u32, u32, u8)> {
        self.segs.iter().map(|s| (s.vaddr, s.memsz, s.flags)).collect()
    }
}

pub(crate) fn compute_layout(
    guest_end: u64,
    input_max: u64,
    output_max: u64,
    usable_len: u64,
) -> CalcLayout {
    let mem_len = CODE_BASE.saturating_add(usable_len);
    let input_base = mem_len.saturating_sub(input_max) & !15;
    let output_base = input_base.saturating_sub(output_max) & !15;
    let stack_top = output_base;
    let stack_bottom = guest_end;
    CalcLayout {
        mem_len,
        usable_len,
        guest_end,
        input_base,
        input_max,
        output_base,
        output_max,
        stack_bottom,
        stack_top,
    }
}

pub fn min_mem_kib_for(guest_end: u64, input_max: u64, output_max: u64) -> Option<u32> {
    let need_top = guest_end
        .checked_add(CALC_MIN_FREE_RW_GAP)?
        .checked_add(15)?
        & !15;
    let input_base = need_top.checked_add(output_max)?.checked_add(15)? & !15;
    let mem_len = input_base.checked_add(input_max)?;
    let mut kib = mem_len
        .saturating_sub(CODE_BASE)
        .saturating_add(1023)
        / 1024;
    if kib < CALC_MEM_KIB_MIN as u64 {
        kib = CALC_MEM_KIB_MIN as u64;
    }
    while kib <= CALC_MEM_KIB_ENGINE_MAX as u64 {
        let lay = compute_layout(guest_end, input_max, output_max, kib * 1024);
        if lay.fits() {
            return Some(kib as u32);
        }
        kib += 1;
    }
    None
}

fn err(t: &str) -> ItrErr {
    ItrErr::new(ItrErrCode::ContractError, t)
}

pub(crate) fn perm_check(
    lay: &CalcLayout,
    segs: &[(u32, u32, u8)],
    addr: u64,
    end: u64,
    write: bool,
) -> bool {
    if end < addr {
        return false;
    }
    if addr == end {
        return true;
    }
    let mut p = addr;
    while p < end {
        let Some((region_end, r, w)) = perm_at(lay, segs, p) else {
            return false;
        };
        if write {
            if !w {
                return false;
            }
        } else if !r {
            return false;
        }
        let next = region_end.min(end);
        if next <= p {
            return false;
        }
        p = next;
    }
    true
}

/// Homogeneous permission region containing `addr`: `(exclusive_end, read, write)`.
fn perm_at(lay: &CalcLayout, segs: &[(u32, u32, u8)], addr: u64) -> Option<(u64, bool, bool)> {
    if addr >= lay.mem_len {
        return None;
    }
    if addr < HALT_STUB {
        return None;
    }
    if addr < HALT_STUB + HALT_STUB_LEN {
        return Some((HALT_STUB + HALT_STUB_LEN, true, false));
    }
    if addr < CODE_BASE {
        return None;
    }
    for &(vaddr, memsz, flags) in segs {
        let start = vaddr as u64;
        let send = start + memsz as u64;
        if addr >= start && addr < send {
            let r = flags & SEG_R != 0;
            let w = flags & SEG_W != 0;
            return Some((send, r, w));
        }
    }
    if addr < lay.stack_bottom {
        return None;
    }
    // [stack_bottom, mem_len): the RW stack + host IO window. There is no
    // guard band between the guest image and the stack.
    Some((lay.mem_len, true, true))
}

impl CalcContainer {
    fn validate_invariants(&self) -> VmrtRes<()> {
        if !profile_known(self.profile) {
            return Err(err("calc container unknown isa profile"));
        }
        if !profile_admitted(self.profile) {
            return Err(err(
                "calc container isa profile not admitted (hash is not in v2)",
            ));
        }
        if self.mem_kib < CALC_MEM_KIB_MIN || self.mem_kib > CALC_MEM_KIB_ENGINE_MAX {
            return Err(err("calc container mem_kib out of range"));
        }
        if self.output_max == 0 {
            return Err(err("calc container io size invalid"));
        }
        if self.segs.is_empty() || self.segs.len() > MAX_SEGS {
            return Err(err("calc container segment count invalid"));
        }
        let compressed = profile_has_compressed(self.profile);
        let code_align = if compressed { 2u64 } else { 4 };
        let mut prev_end = CODE_BASE;
        let mut has_x = false;
        let mut entry_in_x = false;
        for s in &self.segs {
            if s.flags & !(SEG_R | SEG_W | SEG_X) != 0 {
                return Err(err("calc container segment flags invalid"));
            }
            if s.flags & SEG_R == 0 {
                return Err(err("calc container segment is not readable"));
            }
            if s.flags & SEG_W != 0 && s.flags & SEG_X != 0 {
                return Err(err("calc container W^X violated"));
            }
            if (s.data.len() as u32) > s.memsz {
                return Err(err("calc container filesz exceeds memsz"));
            }
            if s.memsz == 0 {
                return Err(err("calc container empty segment"));
            }
            let start = s.vaddr as u64;
            let send = match start.checked_add(s.memsz as u64) {
                Some(e) => e,
                None => return Err(err("calc container segment overflow")),
            };
            if start < CODE_BASE {
                return Err(err("calc container segment below CODE_BASE"));
            }
            if start < prev_end {
                return Err(err("calc container segments overlap or unsorted"));
            }
            if s.flags & SEG_X != 0 {
                has_x = true;
                if s.filesz() != s.memsz {
                    return Err(err("calc container executable bss not allowed"));
                }
                if s.data.len() as u64 % code_align != 0 {
                    return Err(err("calc container code_len invalid"));
                }
                // IALIGN: the segment BASE must satisfy the profile's fetch
                // alignment, not just its length — `push_stream` decodes by
                // segment-relative offset, so an unaligned vaddr would accept
                // instruction streams a spec-compliant RISC-V (and any zkVM)
                // faults on at fetch time.
                if start % code_align != 0 {
                    return Err(err(if compressed {
                        "calc container code vaddr not 2-byte aligned"
                    } else {
                        "calc container code vaddr not 4-byte aligned"
                    }));
                }
                let entry_u = self.entry as u64;
                if entry_u >= start && entry_u < send {
                    entry_in_x = true;
                }
            }
            prev_end = send;
        }
        if !has_x {
            return Err(err("calc container has no executable segment"));
        }
        let lay = self.layout();
        if !lay.fits() {
            return Err(err("calc container memory overflows (io overlaps data)"));
        }
        if prev_end > lay.guest_end {
            return Err(err("calc container segment overlaps host region"));
        }
        let entry_u = self.entry as u64;
        if !entry_in_x {
            return Err(err("calc container entry outside code"));
        }
        if entry_u % code_align != 0 {
            return Err(err(if compressed {
                "calc container entry not 2-byte aligned"
            } else {
                "calc container entry not 4-byte aligned"
            }));
        }
        Ok(())
    }

    pub(crate) fn validate_header(&self, cap: &SpaceCap) -> VmrtRes<()> {
        self.validate_invariants()?;
        if self.mem_kib > cap.calc_mem_kib_max {
            return Err(ItrErr::new(
                ItrErrCode::ContractError,
                "calc verify: mem_kib exceeds chain cap",
            ));
        }
        Ok(())
    }

    pub fn layout(&self) -> CalcLayout {
        compute_layout(
            self.guest_end(),
            self.input_max as u64,
            self.output_max as u64,
            self.mem_kib as u64 * 1024,
        )
    }

    pub fn image_id(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(IMAGE_ID_DOMAIN);
        match self.encode() {
            Ok(bytes) => h.update(&bytes),
            Err(_) => h.update(b"\xffencode-overflow"),
        }
        h.finalize().into()
    }

    pub fn encode(&self) -> VmrtRes<Vec<u8>> {
        let nseg = u8::try_from(self.segs.len())
            .map_err(|_| err("calc container segment count exceeds u8"))?;
        let mut payload = 0usize;
        for s in &self.segs {
            payload = payload
                .checked_add(s.data.len())
                .ok_or_else(|| err("calc container length overflow"))?;
        }
        let table = (self.segs.len())
            .checked_mul(SEG_HDR_LEN)
            .ok_or_else(|| err("calc container length overflow"))?;
        let capacity = HEADER_LEN
            .checked_add(table)
            .and_then(|n| n.checked_add(payload))
            .ok_or_else(|| err("calc container length overflow"))?;
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&CALC_MAGIC);
        out.push(CALC_VERSION);
        out.push(self.profile);
        out.push(nseg);
        out.push(0);
        out.extend_from_slice(&self.mem_kib.to_be_bytes());
        out.extend_from_slice(&self.entry.to_be_bytes());
        out.extend_from_slice(&self.input_max.to_be_bytes());
        out.extend_from_slice(&self.output_max.to_be_bytes());
        for s in &self.segs {
            let filesz = s.filesz();
            out.extend_from_slice(&s.vaddr.to_be_bytes());
            out.extend_from_slice(&filesz.to_be_bytes());
            out.extend_from_slice(&s.memsz.to_be_bytes());
            out.push(s.flags);
            out.extend_from_slice(&[0, 0, 0]);
        }
        for s in &self.segs {
            out.extend_from_slice(&s.data);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), ItrErr> {
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
        let nseg = bytes[6] as usize;
        if bytes[7] != 0 {
            return Err(err("calc container reserved header bytes must be zero"));
        }
        if nseg == 0 || nseg > MAX_SEGS {
            return Err(err("calc container segment count invalid"));
        }
        let mem_kib = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let entry = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let input_max = u16::from_be_bytes([bytes[16], bytes[17]]);
        let output_max = u16::from_be_bytes([bytes[18], bytes[19]]);
        let table = nseg
            .checked_mul(SEG_HDR_LEN)
            .ok_or_else(|| err("calc container length overflow"))?;
        let table_end = HEADER_LEN
            .checked_add(table)
            .ok_or_else(|| err("calc container length overflow"))?;
        if bytes.len() < table_end {
            return Err(err("calc container truncated payload"));
        }
        let mut segs = Vec::with_capacity(nseg);
        let mut data_off = table_end;
        for i in 0..nseg {
            let h = &bytes[HEADER_LEN + i * SEG_HDR_LEN..HEADER_LEN + (i + 1) * SEG_HDR_LEN];
            let vaddr = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
            let filesz = u32::from_be_bytes([h[4], h[5], h[6], h[7]]) as usize;
            let memsz = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
            let flags = h[12];
            if h[13] != 0 || h[14] != 0 || h[15] != 0 {
                return Err(err("calc container segment padding must be zero"));
            }
            let next = data_off
                .checked_add(filesz)
                .ok_or_else(|| err("calc container length overflow"))?;
            if bytes.len() < next {
                return Err(err("calc container truncated payload"));
            }
            segs.push(CalcSegment {
                vaddr,
                memsz,
                flags,
                data: bytes[data_off..next].to_vec(),
            });
            data_off = next;
        }
        if bytes.len() != data_off {
            return Err(err("calc container trailing bytes"));
        }
        let c = CalcContainer {
            profile,
            mem_kib,
            entry,
            segs,
            input_max,
            output_max,
        };
        c.validate_invariants()?;
        Ok((c, data_off))
    }
}
