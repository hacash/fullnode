//! Shared ISA-level test-case generator for the calc VM (RV64IM+Zba/Zbb/Zbs/Zbc, no C).
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
    CalcContainer, CALC_ABI_BYTES_TO_BYTES, CALC_PROFILE_RV64IM_B, CODE_BASE,
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
    /// Byte address of every 4-byte instruction.
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
        }
    }
    pub fn container(&self) -> CalcContainer {
        container_for(self.code.clone())
    }
}

pub fn container_for(code: Vec<u8>) -> CalcContainer {
    CalcContainer {
        profile: CALC_PROFILE_RV64IM_B,
        abi: CALC_ABI_BYTES_TO_BYTES,
        mem_kib: MEM_KIB,
        entry: CODE_BASE as u32,
        code,
        rodata: Vec::new(),
        init: Vec::new(),
        bss_len: 0,
        input_max: INPUT_MAX,
        output_max: OUTPUT_MAX,
    }
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
fn reg_matrix_cases(ops: &[&str]) -> Vec<Case> {
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
            Case::ok(&format!("reg/{op}"), a.finish(), expect)
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
        ("rori", 0b011000, 5),
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
                    "rori" => v.rotate_right(sh as u32),
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
        ("roriw", 0b0110000, 5),
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
                    "roriw" => sext32((v as u32).rotate_right(sh as u32) as u64),
                    _ => unreachable!(),
                });
            }
        }
        a.ret();
        out.push(Case::ok(&format!("shift/{name}"), a.finish(), expect));
    }
    // Single-bit immediate Zbb ops over shamt {0,1,31,32,63}.
    let bit: &[(&str, u32, u32)] = &[
        ("bseti", 0b001010, 1),
        ("bclri", 0b010010, 1),
        ("binvi", 0b011010, 1),
        ("bexti", 0b010010, 5),
    ];
    for (name, f6, f3) in bit {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for &sh in &[0u8, 1, 31, 32, 63] {
            for (i, &v) in VALS.iter().enumerate() {
                a.sh(*f6, *f3, 28, 5 + i as u8, sh, OPI);
                k += 1;
                a.record(28, k);
                let mask = 1u64 << sh;
                expect.push(match *name {
                    "bseti" => v | mask,
                    "bclri" => v & !mask,
                    "binvi" => v ^ mask,
                    "bexti" => (v >> sh) & 1,
                    _ => unreachable!(),
                });
            }
        }
        a.ret();
        out.push(Case::ok(&format!("shift/{name}"), a.finish(), expect));
    }
    // slli.uw (OP-IMM-32, funct6 000010, 6-bit shamt).
    {
        let mut a = Asm::new();
        a.prologue();
        let mut expect = Vec::new();
        let mut k = 0;
        for &sh in &[0u8, 1, 31, 32, 63] {
            for (i, &v) in VALS.iter().enumerate() {
                a.sh(0b000010, 1, 28, 5 + i as u8, sh, OPI32);
                k += 1;
                a.record(28, k);
                expect.push((v as u32 as u64).wrapping_shl(sh as u32));
            }
        }
        a.ret();
        out.push(Case::ok("shift/slli.uw", a.finish(), expect));
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

fn unary_cases() -> Vec<Case> {
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
            Case::ok(&format!("un/{name}"), a.finish(), expect)
        })
        .collect()
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
    // store into code (addr 0)
    {
        let mut a = Asm::new();
        a.prologue();
        a.s(3, 0, 5, 0);
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
    // ecall digest destination inside code
    {
        let mut a = Asm::new();
        a.prologue();
        a.i(0, 10, 0, 0x13, 0);
        a.i(0, 11, 0, 0x13, 0);
        a.i(0, 12, 0, 0x13, 8);
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
    // cost table: ld(2) + addi(1) + div(12) + mul(3) + clmul(24) + jalr(1)
    {
        let mut a = Asm::new();
        a.i(0, 5, 0, 0x13, 1);
        a.i(0, 6, 0, 0x13, 1);
        a.i(3, 28, 2, 0x03, -8);
        a.i(0, 28, 28, 0x13, 1);
        a.r(0b0000001, 4, 28, 5, 6, OP); // div
        a.r(0b0000001, 0, 28, 5, 6, OP); // mul
        a.r(0b0000101, 1, 28, 5, 6, OP); // clmul
        a.ret();
        let mut c = Case::ok("cyc/cost-table", a.finish(), vec![]);
        // 2*addi + ld + addi + div + mul + clmul + 3*addi + ecall
        c.cycles = Some(1 + 1 + 2 + 1 + 12 + 3 + 24 + 1 + 1 + 1 + 1);
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
        let mut c = Case::ok("cyc/ecall-sha2", a.finish(), vec![]);
        // setup addis (5) + hash ecall(1 static) + halt (3 addi + ecall)
        // ecall is a terminator so two blocks; total static 10 + hash 160
        c.cycles = Some(10 + 64 + 96);
        out.push(c);
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
    ]));
    out.extend(reg_matrix_cases(&[
        "mul", "mulh", "mulhu", "mulhsu", "div", "divu", "rem", "remu",
    ]));
    out.extend(reg_matrix_cases(&[
        "addw", "subw", "sllw", "srlw", "sraw", "mulw", "divw", "divuw", "remw", "remuw",
    ]));
    out.extend(reg_matrix_cases(&[
        "adduw", "sh1add", "sh2add", "sh3add", "sh1adduw", "sh2adduw", "sh3adduw", "andn", "orn",
        "xnor", "rol", "ror", "rolw", "rorw", "min", "minu", "max", "maxu", "bset", "bclr",
        "binv", "bext",
    ]));
    out.extend(reg_matrix_cases(&["clmul", "clmulh", "clmulr"]));
    out.push(mulhsu_identity_case());
    out.extend(imm_cases());
    out.extend(shift_imm_cases());
    out.extend(utype_cases());
    out.extend(unary_cases());
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
