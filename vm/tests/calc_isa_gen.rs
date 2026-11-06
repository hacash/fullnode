//! Shared ISA-level test-case generator for the calc VM: the RV64IM base
//! (profile 0), the Zba/Zbb/Zbs/Zbc bitmanip forms (profile B) and the
//! admitted RVC forms (profile C), every case with HOST-side reference
//! semantics.
//!
//! Emits one self-contained program per instruction form: a prologue that
//! materializes eight boundary constants into x5..x12, the instruction(s)
//! under test, and `sd` records of every observable result into the stack
//! slots `[sp-8], [sp-16], ...`. Expected words are computed HOST-side in
//! plain Rust (written per the RISC-V unprivileged spec, independent of
//! the interpreter code paths), so the suite pins decode + execute +
//! memory end to end.
//!
//! Consumers:
//! - `vm/tests/calc_isa_suite.rs` (committed): interpreter vs host
//!   reference, plus AOT whole-state parity under `--features calc-aot`.
//! - `vm/tests.loc/calc_isa_ckbvm.rs` (business-local): the same cases
//!   against ckb-vm 0.24.13 (whole machine) and softcore-rv64.
//!
//! Encodings: 32-bit forms per the spec tables. Branch/jal
//! immediates are patched by the generator - no hand-computed offsets.
//!
//! Register conventions: x5..x12 boundary constants, x27 = sp (record
//! base), x28 = instruction-under-test destination, x29 = li64 scratch,
//! x30/x31 free temporaries. Only dedicated stack cases touch x1/x2.

#![allow(dead_code)]

use vm::calc::container::{
    CalcContainer, CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC, CALC_PROFILE_RV64IMC_B,
    CALC_PROFILE_RV64IM_B, CODE_BASE,
};

pub const MEM_KIB: u32 = 16;
pub const INPUT_MAX: u16 = 512;
pub const OUTPUT_MAX: u16 = 512;

pub fn mem_len() -> u64 {
    CODE_BASE + MEM_KIB as u64 * 1024
}

/// Eight boundary constants prologue-loaded into x5..x12.
pub const VALS: [u64; 8] = [
    0,
    1,
    u64::MAX,              // -1
    i64::MIN as u64,       // 0x8000_0000_0000_0000
    i64::MAX as u64,       // 0x7FFF_FFFF_FFFF_FFFF
    0x7F,                  // sign-boundary byte
    u32::MAX as u64,       // 0xFFFF_FFFF (W-form boundary)
    0x0123_4567_89AB_CDEF, // random
];

/// Byte pattern for load tests (high bits set so sign extension shows).
pub const PATTERN: [u8; 16] = [
    0x80, 0x01, 0x7F, 0xFF, 0x00, 0x81, 0xFE, 0x7F, 0x80, 0x7F, 0x01, 0x02, 0xFF, 0x80, 0x40, 0xC0,
];

// ================================================================ assembler

pub struct Asm {
    pub bytes: Vec<u8>,
    /// Byte address of every instruction (2 or 4 bytes).
    pub word_addr: Vec<u32>,
    b_fixups: Vec<(usize, u32, u8, u8, usize)>, // idx, f3, rs1, rs2, target
    j_fixups: Vec<(usize, u8, usize)>,          // idx, rd, target
}

impl Asm {
    pub fn new() -> Self {
        Asm {
            bytes: Vec::new(),
            word_addr: Vec::new(),
            b_fixups: Vec::new(),
            j_fixups: Vec::new(),
        }
    }

    pub fn raw(&mut self, w: u32) -> usize {
        let idx = self.word_addr.len();
        self.word_addr.push(self.bytes.len() as u32);
        self.bytes.extend_from_slice(&w.to_le_bytes());
        idx
    }

    /// Compressed halfword (RVC profiles only); recorded like any
    /// instruction so branch/jal fixups still resolve by byte offset.
    pub fn half(&mut self, h: u16) -> usize {
        let idx = self.word_addr.len();
        self.word_addr.push(self.bytes.len() as u32);
        self.bytes.extend_from_slice(&h.to_le_bytes());
        idx
    }

    /// R-type.
    pub fn r(&mut self, f7: u32, f3: u32, rd: u8, rs1: u8, rs2: u8, op: u32) -> usize {
        self.raw(
            (f7 << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((rd as u32) << 7)
                | op,
        )
    }

    /// I-type (loads, jalr, op-imm); 12-bit signed immediate.
    pub fn i(&mut self, f3: u32, rd: u8, rs1: u8, op: u32, imm: i32) -> usize {
        assert!((-2048..=2047).contains(&imm), "i-imm {imm} out of range");
        self.raw(
            (((imm as u32) & 0xFFF) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((rd as u32) << 7)
                | op,
        )
    }

    /// Shift-immediate, 6-bit shamt field form (f6 = bits 31:26).
    pub fn sh(&mut self, f6: u32, f3: u32, rd: u8, rs1: u8, shamt: u8, op: u32) -> usize {
        self.raw(
            (f6 << 26)
                | ((shamt as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((rd as u32) << 7)
                | op,
        )
    }

    /// Shift-immediate, 5-bit shamt / funct7 form (W shifts).
    pub fn sh_w(&mut self, f7: u32, f3: u32, rd: u8, rs1: u8, shamt: u8, op: u32) -> usize {
        self.raw(
            (f7 << 25)
                | ((shamt as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((rd as u32) << 7)
                | op,
        )
    }

    /// S-type store; 12-bit signed immediate.
    pub fn s(&mut self, f3: u32, rs1: u8, rs2: u8, imm: i32) -> usize {
        assert!((-2048..=2047).contains(&imm), "s-imm {imm} out of range");
        let imm = imm as u32;
        self.raw(
            (((imm >> 5) & 0x7F) << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((imm & 0x1F) << 7)
                | 0x23,
        )
    }

    /// B-type branch; target is an instruction index patched in `finish`.
    pub fn b(&mut self, f3: u32, rs1: u8, rs2: u8, target: usize) -> usize {
        let idx = self.raw(0);
        self.b_fixups.push((idx, f3, rs1, rs2, target));
        idx
    }

    /// J-type jal; target is an instruction index patched in `finish`.
    pub fn j(&mut self, rd: u8, target: usize) -> usize {
        let idx = self.raw(0);
        self.j_fixups.push((idx, rd, target));
        idx
    }

    /// U-type lui (0x37) / auipc (0x17); imm20 is the raw field.
    pub fn u(&mut self, rd: u8, op: u32, imm20: u32) -> usize {
        assert!(imm20 <= 0xFFFFF);
        self.raw((imm20 << 12) | ((rd as u32) << 7) | op)
    }

    pub fn ecall(&mut self) -> usize {
        self.raw(0x0000_0073)
    }

    // ---------------- compound helpers ----------------

    /// Load an arbitrary u64 into `rd` (scratch `tmp`, tmp != rd).
    /// Exact: (sext32(hi) << 32) + zext32(lo) == v (mod 2^64) — the hi
    /// sign error is a multiple of 2^64 after the shift, the lo part is
    /// zero-extended through a slli/srli pair.
    pub fn li64(&mut self, rd: u8, v: u64, tmp: u8) {
        if v == 0 {
            self.i(0, rd, 0, 0x13, 0);
            return;
        }
        let hi = (v >> 32) as u32;
        let lo = v as u32;
        let load32 = |a: &mut Asm, reg: u8, x: u32| {
            let h = x.wrapping_add(0x800) >> 12;
            let l = x.wrapping_sub(h << 12);
            a.u(reg, 0x37, h);
            if l != 0 || h == 0 {
                a.i(0, reg, reg, 0x13, l as i32);
            }
        };
        load32(self, rd, hi); // rd = sext32(hi)
        self.sh(0, 1, rd, rd, 32, 0x13); // slli rd, rd, 32
        load32(self, tmp, lo); // tmp = sext32(lo)
        self.sh(0, 1, tmp, tmp, 32, 0x13); // slli tmp, tmp, 32
        self.sh(0, 5, tmp, tmp, 32, 0x13); // srli tmp, tmp, 32 -> zext32(lo)
        self.r(0, 0, rd, rd, tmp, 0x33); // add rd, rd, tmp
    }

    /// HALT ecall with exit 0 and empty output.
    pub fn ret(&mut self) {
        self.i(0, 10, 0, 0x13, 0);
        self.i(0, 11, 0, 0x13, 0);
        self.i(0, 17, 0, 0x13, 0);
        self.ecall();
    }

    /// Record one register word at slot k (1-based): `sd reg, -8k(x27)`.
    /// x27 must already hold sp.
    pub fn record(&mut self, reg: u8, k: usize) {
        self.s(3, 27, reg, -(8 * k as i32));
    }

    /// Standard prologue: VALS into x5..x12, x27 = sp. Returns the
    /// instruction count it emitted (stable across instances).
    pub fn prologue(&mut self) -> usize {
        let start = self.word_addr.len();
        for (i, v) in VALS.iter().enumerate() {
            self.li64(5 + i as u8, *v, 29);
        }
        self.i(0, 27, 2, 0x13, 0); // addi x27, x2, 0
        self.word_addr.len() - start
    }

    pub fn finish(mut self) -> Vec<u8> {
        for (idx, f3, rs1, rs2, target) in std::mem::take(&mut self.b_fixups) {
            let rel = (self.word_addr[target] as i64 - self.word_addr[idx] as i64) as i32;
            let imm = rel as u32;
            assert_eq!(imm & 1, 0, "branch offset must be even");
            let w = ((imm >> 12 & 1) << 31)
                | ((imm >> 5 & 0x3F) << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((imm >> 1 & 0xF) << 8)
                | ((imm >> 11 & 1) << 7)
                | 0x63;
            let at = self.word_addr[idx] as usize;
            self.bytes[at..at + 4].copy_from_slice(&w.to_le_bytes());
        }
        for (idx, rd, target) in std::mem::take(&mut self.j_fixups) {
            let rel = (self.word_addr[target] as i64 - self.word_addr[idx] as i64) as i32;
            let imm = rel as u32;
            assert_eq!(imm & 1, 0, "jal offset must be even");
            let w = ((imm >> 20 & 1) << 31)
                | ((imm >> 1 & 0x3FF) << 21)
                | ((imm >> 11 & 1) << 20)
                | ((imm >> 12 & 0xFF) << 12)
                | ((rd as u32) << 7)
                | 0x6F;
            let at = self.word_addr[idx] as usize;
            self.bytes[at..at + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.bytes
    }
}

// ============================================================= host refs

fn sext32(v: u64) -> u64 {
    (v as u32 as i32) as i64 as u64
}

/// Host-side reference semantics for every register-register form
/// (independent of the interpreter implementation).
pub fn ref_reg(op: &str, a: u64, b: u64) -> u64 {
    match op {
        "add" => a.wrapping_add(b),
        "sub" => a.wrapping_sub(b),
        "slt" => ((a as i64) < (b as i64)) as u64,
        "sltu" => (a < b) as u64,
        "xor" => a ^ b,
        "or" => a | b,
        "and" => a & b,
        "sll" => a.wrapping_shl((b & 63) as u32),
        "srl" => a.wrapping_shr((b & 63) as u32),
        "sra" => ((a as i64).wrapping_shr((b & 63) as u32)) as u64,
        "mul" => a.wrapping_mul(b),
        "mulh" => (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u64,
        "mulhu" => (((a as u128) * (b as u128)) >> 64) as u64,
        "mulhsu" => (((a as i64 as i128) * (b as i128)) >> 64) as u64,
        "div" => {
            if b == 0 {
                u64::MAX
            } else if a == i64::MIN as u64 && b == u64::MAX {
                a
            } else {
                ((a as i64) / (b as i64)) as u64
            }
        }
        "divu" => {
            if b == 0 {
                u64::MAX
            } else {
                a / b
            }
        }
        "rem" => {
            if b == 0 {
                a
            } else if a == i64::MIN as u64 && b == u64::MAX {
                0
            } else {
                ((a as i64) % (b as i64)) as u64
            }
        }
        "remu" => {
            if b == 0 {
                a
            } else {
                a % b
            }
        }
        "addw" => sext32(a.wrapping_add(b)),
        "subw" => sext32(a.wrapping_sub(b)),
        "sllw" => sext32(a.wrapping_shl((b & 31) as u32)),
        "srlw" => sext32((a as u32).wrapping_shr((b & 31) as u32) as u64),
        "sraw" => sext32((a as i32).wrapping_shr((b & 31) as u32) as u64),
        "mulw" => sext32(a.wrapping_mul(b)),
        "divw" => {
            if (b as u32) == 0 {
                u64::MAX
            } else {
                sext32((a as u32 as i32).wrapping_div(b as u32 as i32) as u64)
            }
        }
        "divuw" => {
            if (b as u32) == 0 {
                u64::MAX
            } else {
                sext32((a as u32 / b as u32) as u64)
            }
        }
        "remw" => {
            if (b as u32) == 0 {
                sext32(a)
            } else {
                sext32((a as u32 as i32).wrapping_rem(b as u32 as i32) as u64)
            }
        }
        "remuw" => {
            if (b as u32) == 0 {
                sext32(a)
            } else {
                sext32((a as u32 % b as u32) as u64)
            }
        }
        // Zbb register forms
        "adduw" => (a as u32 as u64).wrapping_add(b),
        "sh1add" => (a << 1).wrapping_add(b),
        "sh2add" => (a << 2).wrapping_add(b),
        "sh3add" => (a << 3).wrapping_add(b),
        "sh1adduw" => ((a as u32 as u64) << 1).wrapping_add(b),
        "sh2adduw" => ((a as u32 as u64) << 2).wrapping_add(b),
        "sh3adduw" => ((a as u32 as u64) << 3).wrapping_add(b),
        "andn" => a & !b,
        "orn" => a | !b,
        "xnor" => !(a ^ b),
        "rol" => a.rotate_left((b & 63) as u32),
        "ror" => a.rotate_right((b & 63) as u32),
        "rolw" => sext32((a as u32).rotate_left((b & 31) as u32) as u64),
        "rorw" => sext32((a as u32).rotate_right((b & 31) as u32) as u64),
        "min" => (a as i64).min(b as i64) as u64,
        "minu" => a.min(b),
        "max" => (a as i64).max(b as i64) as u64,
        "maxu" => a.max(b),
        "bset" => a | (1u64 << (b & 63)),
        "bclr" => a & !(1u64 << (b & 63)),
        "binv" => a ^ (1u64 << (b & 63)),
        "bext" => (a >> (b & 63)) & 1,
        // Zbc
        "clmul" => clmul128(a, b) as u64,
        "clmulh" => (clmul128(a, b) >> 64) as u64,
        "clmulr" => (clmul128(a, b) >> 63) as u64,
        _ => unreachable!("ref_reg {op}"),
    }
}

fn clmul128(a: u64, b: u64) -> u128 {
    let mut p: u128 = 0;
    for i in 0..64 {
        if (b >> i) & 1 == 1 {
            p ^= (a as u128) << i;
        }
    }
    p
}

/// Host-side reference for the Zbb unary forms.
pub fn ref_unary(op: &str, a: u64) -> u64 {
    match op {
        "clz" => a.leading_zeros() as u64,
        "ctz" => a.trailing_zeros() as u64,
        "cpop" => a.count_ones() as u64,
        "sextb" => a as i8 as i64 as u64,
        "sexth" => a as i16 as i64 as u64,
        "zexth" => a & 0xFFFF,
        "orcb" => {
            let mut v = 0u64;
            for i in 0..8 {
                if (a >> (i * 8)) & 0xFF != 0 {
                    v |= 0xFFu64 << (i * 8);
                }
            }
            v
        }
        "rev8" => a.swap_bytes(),
        "clzw" => (a as u32).leading_zeros() as u64,
        "ctzw" => (a as u32).trailing_zeros() as u64,
        "cpopw" => (a as u32).count_ones() as u64,
        _ => unreachable!("ref_unary {op}"),
    }
}

// ================================================================= cases

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Clean sentinel return with empty output.
    Ok,
    /// Deterministic kernel fault.
    Trap,
    /// Budget exhausted; the interpreter must report exactly `used`
    /// cycles (the failing charge excluded).
    OutOfGas(u64),
}

pub struct Case {
    pub name: String,
    pub code: Vec<u8>,
    /// Expected words recorded at [sp-8], [sp-16], ... in order.
    pub expect: Vec<u64>,
    pub outcome: Outcome,
    /// Exact interpreter cycle total when known (None = don't check).
    pub cycles: Option<u64>,
        /// Legacy: AOT now implements ecall. Kept so existing case
        /// constructors compile; always false.
        pub aot_declines: bool,
    /// The case may leave x2 (sp) modified at the stop point.
    pub sp_free: bool,
    /// The case may leave x1 (ra) modified at the stop point.
    pub ra_free: bool,
    /// ISA profile the case runs under (base / B / C / C+B).
    pub profile: u8,
}

impl Case {
    fn ok(name: &str, code: Vec<u8>, expect: Vec<u64>) -> Self {
        Case {
            name: name.to_string(),
            code,
            expect,
            outcome: Outcome::Ok,
            cycles: None,
            aot_declines: false,
            sp_free: false,
            ra_free: false,
            profile: CALC_PROFILE_RV64IM,
        }
    }
    pub fn trap(name: &str, code: Vec<u8>) -> Self {
        Case {
            name: name.to_string(),
            code,
            expect: Vec::new(),
            outcome: Outcome::Trap,
            cycles: None,
            aot_declines: false,
            sp_free: false,
            ra_free: false,
            profile: CALC_PROFILE_RV64IM,
        }
    }
    /// Run the case under a non-base profile (B / C / C+B).
    pub fn with_profile(mut self, profile: u8) -> Self {
        self.profile = profile;
        self
    }
    /// The case may clobber x1 (ra), e.g. via c.jalr.
    pub fn with_ra_free(mut self) -> Self {
        self.ra_free = true;
        self
    }
    pub fn container(&self) -> CalcContainer {
        container_for_profile(self.code.clone(), self.profile)
    }
}

pub fn container_for_profile(code: Vec<u8>, profile: u8) -> CalcContainer {
    CalcContainer::from_code(
            profile,
            MEM_KIB,
            CODE_BASE as u32,
            code,
            INPUT_MAX,
            OUTPUT_MAX,
        )
}

pub fn container_for(code: Vec<u8>) -> CalcContainer {
    container_for_profile(code, CALC_PROFILE_RV64IM)
}

/// Read the recorded words from a final memory image (slots 1..=n,
/// i.e. [sp-8], [sp-16], ... in that order). `stack_top` is the entry `sp`.
pub fn record_words(mem: &[u8], stack_top: u64) -> Vec<u64> {
    let top = stack_top as usize;
    let mut out = Vec::new();
    let mut k = 1usize;
    while top >= 8 * k && out.len() < 4096 {
        let at = top - 8 * k;
        out.push(u64::from_le_bytes(mem[at..at + 8].try_into().unwrap()));
        k += 1;
    }
    out
}

// opcode + (funct7, funct3) tables, same tuples as the decoder.
const OP: u32 = 0b0110011;
const OP32: u32 = 0b0111011;
const OPI: u32 = 0b0010011;
const OPI32: u32 = 0b0011011;

fn reg_entry(op: &str) -> (u32, u32, u32) {
    match op {
        "add" => (0b0000000, 0, OP),
        "sub" => (0b0100000, 0, OP),
        "sll" => (0b0000000, 1, OP),
        "slt" => (0b0000000, 2, OP),
        "sltu" => (0b0000000, 3, OP),
        "xor" => (0b0000000, 4, OP),
        "srl" => (0b0000000, 5, OP),
        "sra" => (0b0100000, 5, OP),
        "or" => (0b0000000, 6, OP),
        "and" => (0b0000000, 7, OP),
        "mul" => (0b0000001, 0, OP),
        "mulh" => (0b0000001, 1, OP),
        "mulhsu" => (0b0000001, 2, OP),
        "mulhu" => (0b0000001, 3, OP),
        "div" => (0b0000001, 4, OP),
        "divu" => (0b0000001, 5, OP),
        "rem" => (0b0000001, 6, OP),
        "remu" => (0b0000001, 7, OP),
        "addw" => (0b0000000, 0, OP32),
        "subw" => (0b0100000, 0, OP32),
        "sllw" => (0b0000000, 1, OP32),
        "srlw" => (0b0000000, 5, OP32),
        "sraw" => (0b0100000, 5, OP32),
        "mulw" => (0b0000001, 0, OP32),
        "divw" => (0b0000001, 4, OP32),
        "divuw" => (0b0000001, 5, OP32),
        "remw" => (0b0000001, 6, OP32),
        "remuw" => (0b0000001, 7, OP32),
        "adduw" => (0b0000100, 0, OP32),
        "sh1add" => (0b0010000, 2, OP),
        "sh2add" => (0b0010000, 4, OP),
        "sh3add" => (0b0010000, 6, OP),
        "sh1adduw" => (0b0010000, 2, OP32),
        "sh2adduw" => (0b0010000, 4, OP32),
        "sh3adduw" => (0b0010000, 6, OP32),
        "andn" => (0b0100000, 7, OP),
        "orn" => (0b0100000, 6, OP),
        "xnor" => (0b0100000, 4, OP),
        "rol" => (0b0110000, 1, OP),
        "ror" => (0b0110000, 5, OP),
        "rolw" => (0b0110000, 1, OP32),
        "rorw" => (0b0110000, 5, OP32),
        "min" => (0b0000101, 4, OP),
        "minu" => (0b0000101, 5, OP),
        "max" => (0b0000101, 6, OP),
        "maxu" => (0b0000101, 7, OP),
        "bset" => (0b0010100, 1, OP),
        "bclr" => (0b0100100, 1, OP),
        "binv" => (0b0110100, 1, OP),
        "bext" => (0b0100100, 5, OP),
        "clmul" => (0b0000101, 1, OP),
        "clmulr" => (0b0000101, 2, OP),
        "clmulh" => (0b0000101, 3, OP),
        _ => unreachable!("reg_entry {op}"),
    }
}

/// One case per register-register op: the full 8x8 VALS matrix (64
/// records: (0,0), (0,1)... covering 0, 1, -1, MIN, MAX, 0x7F,
/// 0xFFFFFFFF, random against each other).
fn reg_matrix_cases(ops: &[&str], profile: u8) -> Vec<Case> {
    ops.iter()
        .map(|op| {
            let mut a = Asm::new();
            a.prologue();
            let mut expect = Vec::new();
            let mut k = 0;
            let (f7, f3, oc) = reg_entry(op);
            for i in 0..8u8 {
                for j in 0..8u8 {
                    a.r(f7, f3, 28, 5 + i, 5 + j, oc);
                    k += 1;
                    a.record(28, k);
                    expect.push(ref_reg(op, VALS[i as usize], VALS[j as usize]));
                }
            }
            a.ret();
            Case::ok(&format!("reg/{op}"), a.finish(), expect).with_profile(profile)
        })
        .collect()
}

fn imm_cases() -> Vec<Case> {
    const IMMS: [i32; 7] = [-2048, -1, 0, 1, 1023, 2047, -1025];
    let forms: &[(&str, u32, u32)] = &[
        ("addi", 0, OPI),
        ("slti", 2, OPI),
        ("sltiu", 3, OPI),
        ("xori", 4, OPI),
        ("ori", 6, OPI),
        ("andi", 7, OPI),
        ("addiw", 0, OPI32),
    ];
    forms
        .iter()
        .map(|(name, f3, oc)| {
            let mut a = Asm::new();
            a.prologue();
            let mut expect = Vec::new();
            let mut k = 0;
            for imm in IMMS {
                for i in 0..8u8 {
                    a.i(*f3, 28, 5 + i, *oc, imm);
                    k += 1;
                    a.record(28, k);
                    let v = VALS[i as usize];
                    expect.push(match *name {
                        "addi" => v.wrapping_add(imm as i64 as u64),
                        "slti" => ((v as i64) < imm as i64) as u64,
                        "sltiu" => (v < imm as i64 as u64) as u64,
                        "xori" => v ^ (imm as i64 as u64),
                        "ori" => v | (imm as i64 as u64),
                        "andi" => v & (imm as i64 as u64),
                        "addiw" => sext32(v.wrapping_add(imm as i64 as u64)),
                        _ => unreachable!(),
                    });
                }
            }
            a.ret();
            Case::ok(&format!("imm/{name}"), a.finish(), expect)
        })
        .collect()
}

fn shift_imm_cases() -> Vec<Case> {
    let mut out = Vec::new();
    // 64-bit shifts: funct6 form. (name, f6, f3)
    let wide: &[(&str, u32, u32)] = &[
        ("slli", 0b000000, 1),
        ("srli", 0b000000, 5),
        ("srai", 0b010000, 5),
    ];
    for (name, f6, f3) in wide {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for &sh in &[0u8, 1, 15, 31, 32, 48, 63] {
            for (i, &v) in VALS.iter().enumerate() {
                a.sh(*f6, *f3, 28, 5 + i as u8, sh, OPI);
                k += 1;
                a.record(28, k);
                expect.push(match *name {
                    "slli" => v.wrapping_shl(sh as u32),
                    "srli" => v.wrapping_shr(sh as u32),
                    "srai" => ((v as i64).wrapping_shr(sh as u32)) as u64,
                    _ => unreachable!(),
                });
            }
        }
        a.ret();
        out.push(Case::ok(&format!("shift/{name}"), a.finish(), expect));
    }
    // 32-bit W shifts: funct7 form, 5-bit shamt. (name, f7, f3)
    let words: &[(&str, u32, u32)] = &[
        ("slliw", 0b0000000, 1),
        ("srliw", 0b0000000, 5),
        ("sraiw", 0b0100000, 5),
    ];
    for (name, f7, f3) in words {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for &sh in &[0u8, 1, 15, 31] {
            for (i, &v) in VALS.iter().enumerate() {
                a.sh_w(*f7, *f3, 28, 5 + i as u8, sh, OPI32);
                k += 1;
                a.record(28, k);
                expect.push(match *name {
                    "slliw" => sext32(v.wrapping_shl(sh as u32)),
                    "srliw" => sext32((v as u32).wrapping_shr(sh as u32) as u64),
                    "sraiw" => sext32((v as i32).wrapping_shr(sh as u32) as u64),
                    _ => unreachable!(),
                });
            }
        }
        a.ret();
        out.push(Case::ok(&format!("shift/{name}"), a.finish(), expect));
    }
    out
}

fn utype_cases() -> Vec<Case> {
    let imms: [u32; 5] = [0, 1, 0x7FFFF, 0x80000, 0xFFFFF];
    let sext20_shl12 = |imm: u32| -> u64 {
        // LUI/AUIPC value: imm20 placed in bits 31:12, bit 31
        // sign-extended to 64 (sext32 of the shifted 32-bit value).
        let v32 = ((imm as u64) << 12) & 0xFFFF_FFFF;
        if v32 & (1 << 31) != 0 {
            v32 | 0xFFFF_FFFF_0000_0000
        } else {
            v32
        }
    };
    // lui
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        for (k, imm) in imms.iter().enumerate() {
            a.u(28, 0x37, *imm);
            a.record(28, k + 1);
            expect.push(sext20_shl12(*imm));
        }
        a.ret();
        vec![Case::ok("u/lui", a.finish(), expect)]
    }
    .into_iter()
    .chain({
        // auipc relative check: x28 = auipc(imm) - auipc(0) == sext<<12.
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for imm in imms.iter() {
            // x30 = auipc(0) at THIS pc; x31 = auipc(imm) at NEXT pc;
            // the two instructions are 4 bytes apart, so
            // x31 - x30 = imm - 4.
            a.u(30, 0x17, 0);
            a.u(31, 0x17, *imm);
            a.r(0b0100000, 0, 28, 31, 30, OP); // sub x28, x31, x30
            k += 1;
            a.record(28, k);
            expect.push(sext20_shl12(*imm).wrapping_add(4));
        }
        a.ret();
        vec![Case::ok("u/auipc", a.finish(), expect)]
    })
    .collect()
}

/// Zbb unary forms. All of them are bitmanip (profile bit 0), so the
/// caller picks the profile; `unary_cases(CALC_PROFILE_RV64IM_B)` is the
/// admitted wiring, `unary_cases(CALC_PROFILE_RV64IM)` would be rejected
/// at verify.
fn unary_cases(profile: u8) -> Vec<Case> {
    let forms: &[(&str, u32, u32, u32, u8)] =
        &[
            ("clz", 0b0110000, 1, OPI, 0),
            ("ctz", 0b0110000, 1, OPI, 1),
            ("cpop", 0b0110000, 1, OPI, 2),
            ("sextb", 0b0110000, 1, OPI, 4),
            ("sexth", 0b0110000, 1, OPI, 5),
            ("orcb", 0b0010100, 5, OPI, 0b00111),
            ("rev8", 0b0110101, 5, OPI, 0b11000),
            ("zexth", 0b0000100, 4, OP32, 0), // RV64: OP-32 slot
            ("clzw", 0b0110000, 1, OPI32, 0),
            ("ctzw", 0b0110000, 1, OPI32, 1),
            ("cpopw", 0b0110000, 1, OPI32, 2),
        ];
    forms
        .iter()
        .map(|(name, f7, f3, oc, sel)| {
            let mut a = Asm::new();
            a.prologue();
            let mut expect = Vec::new();
            for (i, v) in VALS.iter().enumerate() {
                a.r(*f7, *f3, 28, 5 + i as u8, *sel, *oc);
                a.record(28, i + 1);
                expect.push(ref_unary(name, *v));
            }
            a.ret();
            Case::ok(&format!("un/{name}"), a.finish(), expect).with_profile(profile)
        })
        .collect()
}

/// Zba/Zbb/Zbs immediate forms (all profile bit 0): bseti / bclri /
/// binvi / bexti / rori / roriw / slli.uw against the VALS boundary set.
fn bit_imm_cases() -> Vec<Case> {
    let mut out = Vec::new();
    // (name, imm_hi, f3, opcode, 5-bit-shamt W form, shamts)
    // `imm_hi` is imm[11:6] for 6-bit shamt forms, imm[11:5] for roriw.
    let forms: [(&str, u32, u32, u32, bool, &[u8]); 7] = [
        ("bseti", 0b001010, 1, OPI, false, &[0, 1, 15, 31, 63]),
        ("bclri", 0b010010, 1, OPI, false, &[0, 1, 15, 31, 63]),
        ("binvi", 0b011010, 1, OPI, false, &[0, 1, 15, 31, 63]),
        ("bexti", 0b010010, 5, OPI, false, &[0, 1, 15, 31, 63]),
        ("rori", 0b011000, 5, OPI, false, &[0, 1, 15, 31, 63]),
        ("roriw", 0b0110000, 5, OPI32, true, &[0, 1, 15, 31]),
        ("slliuw", 0b000010, 1, OPI32, false, &[0, 1, 15, 31, 63]),
    ];
    for (name, hi, f3, oc, w, shs) in forms {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for (i, &v) in VALS.iter().enumerate() {
            for &sh in shs {
                if w {
                    a.sh_w(hi, f3, 28, 5 + i as u8, sh, oc);
                } else {
                    a.sh(hi, f3, 28, 5 + i as u8, sh, oc);
                }
                k += 1;
                a.record(28, k);
                expect.push(match name {
                    "bseti" => v | (1u64 << sh),
                    "bclri" => v & !(1u64 << sh),
                    "binvi" => v ^ (1u64 << sh),
                    "bexti" => (v >> sh) & 1,
                    "rori" => v.rotate_right(sh as u32),
                    "roriw" => sext32((v as u32).rotate_right(sh as u32) as u64),
                    // slli.uw: zero-extended word shifted left, no sext.
                    "slliuw" => (v as u32 as u64).wrapping_shl(sh as u32),
                    _ => unreachable!(),
                });
            }
        }
        a.ret();
        out.push(
            Case::ok(&format!("bimm/{name}"), a.finish(), expect)
                .with_profile(CALC_PROFILE_RV64IM_B),
        );
    }
    out
}

/// The full Zba/Zbb/Zbs/Zbc register matrix under the B profile: all 25
/// ratified register forms against the same 8x8 VALS boundary matrix.
fn bit_reg_cases() -> Vec<Case> {
    reg_matrix_cases(
        &[
            "adduw", "sh1add", "sh2add", "sh3add", "sh1adduw", "sh2adduw", "sh3adduw",
            "andn", "orn", "xnor", "rol", "ror", "rolw", "rorw",
            "min", "minu", "max", "maxu",
            "bset", "bclr", "binv", "bext",
            "clmul", "clmulr", "clmulh",
        ],
        CALC_PROFILE_RV64IM_B,
    )
}

// ==================================================== RVC (profile C) ====
// Halfword encoders, field-for-field against `decode::decode_rvc`. Every
// helper asserts the constraints the decoder enforces (rd != 0 forms,
// reserved immediates) so a mis-encoded case fails here, not mysteriously.

fn rvc_base(f3: u32, op: u32) -> u16 {
    (((f3 as u32) << 13) | op) as u16
}

/// CI with the standard imm[5]|rd|imm[4:0] layout (c.addi/c.li/c.addiw).
fn rvc_ci(f3: u32, rd: u8, imm: i32) -> u16 {
    assert!((-32..=31).contains(&imm), "CI imm {imm}");
    assert!(rd != 0, "CI rd=0 is a hint; not exercised here");
    let u = (imm as u32) & 0x3F;
    rvc_base(f3, 0b01)
        | (((u >> 5) & 1) as u16) << 12
        | ((rd as u16) << 7)
        | (((u & 0x1F) as u16) << 2)
}

fn c_nop() -> u16 {
    // c.addi x0, 0 — the one rd=0 form that is a defined NOP, not a hint.
    rvc_base(0, 0b01)
}

fn c_addi(rd: u8, imm: i32) -> u16 {
    rvc_ci(0, rd, imm)
}

fn c_li(rd: u8, imm: i32) -> u16 {
    rvc_ci(2, rd, imm)
}

fn c_addiw(rd: u8, imm: i32) -> u16 {
    rvc_ci(1, rd, imm)
}

/// c.lui: rd not in {0, 2}, 6-bit nz immediate.
fn c_lui(rd: u8, imm: i32) -> u16 {
    assert!(rd != 0 && rd != 2 && imm != 0, "c.lui reserved form");
    rvc_ci(3, rd, imm)
}

/// c.addi16sp: 10-bit signed immediate (imm[9] is the sign bit, at h12).
fn c_addi16sp(imm: i32) -> u16 {
    assert!(imm != 0 && (-512..=511).contains(&imm), "c.addi16sp imm {imm}");
    let u = (imm as u32) & 0x3FF;
    rvc_base(3, 0b01)
        | ((((u >> 9) & 1) as u16) << 12)
        | (2u16 << 7)
        | ((((u >> 4) & 1) as u16) << 6)
        | ((((u >> 6) & 1) as u16) << 5)
        | ((((u >> 7) & 3) as u16) << 3)
        | ((((u >> 5) & 1) as u16) << 2)
}

/// c.addi4spn: nzuimm byte offset (multiple of 4, < 4096), rd' target.
fn c_addi4spn(rd8: u8, uimm: u32) -> u16 {
    assert!(uimm != 0 && uimm % 4 == 0 && uimm <= 1020, "c.addi4spn {uimm}");
    assert!(rd8 < 8);
    rvc_base(0, 0b00)
        | ((((uimm >> 4) & 3) as u16) << 11)
        | ((((uimm >> 6) & 0xF) as u16) << 7)
        | ((((uimm >> 2) & 1) as u16) << 6)
        | ((((uimm >> 3) & 1) as u16) << 5)
        | ((rd8 as u16) << 2)
}

/// c.slli: shamt[5]|rd|shamt[4:0].
fn c_slli(rd: u8, sh: u8) -> u16 {
    assert!(sh <= 63);
    rvc_base(0, 0b10)
        | (((sh >> 5) & 1) as u16) << 12
        | ((rd as u16) << 7)
        | (((sh & 0x1F) as u16) << 2)
}

/// c.srli / c.srai on rd' (x8..x15).
fn c_srli_srai(rd8: u8, sh: u8, sra: bool) -> u16 {
    assert!(rd8 < 8 && sh <= 63);
    rvc_base(4, 0b01)
        | (((if sra { 1 } else { 0 }) as u16) << 10)
        | ((rd8 as u16) << 7)
        | (((sh >> 5) & 1) as u16) << 12
        | (((sh & 0x1F) as u16) << 2)
}

/// c.andi on rd'.
fn c_andi(rd8: u8, imm: i32) -> u16 {
    assert!(rd8 < 8 && (-32..=31).contains(&imm));
    let u = (imm as u32) & 0x3F;
    rvc_base(4, 0b01)
        | (2u16 << 10)
        | (((u >> 5) & 1) as u16) << 12
        | ((rd8 as u16) << 7)
        | (((u & 0x1F) as u16) << 2)
}

/// c.sub / c.xor / c.or / c.and / c.subw / c.addw (CA group).
/// sel: 00 sub/subw, 01 xor/addw, 10 or, 11 and; `w` picks the W row.
fn c_arith(w: bool, sel: u32, rd8: u8, rs2_8: u8) -> u16 {
    assert!(rd8 < 8 && rs2_8 < 8 && sel <= 3);
    rvc_base(4, 0b01)
        | (3u16 << 10)
        | (((w as u32) as u16) << 12)
        | ((rd8 as u16) << 7)
        | ((sel as u16) << 5)
        | ((rs2_8 as u16) << 2)
}

/// c.mv / c.add (CR, rs2 != 0).
fn c_mv_add(rd: u8, rs2: u8, add: bool) -> u16 {
    assert!(rs2 != 0);
    rvc_base(4, 0b10) | (((add as u32) as u16) << 12) | ((rd as u16) << 7) | ((rs2 as u16) << 2)
}

/// c.jr / c.jalr (rs1 != 0); c.jalr links pc+2 into x1.
fn c_jr(rs1: u8, jalr: bool) -> u16 {
    assert!(rs1 != 0, "c.jr/jalr rs1=0 reserved");
    rvc_base(4, 0b10) | (((jalr as u32) as u16) << 12) | ((rs1 as u16) << 7)
}

fn c_ebreak() -> u16 {
    0x9002
}

/// c.lw / c.ld / c.sw / c.sd (CL/CS on rs1'/rd'/rs2').
/// lw/sw: uimm[5:3]=h[12:10], uimm[2]=h6, uimm[6]=h5 (scale 4, <=252).
/// ld/sd: uimm[5:3]=h[12:10], uimm[7:6]=h[6:5] (scale 8, <=248).
fn c_lw_ld_sw_sd(f3: u32, a8: u8, base8: u8, uimm: u32) -> u16 {
    assert!(a8 < 8 && base8 < 8);
    let h = rvc_base(f3, 0b00)
        | ((((uimm >> 3) & 7) as u16) << 10)
        | ((base8 as u16) << 7)
        | ((a8 as u16) << 2);
    if f3 == 2 || f3 == 6 {
        assert_eq!(uimm % 4, 0, "c.lw/sw off {uimm} not 4-aligned");
        assert!(uimm <= 252, "c.lw/sw offset {uimm} out of range");
        h | ((((uimm >> 2) & 1) as u16) << 6) | ((((uimm >> 6) & 1) as u16) << 5)
    } else {
        assert!(f3 == 3 || f3 == 7, "bad CL/CS f3 {f3}");
        assert_eq!(uimm % 8, 0, "c.ld/sd off {uimm} not 8-aligned");
        assert!(uimm <= 248, "c.ld/sd offset {uimm} out of range");
        h | ((((uimm >> 6) & 3) as u16) << 5)
    }
}

/// c.lwsp / c.ldsp (rd != 0, positive sp offset).
/// lwsp: imm[5]=h12, imm[4:2]=h[6:4], imm[7:6]=h[3:2] (scale 4, <=252).
/// ldsp: imm[5]=h12, imm[4:3]=h[6:5], imm[8:6]=h[4:2] (scale 8, <=504).
fn c_lwsp_ldsp(f3: u32, rd: u8, uimm: u32) -> u16 {
    assert!(rd != 0);
    let h = rvc_base(f3, 0b10)
        | ((((uimm >> 5) & 1) as u16) << 12)
        | ((rd as u16) << 7);
    if f3 == 2 {
        assert_eq!(uimm % 4, 0, "c.lwsp off {uimm} not 4-aligned");
        assert!(uimm <= 252, "c.lwsp offset {uimm} out of range");
        h | ((((uimm >> 2) & 7) as u16) << 4) | ((((uimm >> 6) & 3) as u16) << 2)
    } else {
        assert!(f3 == 3, "bad c.l*sp f3 {f3}");
        assert_eq!(uimm % 8, 0, "c.ldsp off {uimm} not 8-aligned");
        assert!(uimm <= 504, "c.ldsp offset {uimm} out of range");
        h | ((((uimm >> 3) & 3) as u16) << 5) | ((((uimm >> 6) & 7) as u16) << 2)
    }
}

/// c.swsp / c.sdsp (CSS, positive sp offset).
/// swsp: imm[5:2]=h[12:9], imm[7:6]=h[8:7] (scale 4, <=252).
/// sdsp: imm[5:3]=h[12:10], imm[8:6]=h[9:7] (scale 8, <=504).
fn c_swsp_sdsp(f3: u32, rs2: u8, uimm: u32) -> u16 {
    let h = rvc_base(f3, 0b10) | ((rs2 as u16) << 2);
    if f3 == 6 {
        assert_eq!(uimm % 4, 0, "c.swsp off {uimm} not 4-aligned");
        assert!(uimm <= 252, "c.swsp offset {uimm} out of range");
        h | ((((uimm >> 2) & 0xF) as u16) << 9) | ((((uimm >> 6) & 3) as u16) << 7)
    } else {
        assert!(f3 == 7, "bad c.s*sp f3 {f3}");
        assert_eq!(uimm % 8, 0, "c.sdsp off {uimm} not 8-aligned");
        assert!(uimm <= 504, "c.sdsp offset {uimm} out of range");
        h | ((((uimm >> 3) & 7) as u16) << 10) | ((((uimm >> 6) & 7) as u16) << 7)
    }
}

/// c.j: CJ immediate, byte offset (even, |off| <= 2048).
fn c_j(off: i32) -> u16 {
    assert_eq!(off & 1, 0, "c.j offset must be even");
    assert!((-2048..=2046).contains(&off), "c.j offset {off}");
    let o = (off as u32) & 0xFFF;
    rvc_base(5, 0b01)
        | ((((o >> 11) & 1) as u16) << 12)
        | ((((o >> 4) & 1) as u16) << 11)
        | ((((o >> 8) & 3) as u16) << 9)
        | ((((o >> 10) & 1) as u16) << 8)
        | ((((o >> 6) & 1) as u16) << 7)
        | ((((o >> 7) & 1) as u16) << 6)
        | ((((o >> 1) & 7) as u16) << 3)
        | ((((o >> 5) & 1) as u16) << 2)
}

/// c.beqz / c.bnez: CB immediate, byte offset (even, |off| <= 256).
fn c_beqz_bnez(rs8: u8, off: i32, eqz: bool) -> u16 {
    assert!(rs8 < 8 && off & 1 == 0 && (-256..=254).contains(&off));
    let o = (off as u32) & 0x1FF;
    rvc_base(if eqz { 6 } else { 7 }, 0b01)
        | ((((o >> 8) & 1) as u16) << 12)
        | ((((o >> 3) & 3) as u16) << 10)
        | ((rs8 as u16) << 7)
        | ((((o >> 6) & 3) as u16) << 5)
        | ((((o >> 1) & 3) as u16) << 3)
        | ((((o >> 5) & 1) as u16) << 2)
}

/// RVC execution vectors: every admitted compressed form at least once
/// with exact host references (profile C), one C+B mix, and the two
/// compressed trap forms. 32-bit prologue/record slots as in the base
/// cases; sp-forms snapshot the entry sp in x27 and restore x2 before the
/// sentinel return so the harness sp invariant holds.
fn compressed_cases() -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    let ok = |name: &str, code: Vec<u8>, expect: Vec<u64>| {
        Case::ok(name, code, expect).with_profile(CALC_PROFILE_RV64IMC)
    };

    // crv/addi: c.addi over every VALS register with edge immediates.
    // c.addi ACCUMULATES (rd += imm), so the reference tracks the running
    // register value.
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..8u8 {
            let mut cur = VALS[i as usize];
            for imm in [-32, -1, 1, 31] {
                a.half(c_addi(5 + i, imm));
                cur = cur.wrapping_add(imm as i64 as u64);
                k += 1;
                a.record(5 + i, k);
                expect.push(cur);
            }
        }
        a.ret();
        out.push(ok("crv/addi", a.finish(), expect));
    }
    // crv/li
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        for (k, imm) in [-32i32, -1, 0, 1, 31].iter().enumerate() {
            a.half(c_li(28, *imm));
            a.record(28, (k + 1) as usize);
            expect.push(*imm as i64 as u64);
        }
        a.ret();
        out.push(ok("crv/li", a.finish(), expect));
    }
    // crv/addiw (rd != 0): accumulating, reference = sext32(running + imm).
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..8u8 {
            let mut cur = VALS[i as usize];
            for imm in [-32, -1, 1, 31] {
                a.half(c_addiw(5 + i, imm));
                cur = sext32(cur.wrapping_add(imm as i64 as u64));
                k += 1;
                a.record(5 + i, k);
                expect.push(cur);
            }
        }
        a.ret();
        out.push(ok("crv/addiw", a.finish(), expect));
    }
    // crv/lui (rd not 0/2, nz imm)
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..8u8 {
            for imm in [-32, -1, 1, 31] {
                a.half(c_lui(5 + i, imm));
                k += 1;
                a.record(5 + i, k);
                expect.push(((imm as i64) << 12) as u64);
            }
        }
        a.ret();
        out.push(ok("crv/lui", a.finish(), expect));
    }
    // crv/slli: accumulating (rd <<= sh).
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..8u8 {
            let mut cur = VALS[i as usize];
            for sh in [0u8, 1, 15, 31, 63] {
                a.half(c_slli(5 + i, sh));
                cur = cur.wrapping_shl(sh as u32);
                k += 1;
                a.record(5 + i, k);
                expect.push(cur);
            }
        }
        a.ret();
        out.push(ok("crv/slli", a.finish(), expect));
    }
    // crv/srli-srai (rd' = x8..x12): accumulating within each register.
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..5u8 {
            let mut cur = VALS[(3 + i) as usize];
            for sh in [0u8, 1, 15, 63] {
                a.half(c_srli_srai(i, sh, false));
                cur >>= sh;
                k += 1;
                a.record(8 + i, k);
                expect.push(cur);
                a.half(c_srli_srai(i, sh, true));
                cur = ((cur as i64).wrapping_shr(sh as u32)) as u64;
                k += 1;
                a.record(8 + i, k);
                expect.push(cur);
            }
        }
        a.ret();
        out.push(ok("crv/srli-srai", a.finish(), expect));
    }
    // crv/andi: accumulating (rd &= sext(imm)).
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..5u8 {
            let mut cur = VALS[(3 + i) as usize];
            for imm in [-32, -1, 1, 31] {
                a.half(c_andi(i, imm));
                cur &= imm as i64 as u64;
                k += 1;
                a.record(8 + i, k);
                expect.push(cur);
            }
        }
        a.ret();
        out.push(ok("crv/andi", a.finish(), expect));
    }
    // crv/arith: the CA group (sub/xor/or/and/subw/addw) on rd'/rs2'
    // pairs. Registers mutate as the case runs, so the reference mirrors
    // the live x8..x12 values.
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut rv = [VALS[3], VALS[4], VALS[5], VALS[6], VALS[7]];
        let mut k = 0;
        for i in 0..5u8 {
            let j = (i + 1) % 5;
            for (sel, w, name) in [
                (0u32, false, "sub"),
                (1, false, "xor"),
                (2, false, "or"),
                (3, false, "and"),
                (0, true, "subw"),
                (1, true, "addw"),
            ] {
                a.half(c_arith(w, sel, i, j));
                let (va, vb) = (rv[i as usize], rv[j as usize]);
                rv[i as usize] = match name {
                    "sub" => va.wrapping_sub(vb),
                    "xor" => va ^ vb,
                    "or" => va | vb,
                    "and" => va & vb,
                    "subw" => ref_reg("subw", va, vb),
                    "addw" => ref_reg("addw", va, vb),
                    _ => unreachable!(),
                };
                k += 1;
                a.record(8 + i, k);
                expect.push(rv[i as usize]);
            }
        }
        a.ret();
        out.push(ok("crv/arith", a.finish(), expect));
    }
    // crv/mv-add
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for i in 0..8u8 {
            a.half(c_mv_add(28, 5 + i, false)); // c.mv x28, xi
            k += 1;
            a.record(28, k);
            expect.push(VALS[i as usize]);
        }
        a.half(c_mv_add(28, 5, true)); // c.add x28, x5
        k += 1;
        a.record(28, k);
        expect.push(VALS.last().unwrap().wrapping_add(VALS[0]));
        a.ret();
        out.push(ok("crv/mv-add", a.finish(), expect));
    }
    // crv/addi4spn: rd' = sp + uimm, checked as a delta against the sp
    // snapshot in x27 (the absolute sp is a runtime value).
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for uimm in [4u32, 64, 252] {
            a.half(c_addi4spn(0, uimm)); // x8 = sp + uimm
            a.r(0b0100000, 0, 28, 8, 27, OP); // x28 = x8 - sp
            k += 1;
            a.record(28, k);
            expect.push(uimm as u64);
        }
        a.ret();
        out.push(ok("crv/addi4spn", a.finish(), expect));
    }
    // crv/addi16sp: delta check + sp restore.
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for imm in [-512i32, -16, 16, 496] {
            a.half(c_addi16sp(imm));
            a.r(0b0100000, 0, 28, 2, 27, OP); // x28 = sp' - sp
            k += 1;
            a.record(28, k);
            expect.push(imm as i64 as u64);
            a.i(0, 2, 27, 0x13, 0); // restore x2 = entry sp
        }
        a.ret();
        out.push(ok("crv/addi16sp", a.finish(), expect));
    }
    // crv/loads: c.ld/c.lw over the 16-byte PATTERN parked at sp-1024.
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 8, 2, 0x13, -1024); // x8 (rs1') = scratch base
        let p0 = u64::from_le_bytes(PATTERN[0..8].try_into().unwrap());
        let p1 = u64::from_le_bytes(PATTERN[8..16].try_into().unwrap());
        a.li64(31, p0, 29);
        a.s(3, 8, 31, 0);
        a.li64(31, p1, 29);
        a.s(3, 8, 31, 8);
        let mut expect = Vec::new();
        let mut k = 0;
        for (off, want) in [(0u32, p0), (8, p1)] {
            a.half(c_lw_ld_sw_sd(3, 4, 0, off)); // c.ld x12, off(x8)
            k += 1;
            a.record(12, k);
            expect.push(want);
        }
        for (off, want) in [
            (0u32, p0 as u32),
            (4, (p0 >> 32) as u32),
            (12, (p1 >> 32) as u32),
        ] {
            a.half(c_lw_ld_sw_sd(2, 3, 0, off)); // c.lw x11, off(x8)
            k += 1;
            a.record(11, k);
            expect.push(want as i32 as i64 as u64);
        }
        a.ret();
        out.push(ok("crv/loads", a.finish(), expect));
    }
    // crv/stores: c.sd/c.sw into scratch, read back with 32-bit loads.
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 8, 2, 0x13, -1024); // scratch base in x8 (rs1')
        a.i(0, 31, 0, 0x13, 0);
        a.s(3, 8, 31, 0);
        a.s(3, 8, 31, 8);
        let mut expect = Vec::new();
        a.half(c_lw_ld_sw_sd(7, 1, 0, 0)); // c.sd x9, 0(x8)
        a.i(3, 28, 8, 0x03, 0);
        a.record(28, 1);
        expect.push(VALS[4]);
        a.half(c_lw_ld_sw_sd(6, 2, 0, 4)); // c.sw x10, 4(x8)
        a.i(2, 28, 8, 0x03, 4);
        a.record(28, 2);
        expect.push(VALS[5] as u32 as i32 as i64 as u64);
        a.ret();
        out.push(ok("crv/stores", a.finish(), expect));
    }
    // crv/sp-loads: c.ldsp/c.lwsp over words parked ABOVE sp (output
    // region, RW): store with 32-bit sd first.
    {
        let mut a = Asm::new();
        a.prologue();
        let p0 = u64::from_le_bytes(PATTERN[0..8].try_into().unwrap());
        let p1 = u32::from_le_bytes(PATTERN[0..4].try_into().unwrap());
        a.li64(31, p0, 29);
        a.s(3, 2, 31, 16); // [sp+16] = p0
        a.li64(31, p1 as u64, 29);
        a.s(3, 2, 31, 8); // [sp+8] = p1
        a.half(c_lwsp_ldsp(3, 28, 16)); // c.ldsp x28, 16(sp)
        a.record(28, 1);
        a.half(c_lwsp_ldsp(2, 28, 8)); // c.lwsp x28, 8(sp)
        a.record(28, 2);
        a.ret();
        out.push(ok("crv/sp-loads", a.finish(), vec![p0, p1 as i32 as i64 as u64]));
    }
    // crv/sp-stores: c.sdsp/c.swsp into [sp+off], read back with 32-bit ld.
    {
        let mut a = Asm::new();
        a.prologue();
        a.half(c_swsp_sdsp(7, 5, 16)); // c.sdsp x5, 16(sp)
        a.i(3, 28, 2, 0x03, 16);
        a.record(28, 1);
        a.half(c_swsp_sdsp(6, 6, 8)); // c.swsp x6, 8(sp)
        a.i(2, 28, 2, 0x03, 8);
        a.record(28, 2);
        a.ret();
        out.push(ok(
            "crv/sp-stores",
            a.finish(),
            vec![VALS[0], VALS[1] as u32 as i32 as i64 as u64],
        ));
    }
    // crv/j-forward: c.j +8 over three c.nop halfwords.
    {
        let mut a = Asm::new();
        let p = a.prologue();
        let _ = p;
        a.half(c_j(8));
        a.half(c_nop());
        a.half(c_nop());
        a.half(c_nop());
        a.i(0, 28, 0, 0x13, 7); // landing
        a.record(28, 1);
        a.ret();
        out.push(ok("crv/j-forward", a.finish(), vec![7]));
    }
    // crv/j-backward: two-iteration countdown loop.
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 8, 0, 0x13, 2); // x8 = 2
        let head = a.bytes.len() as i32;
        a.half(c_addi(8, -1));
        let cur = a.bytes.len() as i32;
        a.half(c_beqz_bnez(0, head - cur, false)); // c.bnez x8, head
        a.record(8, 1);
        a.ret();
        out.push(ok("crv/j-backward", a.finish(), vec![0]));
    }
    // crv/beqz-bnez: taken and fall-through, zero loaded via 32-bit addi.
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 8, 0, 0x13, 0); // x8 = 0 (VALS regs are all nonzero)
        // c.beqz x8, +6 -> lands on the second addi below.
        a.half(c_beqz_bnez(0, 6, true));
        a.i(0, 28, 0, 0x13, 2); // fall-through marker (two halfword slots)
        a.i(0, 28, 0, 0x13, 1); // taken marker
        a.record(28, 1);
        a.half(c_beqz_bnez(1, 6, false)); // c.bnez x9, +6 (x9 = i64::MAX != 0)
        a.i(0, 28, 0, 0x13, 2);
        a.i(0, 28, 0, 0x13, 1);
        a.record(28, 2);
        a.ret();
        out.push(ok("crv/beqz-bnez", a.finish(), vec![1, 1]));
    }
    // crv/jr: computed forward jump across a c.nop pad.
    {
        let mut a = Asm::new();
        a.prologue();
        let b0 = a.bytes.len();
        a.u(30, 0x17, 0); // [b0] x30 = pc0
        a.i(0, 31, 30, 0x13, 16); // x31 = pc0 + 16
        a.half(c_jr(31, false)); // [b0+8]
        a.half(c_nop());
        a.half(c_nop());
        a.half(c_nop());
        assert_eq!(a.bytes.len() - b0, 16);
        a.i(0, 28, 0, 0x13, 0x5A); // [b0+16]
        a.record(28, 1);
        a.ret();
        out.push(ok("crv/jr", a.finish(), vec![0x5A]));
    }
    // crv/jalr-link: link = pc_of_cjalr + 2 = pc0 + 10.
    {
        let mut a = Asm::new();
        a.prologue();
        let b0 = a.bytes.len();
        a.u(30, 0x17, 0); // [b0] x30 = pc0
        a.i(0, 31, 30, 0x13, 16); // x31 = pc0 + 16
        a.half(c_jr(31, true)); // [b0+8] c.jalr x31
        a.half(c_nop());
        a.half(c_nop());
        a.half(c_nop());
        assert_eq!(a.bytes.len() - b0, 16);
        a.i(0, 28, 0, 0x13, 0x5B); // [b0+16] landing
        a.r(0b0100000, 0, 28, 1, 30, OP); // x28 = link - pc0
        a.record(28, 1);
        a.ret();
        out.push(
            Case::ok("crv/jalr-link", a.finish(), vec![10])
                .with_profile(CALC_PROFILE_RV64IMC)
                .with_ra_free(),
        );
    }
    // crv/jr-mid-inst: masked target lands inside a 4-byte word -> Trap.
    {
        let mut a = Asm::new();
        a.u(30, 0x17, 0); // [b0]
        a.i(0, 31, 30, 0x13, 12); // x31 = pc0 + 12
        a.half(c_jr(31, false)); // [b0+8..10]
        a.i(0, 28, 0, 0x13, 0); // [b0+10..14] target 12 is mid-instruction
        a.ret();
        out.push(Case::trap("crv/jr-mid-inst", a.finish()).with_profile(CALC_PROFILE_RV64IMC));
    }
    // crv/ebreak: the compressed trap, end to end.
    {
        let mut a = Asm::new();
        a.prologue();
        a.half(c_ebreak());
        a.ret();
        out.push(Case::trap("crv/ebreak", a.finish()).with_profile(CALC_PROFILE_RV64IMC));
    }
    // crvb/mixed (profile C+B): compressed and bitmanip in one stream.
    {
        let mut a = Asm::new();
        a.prologue();
        a.half(c_mv_add(9, 10, true)); // c.add x9, x10
        a.r(0b0100000, 7, 28, 9, 5, OP); // andn x28, x9, x5
        a.record(28, 1);
        a.ret();
        let want = VALS[4].wrapping_add(VALS[5]) & !VALS[0];
        out.push(
            Case::ok("crvb/mixed", a.finish(), vec![want])
                .with_profile(CALC_PROFILE_RV64IMC_B),
        );
    }
    out
}

/// On-chip mulhsu correction identity: mulhsu - mulh - (bit63(b) ? a : 0)
/// must be 0 (mod 2^64) for every VALS pair.
fn mulhsu_identity_case() -> Case {
    let mut a = Asm::new();
    a.prologue();
    let mut k = 0;
    for i in 0..8u8 {
        for j in 0..8u8 {
            a.r(0b0000001, 1, 30, 5 + i, 5 + j, OP); // x30 = mulh
            a.r(0b0000001, 2, 31, 5 + i, 5 + j, OP); // x31 = mulhsu
            a.r(0b0100000, 0, 30, 31, 30, OP); // x30 = mulhsu - mulh
            a.sh(0b010000, 5, 31, 5 + j, 63, OPI); // x31 = bj >> 63 (srai)
            a.r(0b0000000, 7, 31, 31, 5 + i, OP); // x31 &= ai
            a.r(0b0100000, 0, 28, 30, 31, OP); // x28 = diff - correction
            k += 1;
            a.record(28, k);
        }
    }
    a.ret();
    Case::ok("m/mulhsu-identity", a.finish(), vec![0; k])
}

fn load_store_cases() -> Vec<Case> {
    let mut out = Vec::new();
    // Loads over the 16-byte pattern at [sp-1024-16, sp-1024).
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 30, 2, 0x13, -1024); // x30 = sp - 1024 (16-aligned)
        let p0 = u64::from_le_bytes(PATTERN[0..8].try_into().unwrap());
        let p1 = u64::from_le_bytes(PATTERN[8..16].try_into().unwrap());
        a.li64(31, p0, 29);
        a.s(3, 30, 31, 0);
        a.li64(31, p1, 29);
        a.s(3, 30, 31, 8);
        let mut expect = Vec::new();
        let mut k = 0;
        let forms: &[(&str, u32, &[i32])] = &[
            ("lb", 0, &[0, 1, 7, 15]),
            ("lbu", 4, &[0, 1, 7, 15]),
            ("lh", 1, &[0, 2, 14]),
            ("lhu", 5, &[0, 2, 14]),
            ("lw", 2, &[0, 4, 12]),
            ("lwu", 6, &[0, 4, 12]),
            ("ld", 3, &[0, 8]),
        ];
        for (name, f3, offs) in forms {
            for &off in offs.iter() {
                a.i(*f3, 28, 30, 0x03, off);
                k += 1;
                a.record(28, k);
                let o = off as usize;
                expect.push(match *name {
                    "lb" => PATTERN[o] as i8 as i64 as u64,
                    "lbu" => PATTERN[o] as u64,
                    "lh" => {
                        i16::from_le_bytes(PATTERN[o..o + 2].try_into().unwrap()) as i64 as u64
                    }
                    "lhu" => {
                        u16::from_le_bytes(PATTERN[o..o + 2].try_into().unwrap()) as u64
                    }
                    "lw" => {
                        i32::from_le_bytes(PATTERN[o..o + 4].try_into().unwrap()) as i64 as u64
                    }
                    "lwu" => {
                        u32::from_le_bytes(PATTERN[o..o + 4].try_into().unwrap()) as u64
                    }
                    "ld" => u64::from_le_bytes(PATTERN[o..o + 8].try_into().unwrap()),
                    _ => unreachable!(),
                });
            }
        }
        a.ret();
        out.push(Case::ok("mem/loads", a.finish(), expect));
    }
    // Stores + readback.
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 30, 2, 0x13, -1024); // scratch base
        a.i(0, 31, 0, 0x13, 0);
        a.s(3, 30, 31, 0); // zero 24 bytes of scratch
        a.s(3, 30, 31, 8);
        a.s(3, 30, 31, 16);
        let mut expect = Vec::new();
        let mut k = 0;
        // sb 0x80 @0 ; sb 0x7F @1 ; sh 0x8001 @2 ; sh 0xFFFE @4 -> word0
        a.li64(31, 0x80, 29);
        a.s(0, 30, 31, 0);
        a.li64(31, 0x7F, 29);
        a.s(0, 30, 31, 1);
        a.li64(31, 0x8001, 29);
        a.s(1, 30, 31, 2);
        a.li64(31, 0xFFFE, 29);
        a.s(1, 30, 31, 4);
        a.i(3, 28, 30, 0x03, 0);
        k += 1;
        a.record(28, k);
        expect.push(0x80u64 | (0x7F << 8) | (0x8001u64 << 16) | (0xFFFEu64 << 32));
        // sw @8 / sw @12 -> word1
        a.li64(31, 0x8000_0001, 29);
        a.s(2, 30, 31, 8);
        a.li64(31, 0x7FFF_FFFF, 29);
        a.s(2, 30, 31, 12);
        a.i(3, 28, 30, 0x03, 8);
        k += 1;
        a.record(28, k);
        expect.push(0x8000_0001u64 | (0x7FFF_FFFFu64 << 32));
        // sd @16
        a.li64(31, 0xDEAD_BEEF_CAFE_F00D, 29);
        a.s(3, 30, 31, 16);
        a.i(3, 28, 30, 0x03, 16);
        k += 1;
        a.record(28, k);
        expect.push(0xDEAD_BEEF_CAFE_F00D);
        a.ret();
        out.push(Case::ok("mem/stores", a.finish(), expect));
    }
    out
}

fn branch_jump_cases() -> Vec<Case> {
    let mut out = Vec::new();
    // Every branch kind on three operand pairs (equal / less / greater in
    // both signednesses). Layout: [P]=branch -> [P+3]; [P+1]=li x28,2
    // (fallthrough marker); [P+2]=j -> [P+4]; [P+3]=li x28,1 (taken
    // marker); [P+4]=record; ret.
    let kinds: &[(u32, &str)] = &[
        (0, "beq"),
        (1, "bne"),
        (4, "blt"),
        (5, "bge"),
        (6, "bltu"),
        (7, "bgeu"),
    ];
    let pairs: [(u8, u8); 3] = [(5, 5), (5, 6), (6, 5)]; // VALS indices
    for (f3, name) in kinds {
        for (ia, ib) in pairs {
            let mut a = Asm::new();
            let p = a.prologue();
            let taken = match *name {
                "beq" => ia == ib,
                "bne" => ia != ib,
                "blt" => (VALS[ia as usize] as i64) < (VALS[ib as usize] as i64),
                "bge" => (VALS[ia as usize] as i64) >= (VALS[ib as usize] as i64),
                "bltu" => VALS[ia as usize] < VALS[ib as usize],
                "bgeu" => VALS[ia as usize] >= VALS[ib as usize],
                _ => unreachable!(),
            };
            a.b(*f3, 5 + ia, 5 + ib, p + 3); // -> taken marker
            a.i(0, 28, 0, 0x13, 2); // [P+1] fallthrough marker
            a.j(0, p + 4); // [P+2] skip the taken marker
            a.i(0, 28, 0, 0x13, 1); // [P+3] taken marker
            a.record(28, 1);
            a.ret();
            out.push(Case::ok(
                &format!("br/{name}/{}", if taken { "taken" } else { "fall" }),
                a.finish(),
                vec![if taken { 1 } else { 2 }],
            ));
        }
    }
    // Backward branch: 3-iteration countdown loop.
    {
        let mut a = Asm::new();
        let p = a.prologue();
        a.i(0, 28, 0, 0x13, 3); // counter = 3
        let head = p + 1;
        a.i(0, 28, 28, 0x13, -1); // head: counter -= 1
        a.b(1, 28, 0, head); // bne counter, x0, head (backward)
        a.record(28, 1);
        a.ret();
        out.push(Case::ok("br/backward-loop", a.finish(), vec![0]));
    }
    // jal over dead code; link pinned relative to an adjacent auipc.
    // The link register is x29 (NOT ra) so the sentinel return survives.
    {
        let mut a = Asm::new();
        let p = a.prologue();
        a.u(30, 0x17, 0); // [P]   x30 = pc0
        a.j(29, p + 4); // [P+1] jal x29 over 2 dead words
        a.i(0, 31, 0, 0x13, 0x5A); // [P+2] dead
        a.i(0, 31, 0, 0x13, 0x5B); // [P+3] dead
        a.i(0, 28, 0, 0x13, 0x33); // [P+4] landing
        a.r(0b0100000, 0, 28, 29, 30, OP); // x28 = link - pc0
        a.record(28, 1); // link = jal_addr + 4 = pc0 + 8
        a.ret();
        out.push(Case::ok("jump/jal", a.finish(), vec![8]));
    }
    // jalr with odd computed target (bit 0 masked) landing mid-stream.
    {
        let mut a = Asm::new();
        let _p = a.prologue();
        a.u(30, 0x17, 0); // [P]   x30 = pc0
        a.i(0, 30, 30, 0x13, 12 + 1); // [P+1] x30 = pc0 + 13 (odd)
        a.i(0, 0, 30, 0x67, 0); // [P+2] jalr x0, x30, 0 -> pc0 + 12
        a.i(0, 31, 0, 0x13, 0x99); // [P+3] (+12) executed
        a.i(0, 28, 0, 0x13, 0x77); // [P+4] (+16) executed
        a.record(31, 1);
        a.record(28, 2);
        a.ret();
        out.push(Case::ok("jump/jalr-mask", a.finish(), vec![0x99, 0x77]));
    }
    // jalr link value relative to an adjacent auipc (link into x29 so
    // the sentinel return survives): link - auipc = 12.
    {
        let mut a = Asm::new();
        let _p = a.prologue();
        a.u(30, 0x17, 0); // [P]   x30 = pc0
        a.i(0, 30, 30, 0x13, 24); // [P+1] target = pc0 + 24
        a.i(0, 29, 30, 0x67, 0); // [P+2] jalr x29, x30, 0 ; link = pc0 + 12
        a.i(0, 31, 0, 0x13, 1); // [P+3] executed
        a.i(0, 31, 0, 0x13, 2); // [P+4]
        a.i(0, 31, 0, 0x13, 3); // [P+5]
        a.i(0, 31, 0, 0x13, 4); // [P+6] (= pc0+24) landing
        a.i(0, 30, 30, 0x13, -24); // x30 back to pc0
        a.r(0b0100000, 0, 28, 29, 30, OP); // x28 = link - pc0
        a.record(28, 1);
        a.ret();
        out.push(Case::ok("jump/jalr-link", a.finish(), vec![12]));
    }
    out
}

fn fence_case() -> Case {
    let mut a = Asm::new();
    a.prologue();
    a.raw(0x0FF0_000F); // fence
    a.i(0, 28, 0, 0x13, 7);
    a.record(28, 1);
    a.ret();
    Case::ok("fence/noop", a.finish(), vec![7])
}

fn trap_cases() -> Vec<Case> {
    let mut out = Vec::new();
    let mut t = |name: &str, code: Vec<u8>| out.push(Case::trap(name, code));
    // store into the code region itself (mapped, read-only)
    {
        let mut a = Asm::new();
        a.prologue();
        a.li64(30, CODE_BASE, 29);
        a.s(3, 30, 5, 0);
        a.ret();
        t("trap/store-code", a.finish());
    }
    // store into rodata: container with rodata; rodata_base = align16(code)
    // — target computed post-build; see `rodata_trap_code` below.
    // load out of bounds
    {
        let mut a = Asm::new();
        a.prologue();
        a.li64(30, mem_len() - 4, 29);
        a.i(3, 28, 30, 0x03, 0);
        a.ret();
        t("trap/load-oob", a.finish());
    }
    // load address wrap
    {
        let mut a = Asm::new();
        a.prologue();
        a.li64(30, u64::MAX - 3, 29);
        a.i(3, 28, 30, 0x03, 0);
        a.ret();
        t("trap/load-wrap", a.finish());
    }
    // store address wrap
    {
        let mut a = Asm::new();
        a.prologue();
        a.li64(30, u64::MAX - 3, 29);
        a.s(3, 30, 5, 0);
        a.ret();
        t("trap/store-wrap", a.finish());
    }
    // unmapped load (addr 0)
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(3, 28, 0, 0x03, 0);
        a.ret();
        t("trap/load-unmapped", a.finish());
    }
    // jalr to a mid-instruction offset
    {
        let mut a = Asm::new();
        a.prologue();
        a.u(30, 0x17, 0);
        a.i(0, 30, 30, 0x13, 6); // +6: inside the next 4-byte word
        a.i(0, 0, 30, 0x67, 0);
        a.ret();
        t("trap/jalr-mid-inst", a.finish());
    }
    // jalr to >u32::MAX (interp traps; AOT must agree - audit B2)
    {
        let mut a = Asm::new();
        a.prologue();
        a.li64(30, 1u64 << 32, 29);
        a.i(0, 0, 30, 0x67, 0);
        a.ret();
        t("trap/jalr-u32-overflow", a.finish());
    }
    // fall off the end of the code stream
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 28, 0, 0x13, 0);
        t("trap/past-end", a.finish());
    }
    // ecall with an unknown builtin number
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 17, 0, 0x13, 99);
        a.ecall();
        a.ret();
        t("trap/ecall-unknown", a.finish());
    }
    // ecall digest destination inside code. Only reachable once the host
    // ecall table is enabled: v1 traps every non-HALT ecall first.
    if false {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 10, 2, 0x13, -1024); // a0 = sp-1024: a mapped, readable src
        a.i(0, 11, 0, 0x13, 0); // len 0 (the charge is still applied)
        a.li64(12, CODE_BASE, 29); // dst = code (read-only)
        a.i(0, 17, 0, 0x13, 1);
        a.ecall();
        a.ret();
        t("trap/ecall-dst-readonly", a.finish());
    }
    // output length exceeds output_max at HALT (a1 carries the length).
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 10, 0, 0x13, 0);
        a.li64(11, OUTPUT_MAX as u64 + 1, 29);
        a.i(0, 17, 0, 0x13, 0);
        a.ecall();
        t("trap/output-overflow", a.finish());
    }
    out
}

fn ecall_cases() -> Vec<Case> {
    // HASH ecall digest cases are per-image. The default generator stays
    // off so the suite remains zk-portable; HASH is covered by module tests.
    if !false {
        return Vec::new();
    }
    let kinds: &[(&str, u8)] = &[
        ("sha2", 1),
        ("sha3", 2),
        ("ripemd160", 3),
        ("keccak256", 4),
        ("blake2s", 5),
        ("blake2b", 6),
    ];
    kinds
        .iter()
        .map(|(name, num)| {
            let mut a = Asm::new();
            a.prologue();
            // deterministic 32-byte input at [sp-1024]
            a.i(0, 30, 2, 0x13, -1024);
            let mut pat = [0u8; 64];
            for (i, b) in pat.iter_mut().enumerate() {
                *b = (i * i + 7) as u8;
            }
            for w in 0..4 {
                a.li64(31, u64::from_le_bytes(pat[w * 8..w * 8 + 8].try_into().unwrap()), 29);
                a.s(3, 30, 31, (w * 8) as i32);
            }
            // digest area at [sp-1152]
            a.i(0, 31, 2, 0x13, -1024 - 128);
            a.i(0, 10, 30, 0x13, 0);
            a.i(0, 11, 0, 0x13, 32);
            a.i(0, 12, 31, 0x13, 0);
            a.i(0, 17, 0, 0x13, *num as i32);
            a.ecall();
            let mut expect = Vec::new();
            let dlen = if *num == 3 { 20 } else { 32 };
            let kind = vm::calc::HashKind::from_a7(*num as u64).unwrap();
            let mut digest = vec![0u8; 32];
            vm::calc::run_hash(kind, &pat[..32], &mut digest[..dlen]);
            let words = dlen.div_ceil(8);
            for w in 0..words {
                let mut buf = [0u8; 8];
                let start = w * 8;
                let end = dlen.min(start + 8);
                buf[..end - start].copy_from_slice(&digest[start..end]);
                a.i(3, 28, 31, 0x03, (w * 8) as i32);
                a.record(28, w + 1);
                expect.push(u64::from_le_bytes(buf));
            }
            let n = words + 1;
            a.record(10, n); // a0 == 0 after a successful ecall
            expect.push(0);
            a.ret();
            Case::ok(&format!("ecall/{name}"), a.finish(), expect)
        })
        .collect()
}

fn cycles_cases() -> Vec<Case> {
    let mut out = Vec::new();
    // single block: 3 addi + ecall_base = 4 cycles
    {
        let mut a = Asm::new();
        a.ret();
        let mut c = Case::ok("cyc/minimal", a.finish(), vec![]);
        c.cycles = Some(4);
        out.push(c);
    }
    // cost table: ld(2) + addi(1) + div(12) + mul(3) + jalr(1)
    {
        let mut a = Asm::new();
        a.i(0, 5, 0, 0x13, 1);
        a.i(0, 6, 0, 0x13, 1);
        a.i(3, 28, 2, 0x03, -8);
        a.i(0, 28, 28, 0x13, 1);
        a.r(0b0000001, 4, 28, 5, 6, OP); // div
        a.r(0b0000001, 0, 28, 5, 6, OP); // mul
        a.ret();
        let mut c = Case::ok("cyc/cost-table", a.finish(), vec![]);
        // 2*addi + ld + addi + div + mul + 3*addi + ecall
        c.cycles = Some(1 + 1 + 2 + 1 + 12 + 3 + 1 + 1 + 1 + 1);
        out.push(c);
    }
    // branch splits blocks: [addi,beq](2) + [li,jalr](2) = 4 (taken)
    {
        let mut a = Asm::new();
        let i0 = a.i(0, 5, 0, 0x13, 0); // [0]
        let _ = i0;
        a.b(0, 5, 0, 3); // [1] beq x5, x0 -> [3]
        a.i(0, 28, 0, 0x13, 0x99); // [2] skipped
        a.ret(); // [3..] halt
        let mut c = Case::ok("cyc/branch-split", a.finish(), vec![]);
        // [addi,beq](2) + halt block (3 addi + ecall)
        c.cycles = Some(6);
        out.push(c);
    }
    // hash pricing: ecall sha2 over 32 bytes = block[setup..] + 64 + 96
    // (program: li a0; li a1,32; li a2; li a7,1; ecall; li a0,0; ret —
    // all one block: 6*1 (li/jalr mix) + ecall cost; hand total below)
    {
        let mut a = Asm::new();
        a.i(0, 30, 2, 0x13, -1024);
        a.i(0, 10, 30, 0x13, 0); // 1
        a.i(0, 11, 0, 0x13, 32); // 1
        a.i(0, 12, 30, 0x13, -128); // 1 (dst inside writable stack area)
        a.i(0, 17, 0, 0x13, 1); // 1
        a.ecall(); // 64 + 96*1
        a.ret();
        // The hash ecall case only applies once the consensus table is on.
        if false {
            let mut c = Case::ok("cyc/ecall-sha2", a.finish(), vec![]);
            // setup addis (5) + hash ecall(1 static) + halt (3 addi + ecall)
            // ecall is a terminator so two blocks; total static 10 + hash 160
            c.cycles = Some(10 + 64 + 96);
            out.push(c);
        }
    }
    // infinite 1-cycle loop truncated by the budget
    {
        let mut a = Asm::new();
        let _ = a.j(0, 0); // jal x0, 0 -> itself
        let c = Case {
            name: "cyc/infinite-oog".to_string(),
            code: a.finish(),
            expect: Vec::new(),
            outcome: Outcome::OutOfGas(1000),
            cycles: Some(1000),
            aot_declines: false,
            sp_free: false,
            ra_free: false,
            profile: CALC_PROFILE_RV64IM,
        };
        out.push(c);
    }
    out
}

/// All instruction-level cases.
pub fn all_cases() -> Vec<Case> {
    let mut out = Vec::new();
    out.extend(reg_matrix_cases(&[
        "add", "sub", "slt", "sltu", "xor", "or", "and", "sll", "srl", "sra",
    ], CALC_PROFILE_RV64IM));
    out.extend(reg_matrix_cases(&[
        "mul", "mulh", "mulhu", "mulhsu", "div", "divu", "rem", "remu",
    ], CALC_PROFILE_RV64IM));
    out.extend(reg_matrix_cases(&[
        "addw", "subw", "sllw", "srlw", "sraw", "mulw", "divw", "divuw", "remw", "remuw",
    ], CALC_PROFILE_RV64IM));
    // Zba/Zbb/Zbs/Zbc register + immediate + unary forms (profile B) and
    // the admitted RVC forms (profile C / C+B).
    out.extend(bit_reg_cases());
    out.extend(bit_imm_cases());
    out.extend(unary_cases(CALC_PROFILE_RV64IM_B));
    out.extend(compressed_cases());
    out.push(mulhsu_identity_case());
    out.extend(imm_cases());
    out.extend(shift_imm_cases());
    out.extend(utype_cases());
    out.extend(load_store_cases());
    out.extend(branch_jump_cases());
    out.push(fence_case());
    out.extend(trap_cases());
    out.extend(ecall_cases());
    out.extend(cycles_cases());
    out
}

/// Store-into-rodata trap: `code` padded so `rodata_base` (= align16 of
/// the padded code length) is addressable by a 12-bit immediate from a
/// register loaded with `align16(len)`. The container gets 16 rodata
/// bytes; the store must trap.
pub fn rodata_trap_parts() -> (Vec<u8>, Vec<u8>, u64) {
    let mut a = Asm::new();
    // x30 = 16 (will be patched to rodata_base by the caller adding the
    // page offset: we instead emit addi with imm = 0 and rely on the base
    // register value written by two instructions built after padding).
    // Simplest robust form: the program stores at address computed as
    // (x2 - 2048) which is inside... no — rodata must be targeted. The
    // caller pads code to 16N and REWRITES the addi immediate at the
    // fixed offset 4 (second instruction) to align16(code_len).
    a.u(30, 0x37, 1); // lui x30, 1 -> CODE_BASE
    let i1 = a.i(0, 30, 30, 0x13, 0); // addi x30, x30, IMM (patched)
    let _ = i1;
    let s0 = a.s(3, 30, 5, 0); // sd x5, 0(x30)
    let _ = s0;
    a.ret();
    let code = a.finish();
    let rodata = vec![0xAAu8; 16];
    (code, rodata, 0) // imm placeholder; caller patches code[4..8]
}
