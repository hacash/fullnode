//! RV64IMC whitelist instruction decoder: only whitelisted encodings decode.
//!
//! Decodes one instruction at `off` in the code stream into an executable
//! `Inst`. Branch/JAL immediates stay byte-relative here; the verifier
//! (`verify.rs`) resolves them to absolute in-container targets. Encoding
//! reference: RISC-V unprivileged spec Volume I (I, M, C chapters). Every
//! acceptance in this file is cross-checked against clang 18 encodings in
//! `decode` unit tests and against qemu/ckb-vm in the differential suite.

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
/// length in bytes (2 or 4).
pub fn decode_at(code: &[u8], off: usize) -> Result<(Inst, u32), ItrErr> {
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
        decode_32(w)
    }
}

fn decode_32(w: u32) -> Result<(Inst, u32), ItrErr> {
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
            1 => {
                if bits(w, 31, 26) != 0 {
                    return Err(bad("calc decode: slli funct6"));
                }
                Inst::ShiftImm { kind: ShiftKind::Sll, rd, rs1, shamt: bits(w, 25, 20) as u8, word: false }
            }
            5 => {
                match bits(w, 31, 26) {
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
                    _ => return Err(bad("calc decode: srli/srai funct6")),
                }
            }
            _ => return Err(bad("calc decode: op-imm funct3")),
        },
        0b0011011 => match funct3 {
            0 => Inst::ImmAlu { op: ImmAluOp::Addiw, rd, rs1, imm: i_imm },
            1 => {
                if funct7 != 0 {
                    return Err(bad("calc decode: slliw funct7"));
                }
                Inst::ShiftImm { kind: ShiftKind::Sll, rd, rs1, shamt: bits(w, 24, 20) as u8, word: true }
            }
            5 => match funct7 {
                0b0000000 => {
                    Inst::ShiftImm { kind: ShiftKind::Srl, rd, rs1, shamt: bits(w, 24, 20) as u8, word: true }
                }
                0b0100000 => {
                    Inst::ShiftImm { kind: ShiftKind::Sra, rd, rs1, shamt: bits(w, 24, 20) as u8, word: true }
                }
                _ => return Err(bad("calc decode: srliw/sraiw funct7")),
            },
            _ => return Err(bad("calc decode: op-imm-32 funct3")),
        },
        0b0110011 | 0b0111011 => {
            let word = opcode == 0b0111011;
            let op = match (funct7, funct3) {
                (0b0000000, 0) => RegAluOp::Add,
                (0b0000000, 1) => RegAluOp::Sll,
                (0b0000000, 2) => RegAluOp::Slt,
                (0b0000000, 3) => RegAluOp::Sltu,
                (0b0000000, 4) => RegAluOp::Xor,
                (0b0000000, 5) => RegAluOp::Srl,
                (0b0000000, 6) => RegAluOp::Or,
                (0b0000000, 7) => RegAluOp::And,
                (0b0100000, 0) => RegAluOp::Sub,
                (0b0100000, 5) => RegAluOp::Sra,
                (0b0000001, 0) => RegAluOp::Mul,
                (0b0000001, 1) => RegAluOp::Mulh,
                (0b0000001, 2) => RegAluOp::Mulhsu,
                (0b0000001, 3) => RegAluOp::Mulhu,
                (0b0000001, 4) => RegAluOp::Div,
                (0b0000001, 5) => RegAluOp::Divu,
                (0b0000001, 6) => RegAluOp::Rem,
                (0b0000001, 7) => RegAluOp::Remu,
                _ => return Err(bad("calc decode: op funct7/funct3")),
            };
            // W forms only exist for a subset; reuse the same table then check.
            if word
                && !matches!(
                    op,
                    RegAluOp::Add
                        | RegAluOp::Sub
                        | RegAluOp::Sll
                        | RegAluOp::Srl
                        | RegAluOp::Sra
                        | RegAluOp::Mul
                        | RegAluOp::Div
                        | RegAluOp::Divu
                        | RegAluOp::Rem
                        | RegAluOp::Remu
                )
            {
                return Err(bad("calc decode: op-32 invalid funct"));
            }
            let op = if word {
                match op {
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
                }
            } else {
                op
            };
            Inst::RegAlu { op, rd, rs1, rs2 }
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
                _ => 1,
            },
            _ => 1,
        },
        Inst::Load { .. } | Inst::Store { .. } => 2,
        Inst::Ecall => 0,
    }
}
