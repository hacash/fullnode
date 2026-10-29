//! RV64IMC(+Zbb/Zbc) whitelist instruction decoder: only whitelisted
//! encodings decode.
//!
//! Decodes one instruction at `off` in the code stream into an executable
//! `Inst`. The `isa_b` flag (container `isa_profile == 2`) additionally
//! enables the ratified Zbb/Zbc bit-manipulation set. Branch/JAL immediates
//! stay byte-relative here; the verifier (`verify.rs`) resolves them to
//! absolute in-container targets. Encoding reference: RISC-V unprivileged
//! spec Volume I (I, M, C chapters) and the Bitmanip 1.0.0 release
//! (Zbb/Zbc), cross-checked against clang 18 encodings in `decode` unit
//! tests and against qemu/ckb-vm in the differential suite.

use crate::rt::{ItrErr, ItrErrCode};

fn bad(t: &str) -> ItrErr {
    ItrErr::new(ItrErrCode::ContractError, t)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrKind {
    Eq,
    Ne,
    Lt,
    Ge,
    Ltu,
    Geu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    Lb,
    Lh,
    Lw,
    Ld,
    Lbu,
    Lhu,
    Lwu,
}

impl LoadOp {
    pub fn size(self) -> u64 {
        match self {
            LoadOp::Lb | LoadOp::Lbu => 1,
            LoadOp::Lh | LoadOp::Lhu => 2,
            LoadOp::Lw | LoadOp::Lwu => 4,
            LoadOp::Ld => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    Sb,
    Sh,
    Sw,
    Sd,
}

impl StoreOp {
    pub fn size(self) -> u64 {
        match self {
            StoreOp::Sb => 1,
            StoreOp::Sh => 2,
            StoreOp::Sw => 4,
            StoreOp::Sd => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKind {
    Sll,
    Srl,
    Sra,
    Ror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmAluOp {
    Addi,
    Slti,
    Sltiu,
    Xori,
    Ori,
    Andi,
    Addiw,
}

/// Unary Zbb operations (`rd = f(rs1)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnAluOp {
    Clz,
    Ctz,
    Cpop,
    Sextb,
    Sexth,
    Zexth,
    Orcb,
    Rev8,
    Clzw,
    Ctzw,
    Cpopw,
}

/// Single-bit immediate operations from Zbb (`rd = rs1 <op> 1 << shamt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitImmOp {
    Set,
    Clr,
    Inv,
    Ext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegAluOp {
    Add,
    Sub,
    Sll,
    Slt,
    Sltu,
    Xor,
    Srl,
    Sra,
    Or,
    And,
    Mul,
    Mulh,
    Mulhsu,
    Mulhu,
    Div,
    Divu,
    Rem,
    Remu,
    Addw,
    Subw,
    Sllw,
    Srlw,
    Sraw,
    Mulw,
    Divw,
    Divuw,
    Remw,
    Remuw,
    // ---- Zbb / Zbc (isa_profile 2 only) ----
    Adduw,
    Sh1add,
    Sh2add,
    Sh3add,
    Sh1adduw,
    Sh2adduw,
    Sh3adduw,
    Andn,
    Orn,
    Xnor,
    Rol,
    Ror,
    Rolw,
    Rorw,
    Min,
    Minu,
    Max,
    Maxu,
    Bset,
    Bclr,
    Binv,
    Bext,
    Clmul,
    Clmulh,
    Clmulr,
}

/// Pre-decoded instruction. Branch/JAL targets are byte offsets relative to
/// this instruction's own address until the verifier rewrites them to
/// absolute container targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inst {
    /// rd = val (already sign-expanded; LUI imm20 << 12 / C.LUI sext << 12)
    Lui { rd: u8, val: i64 },
    /// rd = pc + val (val likewise pre-shifted)
    Auipc { rd: u8, val: i64 },
    /// rd = pc+4 (rd != 0); target = pc + imm
    Jal { rd: u8, target: i64 },
    /// rd = pc+4; pc = (rs1 + imm) & !1
    Jalr { rd: u8, rs1: u8, imm: i64 },
    Branch { kind: BrKind, rs1: u8, rs2: u8, target: i64 },
    Load { op: LoadOp, rd: u8, rs1: u8, imm: i64 },
    Store { op: StoreOp, rs1: u8, rs2: u8, imm: i64 },
    ImmAlu { op: ImmAluOp, rd: u8, rs1: u8, imm: i64 },
    /// shift by immediate; `word` selects the W (32-bit) form, shamt < 64
    /// (and < 32 when word).
    ShiftImm { kind: ShiftKind, rd: u8, rs1: u8, shamt: u8, word: bool },
    RegAlu { op: RegAluOp, rd: u8, rs1: u8, rs2: u8 },
    /// Unary Zbb op (clz/ctz/cpop/sext.b/sext.h/zext.h/orc.b/rev8 + W forms).
    UnAlu { op: UnAluOp, rd: u8, rs1: u8 },
    /// Single-bit immediate Zbb op (bseti/bclri/binvi/bexti), shamt < 64.
    BitImm { op: BitImmOp, rd: u8, rs1: u8, shamt: u8 },
    /// SLLI.UW: zero-extend rs1's low 32 bits, then shift left by shamt (< 64);
    /// the result is a full 64-bit value (no sign truncation).
    Slliuw { rd: u8, rs1: u8, shamt: u8 },
    /// FENCE — accepted for toolchain compatibility; single-hart pure-compute
    /// machine, so it executes as a no-op.
    Fence,
    Ecall,
}

#[inline]
fn bits(x: u32, hi: u32, lo: u32) -> u32 {
    (x >> lo) & ((1 << (hi - lo + 1)) - 1)
}

#[inline]
fn sext(v: u32, sign_bit: u32) -> i64 {
    // v is a `sign_bit+1` bit value; sign-extend to i64.
    let shift = 63 - sign_bit;
    (((v as u64) << shift) as i64) >> shift
}

/// Decode the instruction at `off`. Returns the instruction and its encoded
/// length in bytes (2 or 4). `isa_b` enables the Zbb/Zbc subset.
pub fn decode_at(code: &[u8], off: usize, isa_b: bool) -> Result<(Inst, u32), ItrErr> {
    if off + 2 > code.len() {
        return Err(bad("calc decode: truncated instruction"));
    }
    let half = u16::from_le_bytes([code[off], code[off + 1]]) as u32;
    if half & 0b11 != 0b11 {
        decode_rvc(half)
    } else {
        if off + 4 > code.len() {
            return Err(bad("calc decode: truncated 32-bit instruction"));
        }
        let w = u32::from_le_bytes([code[off], code[off + 1], code[off + 2], code[off + 3]]);
        decode_32(w, isa_b)
    }
}

fn decode_32(w: u32, isa_b: bool) -> Result<(Inst, u32), ItrErr> {
    let opcode = w & 0x7f;
    let rd = bits(w, 11, 7) as u8;
    let funct3 = bits(w, 14, 12);
    let rs1 = bits(w, 19, 15) as u8;
    let rs2 = bits(w, 24, 20) as u8;
    let funct7 = bits(w, 31, 25);
    let i_imm = sext(bits(w, 31, 20), 11);
    let s_imm = sext((bits(w, 31, 25) << 5) | bits(w, 11, 7), 11);
    let inst = match opcode {
        0b0110111 => Inst::Lui { rd, val: sext(bits(w, 31, 12), 19) << 12 },
        0b0010111 => Inst::Auipc { rd, val: sext(bits(w, 31, 12), 19) << 12 },
        0b1101111 => {
            // JAL: imm[20|10:1|11|19:12]
            let imm = (bits(w, 31, 31) << 20)
                | (bits(w, 30, 21) << 1)
                | (bits(w, 20, 20) << 11)
                | (bits(w, 19, 12) << 12);
            Inst::Jal { rd, target: sext(imm, 20) }
        }
        0b1100111 => {
            if funct3 != 0 {
                return Err(bad("calc decode: jalr funct3"));
            }
            Inst::Jalr { rd, rs1, imm: i_imm }
        }
        0b1100011 => {
            let imm = (bits(w, 31, 31) << 12)
                | (bits(w, 30, 25) << 5)
                | (bits(w, 11, 8) << 1)
                | (bits(w, 7, 7) << 11);
            let kind = match funct3 {
                0 => BrKind::Eq,
                1 => BrKind::Ne,
                4 => BrKind::Lt,
                5 => BrKind::Ge,
                6 => BrKind::Ltu,
                7 => BrKind::Geu,
                _ => return Err(bad("calc decode: branch funct3")),
            };
            Inst::Branch { kind, rs1, rs2, target: sext(imm, 12) }
        }
        0b0000011 => {
            let op = match funct3 {
                0 => LoadOp::Lb,
                1 => LoadOp::Lh,
                2 => LoadOp::Lw,
                3 => LoadOp::Ld,
                4 => LoadOp::Lbu,
                5 => LoadOp::Lhu,
                6 => LoadOp::Lwu,
                _ => return Err(bad("calc decode: load funct3")),
            };
            Inst::Load { op, rd, rs1, imm: i_imm }
        }
        0b0100011 => {
            let op = match funct3 {
                0 => StoreOp::Sb,
                1 => StoreOp::Sh,
                2 => StoreOp::Sw,
                3 => StoreOp::Sd,
                _ => return Err(bad("calc decode: store funct3")),
            };
            Inst::Store { op, rs1, rs2, imm: s_imm }
        }
        0b0010011 => match funct3 {
            0 => Inst::ImmAlu { op: ImmAluOp::Addi, rd, rs1, imm: i_imm },
            2 => Inst::ImmAlu { op: ImmAluOp::Slti, rd, rs1, imm: i_imm },
            3 => Inst::ImmAlu { op: ImmAluOp::Sltiu, rd, rs1, imm: i_imm },
            4 => Inst::ImmAlu { op: ImmAluOp::Xori, rd, rs1, imm: i_imm },
            6 => Inst::ImmAlu { op: ImmAluOp::Ori, rd, rs1, imm: i_imm },
            7 => Inst::ImmAlu { op: ImmAluOp::Andi, rd, rs1, imm: i_imm },
            1 => match bits(w, 31, 26) {
                0b000000 => Inst::ShiftImm {
                    kind: ShiftKind::Sll,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                    word: false,
                },
                0b001010 if isa_b => Inst::BitImm {
                    op: BitImmOp::Set,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                },
                0b010010 if isa_b => Inst::BitImm {
                    op: BitImmOp::Clr,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                },
                0b011010 if isa_b => Inst::BitImm {
                    op: BitImmOp::Inv,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                },
                // clz/ctz/cpop/sext.b/sext.h: funct7 == 0110000, selector in
                // the rs2 field.
                0b011000 if isa_b && bits(w, 25, 25) == 0 => {
                    let op = match rs2 {
                        0 => UnAluOp::Clz,
                        1 => UnAluOp::Ctz,
                        2 => UnAluOp::Cpop,
                        4 => UnAluOp::Sextb,
                        5 => UnAluOp::Sexth,
                        _ => return Err(bad("calc decode: zbb unary rs2 selector")),
                    };
                    Inst::UnAlu { op, rd, rs1 }
                }
                _ => return Err(bad("calc decode: slli/zbb-imm funct6")),
            },
            5 => match bits(w, 31, 26) {
                0b000000 => Inst::ShiftImm {
                    kind: ShiftKind::Srl,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                    word: false,
                },
                0b010000 => Inst::ShiftImm {
                    kind: ShiftKind::Sra,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                    word: false,
                },
                0b011000 if isa_b => Inst::ShiftImm {
                    kind: ShiftKind::Ror,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                    word: false,
                },
                // bexti: funct6 == 010010, 6-bit shamt
                0b010010 if isa_b => Inst::BitImm {
                    op: BitImmOp::Ext,
                    rd,
                    rs1,
                    shamt: bits(w, 25, 20) as u8,
                },
                // orc.b: funct7 == 0010100, rs2 == 00111
                0b001010 if isa_b && bits(w, 25, 25) == 0 && rs2 == 0b00111 => {
                    Inst::UnAlu { op: UnAluOp::Orcb, rd, rs1 }
                }
                // rev8 (RV64): funct7 == 0110101, rs2 == 11000
                0b011010 if isa_b && bits(w, 25, 25) == 1 && rs2 == 0b11000 => {
                    Inst::UnAlu { op: UnAluOp::Rev8, rd, rs1 }
                }
                _ => return Err(bad("calc decode: srli/srai/rori funct6")),
            },
            _ => return Err(bad("calc decode: op-imm funct3")),
        },
        0b0011011 => match funct3 {
            0 => Inst::ImmAlu { op: ImmAluOp::Addiw, rd, rs1, imm: i_imm },
            1 => match bits(w, 31, 26) {
                0b000000 => Inst::ShiftImm {
                    kind: ShiftKind::Sll,
                    rd,
                    rs1,
                    shamt: bits(w, 24, 20) as u8,
                    word: true,
                },
                // slli.uw: funct6 == 000010, 6-bit shamt
                0b000010 if isa_b => Inst::Slliuw { rd, rs1, shamt: bits(w, 25, 20) as u8 },
                // clzw/ctzw/cpopw: funct7 == 0110000
                0b011000 if isa_b && bits(w, 25, 25) == 0 => {
                    let op = match rs2 {
                        0 => UnAluOp::Clzw,
                        1 => UnAluOp::Ctzw,
                        2 => UnAluOp::Cpopw,
                        _ => return Err(bad("calc decode: zbb unary-w rs2 selector")),
                    };
                    Inst::UnAlu { op, rd, rs1 }
                }
                _ => return Err(bad("calc decode: slliw/slliuw funct6")),
            },
            5 => match funct7 {
                0b0000000 => {
                    Inst::ShiftImm { kind: ShiftKind::Srl, rd, rs1, shamt: bits(w, 24, 20) as u8, word: true }
                }
                0b0100000 => {
                    Inst::ShiftImm { kind: ShiftKind::Sra, rd, rs1, shamt: bits(w, 24, 20) as u8, word: true }
                }
                // roriw: 5-bit shamt
                0b0110000 if isa_b => Inst::ShiftImm {
                    kind: ShiftKind::Ror,
                    rd,
                    rs1,
                    shamt: bits(w, 24, 20) as u8,
                    word: true,
                },
                _ => return Err(bad("calc decode: srliw/sraiw/roriw funct7")),
            },
            _ => return Err(bad("calc decode: op-imm-32 funct3")),
        },
        0b0110011 => {
            // Base + M table first; the Zbb/Zbc table is only reachable
            // under isa_b.
            if let Some(op) = match (funct7, funct3) {
                (0b0000000, 0) => Some(RegAluOp::Add),
                (0b0000000, 1) => Some(RegAluOp::Sll),
                (0b0000000, 2) => Some(RegAluOp::Slt),
                (0b0000000, 3) => Some(RegAluOp::Sltu),
                (0b0000000, 4) => Some(RegAluOp::Xor),
                (0b0000000, 5) => Some(RegAluOp::Srl),
                (0b0000000, 6) => Some(RegAluOp::Or),
                (0b0000000, 7) => Some(RegAluOp::And),
                (0b0100000, 0) => Some(RegAluOp::Sub),
                (0b0100000, 5) => Some(RegAluOp::Sra),
                (0b0000001, 0) => Some(RegAluOp::Mul),
                (0b0000001, 1) => Some(RegAluOp::Mulh),
                (0b0000001, 2) => Some(RegAluOp::Mulhsu),
                (0b0000001, 3) => Some(RegAluOp::Mulhu),
                (0b0000001, 4) => Some(RegAluOp::Div),
                (0b0000001, 5) => Some(RegAluOp::Divu),
                (0b0000001, 6) => Some(RegAluOp::Rem),
                (0b0000001, 7) => Some(RegAluOp::Remu),
                _ => None,
            } {
                Inst::RegAlu { op, rd, rs1, rs2 }
            } else if isa_b {
                match (funct7, funct3) {
                    (0b0100000, 0b100) => Inst::RegAlu { op: RegAluOp::Xnor, rd, rs1, rs2 },
                    (0b0100000, 0b110) => Inst::RegAlu { op: RegAluOp::Orn, rd, rs1, rs2 },
                    (0b0100000, 0b111) => Inst::RegAlu { op: RegAluOp::Andn, rd, rs1, rs2 },
                    (0b0110000, 0b001) => Inst::RegAlu { op: RegAluOp::Rol, rd, rs1, rs2 },
                    (0b0110000, 0b101) => Inst::RegAlu { op: RegAluOp::Ror, rd, rs1, rs2 },
                    (0b0000101, 0b001) => Inst::RegAlu { op: RegAluOp::Clmul, rd, rs1, rs2 },
                    (0b0000101, 0b010) => Inst::RegAlu { op: RegAluOp::Clmulr, rd, rs1, rs2 },
                    (0b0000101, 0b011) => Inst::RegAlu { op: RegAluOp::Clmulh, rd, rs1, rs2 },
                    (0b0000101, 0b100) => Inst::RegAlu { op: RegAluOp::Min, rd, rs1, rs2 },
                    (0b0000101, 0b101) => Inst::RegAlu { op: RegAluOp::Minu, rd, rs1, rs2 },
                    (0b0000101, 0b110) => Inst::RegAlu { op: RegAluOp::Max, rd, rs1, rs2 },
                    (0b0000101, 0b111) => Inst::RegAlu { op: RegAluOp::Maxu, rd, rs1, rs2 },
                    (0b0010000, 0b010) => Inst::RegAlu { op: RegAluOp::Sh1add, rd, rs1, rs2 },
                    (0b0010000, 0b100) => Inst::RegAlu { op: RegAluOp::Sh2add, rd, rs1, rs2 },
                    (0b0010000, 0b110) => Inst::RegAlu { op: RegAluOp::Sh3add, rd, rs1, rs2 },
                    (0b0100100, 0b001) => Inst::RegAlu { op: RegAluOp::Bclr, rd, rs1, rs2 },
                    (0b0100100, 0b101) => Inst::RegAlu { op: RegAluOp::Bext, rd, rs1, rs2 },
                    (0b0010100, 0b001) => Inst::RegAlu { op: RegAluOp::Bset, rd, rs1, rs2 },
                    (0b0110100, 0b001) => Inst::RegAlu { op: RegAluOp::Binv, rd, rs1, rs2 },
                    // zext.h: funct7 == 0000100, rs2 must be 0 (reserved otherwise)
                    (0b0000100, 0b100) if rs2 == 0 => Inst::UnAlu { op: UnAluOp::Zexth, rd, rs1 },
                    _ => return Err(bad("calc decode: op funct7/funct3")),
                }
            } else {
                return Err(bad("calc decode: op funct7/funct3"));
            }
        }
        0b0111011 => {
            // W forms of the base/M subset; B-extension W forms are distinct
            // ops and never pass through the W conversion.
            if let Some(op) = match (funct7, funct3) {
                (0b0000000, 0) => Some(RegAluOp::Add),
                (0b0000000, 1) => Some(RegAluOp::Sll),
                (0b0000000, 5) => Some(RegAluOp::Srl),
                (0b0100000, 0) => Some(RegAluOp::Sub),
                (0b0100000, 5) => Some(RegAluOp::Sra),
                (0b0000001, 0) => Some(RegAluOp::Mul),
                (0b0000001, 4) => Some(RegAluOp::Div),
                (0b0000001, 5) => Some(RegAluOp::Divu),
                (0b0000001, 6) => Some(RegAluOp::Rem),
                (0b0000001, 7) => Some(RegAluOp::Remu),
                _ => None,
            } {
                let op = match op {
                    RegAluOp::Add => RegAluOp::Addw,
                    RegAluOp::Sub => RegAluOp::Subw,
                    RegAluOp::Sll => RegAluOp::Sllw,
                    RegAluOp::Srl => RegAluOp::Srlw,
                    RegAluOp::Sra => RegAluOp::Sraw,
                    RegAluOp::Mul => RegAluOp::Mulw,
                    RegAluOp::Div => RegAluOp::Divw,
                    RegAluOp::Divu => RegAluOp::Divuw,
                    RegAluOp::Rem => RegAluOp::Remw,
                    RegAluOp::Remu => RegAluOp::Remuw,
                    other => other,
                };
                Inst::RegAlu { op, rd, rs1, rs2 }
            } else if isa_b {
                match (funct7, funct3) {
                    (0b0000100, 0b000) => Inst::RegAlu { op: RegAluOp::Adduw, rd, rs1, rs2 },
                    (0b0010000, 0b010) => Inst::RegAlu { op: RegAluOp::Sh1adduw, rd, rs1, rs2 },
                    (0b0010000, 0b100) => Inst::RegAlu { op: RegAluOp::Sh2adduw, rd, rs1, rs2 },
                    (0b0010000, 0b110) => Inst::RegAlu { op: RegAluOp::Sh3adduw, rd, rs1, rs2 },
                    (0b0110000, 0b001) => Inst::RegAlu { op: RegAluOp::Rolw, rd, rs1, rs2 },
                    (0b0110000, 0b101) => Inst::RegAlu { op: RegAluOp::Rorw, rd, rs1, rs2 },
                    _ => return Err(bad("calc decode: op-32 funct7/funct3")),
                }
            } else {
                return Err(bad("calc decode: op-32 funct7/funct3"));
            }
        }
        0b0001111 => {
            // FENCE (base I): memory/ordering fences are no-ops on this
            // single-hart, in-memory machine; accepting them keeps rustc/LLVM
            // output friction-free. fence.i and other funct3 values stay
            // rejected (Zifencei is not whitelisted).
            if funct3 != 0 {
                return Err(bad("calc decode: fence funct3 not allowed"));
            }
            Inst::Fence
        }
        0b1110011 => {
            if w == 0x0000_0073 {
                Inst::Ecall
            } else {
                return Err(bad("calc decode: system instruction not allowed"));
            }
        }
        _ => return Err(bad("calc decode: opcode not in RV64IMC whitelist")),
    };
    Ok((inst, 4))
}

fn decode_rvc(h: u32) -> Result<(Inst, u32), ItrErr> {
    let op = h & 0b11;
    let funct3 = bits(h, 15, 13);
    let inst = match op {
        0b00 => {
            // CIW/CL/CS format: bits[4:2] select rd'/rs2' (+8), bits[9:7]
            // select the address register rs1' (+8).
            let rd = (bits(h, 4, 2) + 8) as u8;
            let rs1 = (bits(h, 9, 7) + 8) as u8;
            match funct3 {
                0 => {
                    // C.ADDI4SPN
                    let imm = (bits(h, 12, 11) << 4)
                        | (bits(h, 10, 7) << 6)
                        | (bits(h, 6, 6) << 2)
                        | (bits(h, 5, 5) << 3);
                    if imm == 0 {
                        return Err(bad("calc decode: c.addi4spn reserved imm 0"));
                    }
                    Inst::ImmAlu { op: ImmAluOp::Addi, rd, rs1: 2, imm: imm as i64 }
                }
                2 => {
                    // C.LW
                    let imm = (bits(h, 12, 10) << 3) | (bits(h, 6, 6) << 2) | (bits(h, 5, 5) << 6);
                    Inst::Load { op: LoadOp::Lw, rd, rs1, imm: imm as i64 }
                }
                3 => {
                    // C.LD
                    let imm = (bits(h, 12, 10) << 3) | (bits(h, 6, 5) << 6);
                    Inst::Load { op: LoadOp::Ld, rd, rs1, imm: imm as i64 }
                }
                6 => {
                    // C.SW: rs2 field
                    let rs2 = (bits(h, 4, 2) + 8) as u8;
                    let imm = (bits(h, 12, 10) << 3) | (bits(h, 6, 6) << 2) | (bits(h, 5, 5) << 6);
                    Inst::Store { op: StoreOp::Sw, rs1, rs2, imm: imm as i64 }
                }
                7 => {
                    // C.SD
                    let rs2 = (bits(h, 4, 2) + 8) as u8;
                    let imm = (bits(h, 12, 10) << 3) | (bits(h, 6, 5) << 6);
                    Inst::Store { op: StoreOp::Sd, rs1, rs2, imm: imm as i64 }
                }
                _ => return Err(bad("calc decode: quadrant-0 funct3 not allowed")),
            }
        }
        0b01 => {
            let rd = bits(h, 11, 7) as u8;
            match funct3 {
                0 => {
                    // C.ADDI / C.NOP
                    let imm = sext((bits(h, 12, 12) << 5) | bits(h, 6, 2), 5);
                    Inst::ImmAlu { op: ImmAluOp::Addi, rd, rs1: rd, imm }
                }
                1 => {
                    // C.ADDIW (RV64), rd != 0
                    if rd == 0 {
                        return Err(bad("calc decode: c.addiw rd=0 reserved"));
                    }
                    let imm = sext((bits(h, 12, 12) << 5) | bits(h, 6, 2), 5);
                    Inst::ImmAlu { op: ImmAluOp::Addiw, rd, rs1: rd, imm }
                }
                2 => {
                    // C.LI
                    let imm = sext((bits(h, 12, 12) << 5) | bits(h, 6, 2), 5);
                    Inst::ImmAlu { op: ImmAluOp::Addi, rd, rs1: 0, imm }
                }
                3 => {
                    if rd == 2 {
                        // C.ADDI16SP
                        let imm = (bits(h, 12, 12) << 9)
                            | (bits(h, 6, 6) << 4)
                            | (bits(h, 5, 5) << 6)
                            | (bits(h, 4, 3) << 7)
                            | (bits(h, 2, 2) << 5);
                        if imm == 0 {
                            return Err(bad("calc decode: c.addi16sp reserved imm 0"));
                        }
                        Inst::ImmAlu { op: ImmAluOp::Addi, rd: 2, rs1: 2, imm: sext(imm, 9) }
                    } else {
                        // C.LUI
                        if rd == 0 {
                            return Err(bad("calc decode: c.lui rd=0 reserved"));
                        }
                        let v = sext((bits(h, 12, 12) << 5) | bits(h, 6, 2), 5) << 12;
                        if v == 0 {
                            return Err(bad("calc decode: c.lui reserved imm 0"));
                        }
                        Inst::Lui { rd, val: v }
                    }
                }
                4 => {
                    let rd = (bits(h, 9, 7) + 8) as u8;
                    match bits(h, 11, 10) {
                        0 | 1 => {
                            // C.SRLI / C.SRAI (RV64: 6-bit shamt)
                            let kind = if bits(h, 11, 10) == 1 {
                                ShiftKind::Sra
                            } else {
                                ShiftKind::Srl
                            };
                            let shamt = (bits(h, 12, 12) << 5) | bits(h, 6, 2);
                            Inst::ShiftImm {
                                kind,
                                rd,
                                rs1: rd,
                                shamt: shamt as u8,
                                word: false,
                            }
                        }
                        2 => {
                            // C.ANDI
                            let imm = sext((bits(h, 12, 12) << 5) | bits(h, 6, 2), 5);
                            Inst::ImmAlu { op: ImmAluOp::Andi, rd, rs1: rd, imm }
                        }
                        _ => {
                            let rs2 = (bits(h, 4, 2) + 8) as u8;
                            match (bits(h, 12, 12), bits(h, 6, 5)) {
                                (0, 0) => Inst::RegAlu { op: RegAluOp::Sub, rd, rs1: rd, rs2 },
                                (0, 1) => Inst::RegAlu { op: RegAluOp::Xor, rd, rs1: rd, rs2 },
                                (0, 2) => Inst::RegAlu { op: RegAluOp::Or, rd, rs1: rd, rs2 },
                                (0, 3) => Inst::RegAlu { op: RegAluOp::And, rd, rs1: rd, rs2 },
                                (1, 0) => Inst::RegAlu { op: RegAluOp::Subw, rd, rs1: rd, rs2 },
                                (1, 1) => Inst::RegAlu { op: RegAluOp::Addw, rd, rs1: rd, rs2 },
                                _ => return Err(bad("calc decode: c.arith reserved")),
                            }
                        }
                    }
                }
                5 => {
                    // C.J
                    let off = (bits(h, 12, 12) << 11)
                        | (bits(h, 11, 11) << 4)
                        | (bits(h, 10, 9) << 8)
                        | (bits(h, 8, 8) << 10)
                        | (bits(h, 7, 7) << 6)
                        | (bits(h, 6, 6) << 7)
                        | (bits(h, 5, 3) << 1)
                        | (bits(h, 2, 2) << 5);
                    Inst::Jal { rd: 0, target: sext(off, 11) }
                }
                6 | 7 => {
                    // C.BEQZ / C.BNEZ
                    let rs1 = (bits(h, 9, 7) + 8) as u8;
                    let off = (bits(h, 12, 12) << 8)
                        | (bits(h, 11, 10) << 3)
                        | (bits(h, 6, 5) << 6)
                        | (bits(h, 4, 3) << 1)
                        | (bits(h, 2, 2) << 5);
                    Inst::Branch {
                        kind: if funct3 == 6 { BrKind::Eq } else { BrKind::Ne },
                        rs1,
                        rs2: 0,
                        target: sext(off, 8),
                    }
                }
                _ => unreachable!(),
            }
        }
        0b10 => {
            let rd = bits(h, 11, 7) as u8;
            match funct3 {
                0 => {
                    // C.SLLI (RV64: 6-bit shamt)
                    let shamt = (bits(h, 12, 12) << 5) | bits(h, 6, 2);
                    Inst::ShiftImm { kind: ShiftKind::Sll, rd, rs1: rd, shamt: shamt as u8, word: false }
                }
                2 => {
                    // C.LWSP
                    if rd == 0 {
                        return Err(bad("calc decode: c.lwsp rd=0 reserved"));
                    }
                    let imm = (bits(h, 12, 12) << 5) | (bits(h, 6, 4) << 2) | (bits(h, 3, 2) << 6);
                    Inst::Load { op: LoadOp::Lw, rd, rs1: 2, imm: imm as i64 }
                }
                3 => {
                    // C.LDSP
                    if rd == 0 {
                        return Err(bad("calc decode: c.ldsp rd=0 reserved"));
                    }
                    let imm = (bits(h, 12, 12) << 5) | (bits(h, 6, 5) << 3) | (bits(h, 4, 2) << 6);
                    Inst::Load { op: LoadOp::Ld, rd, rs1: 2, imm: imm as i64 }
                }
                4 => {
                    let rs2 = bits(h, 6, 2) as u8;
                    if bits(h, 12, 12) == 0 {
                        if rs2 == 0 {
                            if rd == 0 {
                                // C.EBREAK
                                return Err(bad("calc decode: c.ebreak not allowed"));
                            }
                            // C.JR
                            Inst::Jalr { rd: 0, rs1: rd, imm: 0 }
                        } else {
                            // C.MV (rd=0 form is a HINT; executes as add x0)
                            Inst::RegAlu { op: RegAluOp::Add, rd, rs1: 0, rs2 }
                        }
                    } else if rs2 == 0 {
                        if rd == 0 {
                            return Err(bad("calc decode: c.jalr rd/rs1 reserved"));
                        }
                        // C.JALR
                        Inst::Jalr { rd: 1, rs1: rd, imm: 0 }
                    } else {
                        // C.ADD
                        Inst::RegAlu { op: RegAluOp::Add, rd, rs1: rd, rs2 }
                    }
                }
                6 => {
                    // C.SWSP
                    let rs2 = bits(h, 6, 2) as u8;
                    let imm = (bits(h, 12, 9) << 2) | (bits(h, 8, 7) << 6);
                    Inst::Store { op: StoreOp::Sw, rs1: 2, rs2, imm: imm as i64 }
                }
                7 => {
                    // C.SDSP
                    let rs2 = bits(h, 6, 2) as u8;
                    let imm = (bits(h, 12, 10) << 3) | (bits(h, 9, 7) << 6);
                    Inst::Store { op: StoreOp::Sd, rs1: 2, rs2, imm: imm as i64 }
                }
                _ => return Err(bad("calc decode: quadrant-2 funct3 not allowed")),
            }
        }
        _ => unreachable!("compressed quadrants are 00/01/10"),
    };
    Ok((inst, 2))
}

/// Cycles charged for the carryless-multiply family. Every other whitelisted
/// B instruction is a single ALU pass (1 cycle, matching the CKB-VM table);
/// clmul is a 64-iteration carryless loop in software, so it is priced by
/// actual host work. CKB charges 1 for it — treated here as their pricing
/// pitfall, not a rule to copy.
pub const CLMUL_CYCLES: u64 = 16;

/// Static cycle cost; ecall is priced at runtime by hash kind.
pub fn inst_cycles(inst: &Inst) -> u64 {
    match inst {
        Inst::Lui { .. }
        | Inst::Auipc { .. }
        | Inst::Jal { .. }
        | Inst::Jalr { .. }
        | Inst::Branch { .. }
        | Inst::ImmAlu { .. }
        | Inst::ShiftImm { .. }
        | Inst::RegAlu { .. } => match inst {
            Inst::RegAlu { op, .. } => match op {
                RegAluOp::Mul
                | RegAluOp::Mulh
                | RegAluOp::Mulhsu
                | RegAluOp::Mulhu
                | RegAluOp::Mulw => 3,
                RegAluOp::Div
                | RegAluOp::Divu
                | RegAluOp::Rem
                | RegAluOp::Remu
                | RegAluOp::Divw
                | RegAluOp::Divuw
                | RegAluOp::Remw
                | RegAluOp::Remuw => 32,
                RegAluOp::Clmul | RegAluOp::Clmulh | RegAluOp::Clmulr => CLMUL_CYCLES,
                _ => 1,
            },
            _ => 1,
        },
        // Unary/single-bit Zbb ops: one ALU pass each.
        Inst::UnAlu { .. } | Inst::BitImm { .. } | Inst::Slliuw { .. } | Inst::Fence => 1,
        Inst::Load { .. } | Inst::Store { .. } => 2,
        Inst::Ecall => 0,
    }
}

#[cfg(test)]
mod zbb_tests {
    use super::*;

    fn r_word(f7: u32, f3: u32, rd: u8, rs1: u8, rs2: u8, opcode: u32) -> u32 {
        (f7 << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (f3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }
    fn sh_word(f6: u32, f3: u32, rd: u8, rs1: u8, shamt: u32, opcode: u32) -> u32 {
        (f6 << 26)
            | (shamt << 20)
            | ((rs1 as u32) << 15)
            | (f3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }

    fn dec(w: u32) -> Inst {
        decode_32(w, true).unwrap().0
    }

    #[test]
    fn zbb_register_ops_decode() {
        let op = 0b0110011u32;
        for (f7, f3, want) in [
            (0b0100000u32, 0b100u32, RegAluOp::Xnor),
            (0b0100000, 0b110, RegAluOp::Orn),
            (0b0100000, 0b111, RegAluOp::Andn),
            (0b0110000, 0b001, RegAluOp::Rol),
            (0b0110000, 0b101, RegAluOp::Ror),
            (0b0000101, 0b001, RegAluOp::Clmul),
            (0b0000101, 0b010, RegAluOp::Clmulr),
            (0b0000101, 0b011, RegAluOp::Clmulh),
            (0b0000101, 0b100, RegAluOp::Min),
            (0b0000101, 0b101, RegAluOp::Minu),
            (0b0000101, 0b110, RegAluOp::Max),
            (0b0000101, 0b111, RegAluOp::Maxu),
            (0b0010000, 0b010, RegAluOp::Sh1add),
            (0b0010000, 0b100, RegAluOp::Sh2add),
            (0b0010000, 0b110, RegAluOp::Sh3add),
            (0b0100100, 0b001, RegAluOp::Bclr),
            (0b0100100, 0b101, RegAluOp::Bext),
            (0b0010100, 0b001, RegAluOp::Bset),
            (0b0110100, 0b001, RegAluOp::Binv),
        ] {
            assert_eq!(
                dec(r_word(f7, f3, 3, 4, 5, op)),
                Inst::RegAlu { op: want, rd: 3, rs1: 4, rs2: 5 },
                "f7={f7:07b} f3={f3:03b}"
            );
            // Profile 1 must reject the same word.
            assert!(decode_32(r_word(f7, f3, 3, 4, 5, op), false).is_err());
        }
        // OP-32 W forms.
        for (f7, f3, want) in [
            (0b0000100u32, 0b000u32, RegAluOp::Adduw),
            (0b0110000, 0b001, RegAluOp::Rolw),
            (0b0110000, 0b101, RegAluOp::Rorw),
            (0b0010000, 0b010, RegAluOp::Sh1adduw),
            (0b0010000, 0b100, RegAluOp::Sh2adduw),
            (0b0010000, 0b110, RegAluOp::Sh3adduw),
        ] {
            assert_eq!(
                dec(r_word(f7, f3, 3, 4, 5, 0b0111011)),
                Inst::RegAlu { op: want, rd: 3, rs1: 4, rs2: 5 },
                "op32 f7={f7:07b} f3={f3:03b}"
            );
        }
    }

    #[test]
    fn zexth_requires_zero_rs2() {
        let w = r_word(0b0000100, 0b100, 3, 4, 0, 0b0110011);
        assert_eq!(dec(w), Inst::UnAlu { op: UnAluOp::Zexth, rd: 3, rs1: 4 });
        let w_bad = r_word(0b0000100, 0b100, 3, 4, 1, 0b0110011);
        assert!(decode_32(w_bad, true).is_err());
    }

    #[test]
    fn zbb_unary_and_imm_ops_decode() {
        let op_imm = 0b0010011u32;
        // clz/ctz/cpop/sext.b/sext.h via the rs2 selector.
        for (rs2, want) in [
            (0u32, UnAluOp::Clz),
            (1, UnAluOp::Ctz),
            (2, UnAluOp::Cpop),
            (4, UnAluOp::Sextb),
            (5, UnAluOp::Sexth),
        ] {
            let w = r_word(0b0110000, 0b001, 3, 4, rs2 as u8, op_imm);
            assert_eq!(dec(w), Inst::UnAlu { op: want, rd: 3, rs1: 4 });
        }
        // orc.b / rev8 fixed-encoding unaries.
        assert_eq!(
            dec(r_word(0b0010100, 0b101, 3, 4, 0b00111, op_imm)),
            Inst::UnAlu { op: UnAluOp::Orcb, rd: 3, rs1: 4 }
        );
        assert_eq!(
            dec(r_word(0b0110101, 0b101, 3, 4, 0b11000, op_imm)),
            Inst::UnAlu { op: UnAluOp::Rev8, rd: 3, rs1: 4 }
        );
        // bseti/bclri/binvi/bexti (6-bit shamt includes bit 25).
        for (f6, want) in [
            (0b001010u32, BitImmOp::Set),
            (0b010010, BitImmOp::Clr),
            (0b011010, BitImmOp::Inv),
        ] {
            let w = sh_word(f6, 0b001, 3, 4, 0b10_0001, op_imm);
            assert_eq!(dec(w), Inst::BitImm { op: want, rd: 3, rs1: 4, shamt: 33 });
        }
        assert_eq!(
            dec(sh_word(0b010010, 0b101, 3, 4, 33, op_imm)),
            Inst::BitImm { op: BitImmOp::Ext, rd: 3, rs1: 4, shamt: 33 }
        );
        // rori (6-bit) / roriw (5-bit).
        assert_eq!(
            dec(sh_word(0b011000, 0b101, 3, 4, 33, op_imm)),
            Inst::ShiftImm { kind: ShiftKind::Ror, rd: 3, rs1: 4, shamt: 33, word: false }
        );
        assert_eq!(
            dec(r_word(0b0110000, 0b101, 3, 4, 0, 0b0011011)),
            Inst::ShiftImm { kind: ShiftKind::Ror, rd: 3, rs1: 4, shamt: 0, word: true }
        );
        // clzw/ctzw/cpopw and slli.uw.
        for (rs2, want) in [(0u32, UnAluOp::Clzw), (1, UnAluOp::Ctzw), (2, UnAluOp::Cpopw)] {
            let w = r_word(0b0110000, 0b001, 3, 4, rs2 as u8, 0b0011011);
            assert_eq!(dec(w), Inst::UnAlu { op: want, rd: 3, rs1: 4 });
        }
        assert_eq!(
            dec(sh_word(0b000010, 0b001, 3, 4, 33, 0b0011011)),
            Inst::Slliuw { rd: 3, rs1: 4, shamt: 33 }
        );
    }

    #[test]
    fn fence_decodes_but_fence_i_rejected() {
        assert_eq!(dec(0x0FF0_000F), Inst::Fence);
        // fence.i: funct3 = 1 (Zifencei) stays outside the whitelist.
        assert!(decode_32(0x0000_100F, true).is_err());
        assert!(decode_32(0x0000_100F, false).is_err());
    }

    #[test]
    fn reserved_zbb_selectors_rejected() {
        // clz table with rs2 = 3 (reserved).
        assert!(decode_32(r_word(0b0110000, 0b001, 3, 4, 3, 0b0010011), true).is_err());
        // clz table with funct7 bit 25 set (reserved).
        assert!(decode_32(r_word(0b0110001, 0b001, 3, 4, 0, 0b0010011), true).is_err());
    }
}
