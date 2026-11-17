use super::*;


/* ------------------------------------------------------------------ */
/* Assembler                                                           */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FixKind {
    Branch,
    Jal,
    /// 2-byte c.beqz/c.bnez slot; relaxed to a full Branch on overflow.
    CBranchZ,
    /// 2-byte c.j slot; relaxed to a full Jal on overflow.
    CJ,
    /// `auipc rd, hi; addi rd, rd, lo` pair loading a label address.
    PcRelHiLo,
    /// `addi rd, base, off` — rd = (base label address) + (lit label offset),
    /// patched once both labels resolve (the pool-base rematerialization).
    PoolOff,
    /// `ld rd, off(base)` — off = lit label − pool-base label (a pooled
    /// constant materialization; the base register holds the pool start).
    PoolLd,
}

pub(crate) struct Fixup {
    pub(crate) at: usize,
    pub(crate) label: usize,
    pub(crate) kind: FixKind,
    /// Branch emission sequence id, used to re-target the sites that need
    /// trampolines in the next relaxation pass (see `branch`).
    pub(crate) seq: usize,
    /// Secondary label (the base for PoolOff).
    pub(crate) aux: usize,
}

/// Minimal two-pass assembler with labels. Encoding paths are pinned by the
/// `assembler_encodings_decode` test through the consensus decoder.
pub(crate) struct Asm {
    pub(crate) buf: Vec<u8>,
    pub(crate) labels: Vec<Option<usize>>,
    pub(crate) fixups: Vec<Fixup>,
    /// Emit RVC (C-extension) encodings where the operands admit them.
    pub(crate) rvc: bool,
    /// Dedicated scratch registers (outside the value pool); mode-dependent.
    pub(crate) sc1: u8,
    pub(crate) sc2: u8,
    /// Monotonic id per conditional-branch emission; stable across passes
    /// because lowering is deterministic.
    pub(crate) branch_seq: usize,
    /// Branch sites (by seq id) that resolved out of the ±4 KiB B-type range
    /// in a previous relaxation pass and must use the jal-trampoline form.
    pub(crate) long_branch_ids: std::collections::HashSet<usize>,
    /// c.bz/c.j sites (by seq id) that did not fit the 2-byte ±256 B / ±2 KiB
    /// windows in a previous pass and must be emitted full-width (the
    /// "compress optimistically, relax on overflow" strategy — same ladder
    /// as LLVM's RISCVAsmBackend c.beqz→beq→inversion).
    pub(crate) wide_c_ids: std::collections::HashSet<usize>,
    /// Monotonic id per jal emission (parallel to branch_seq).
    pub(crate) jal_seq: usize,
}

/// `finish` failure: a plain message, or the sites to relax and retry.
#[derive(Debug)]
pub(crate) enum FinishErr {
    Msg(String),
    /// 4-byte conditional branches out of the ±4 KiB B-type range: retry
    /// those sites in the jal-trampoline form.
    BranchOverflow(Vec<usize>),
    /// c.bz / c.j sites out of their ±256 B / ±2 KiB windows: retry those
    /// sites full-width.
    WideCOverflow(Vec<usize>),
}

/// RVC compressed-register window (c.ld/c.sd/c.beqz/c.bnez and the
/// quadrant-1 RR arithmetic only accept x8..x15).
pub(crate) fn rvc_win(r: u8) -> bool {
    (8..=15).contains(&r)
}

impl Asm {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            rvc: false,
            sc1: SC1,
            sc2: SC2,
            branch_seq: 0,
            long_branch_ids: Default::default(),
            wide_c_ids: Default::default(),
            jal_seq: 0,
        }
    }
    pub(crate) fn new_rvc() -> Self {
        // RVC mode also moves the value pool into the compressed window and
        // the scratch registers out of its way (see Rv64Mode::ImcRemap).
        Self::new_rvc_scratch(SC1C, SC2C)
    }
    /// RVC assembler with an explicit scratch pair: ImcRemap keeps x30/x31
    /// (outside VSTACK_ORDER_RVC); Imc must take SC1I/SC2I because its pool
    /// is Im's VSTACK_REGS, which contains x30/x31 (BUG-024).
    pub(crate) fn new_rvc_scratch(sc1: u8, sc2: u8) -> Self {
        Self {
            buf: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            rvc: true,
            sc1,
            sc2,
            branch_seq: 0,
            long_branch_ids: Default::default(),
            wide_c_ids: Default::default(),
            jal_seq: 0,
        }
    }
    pub(crate) fn here(&self) -> usize {
        self.buf.len()
    }
    pub(crate) fn new_label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }
    pub(crate) fn place(&mut self, label: usize) {
        assert!(self.labels[label].is_none(), "label {label} placed twice");
        self.labels[label] = Some(self.buf.len());
    }
    pub(crate) fn word(&mut self, w: u32) {
        self.buf.extend_from_slice(&w.to_le_bytes());
    }

    /* ---- compressed emitters (consensus decode_profile_c forms) ---- */

    pub(crate) fn half(&mut self, h: u16) {
        self.buf.extend_from_slice(&h.to_le_bytes());
    }

    /// Quadrant-2 CI-ish: rd[11:7] full 5-bit, rs2[6:2] full 5-bit.
    /// c.mv(f3=4,b12=0,rs2!=0) / c.add(f3=4,b12=1,rs2!=0) / c.slli(f3=0,
    /// shamt in rs2) / c.jr(f3=4,b12=0,rs2=0) / c.jalr(f3=4,b12=1,rs2=0).
    pub(crate) fn c_q2(f3: u32, bit12: u32, rd: u8, rs2: u8) -> u16 {
        0b10 | ((rd as u16) << 7) | ((f3 as u16) << 13) | ((bit12 as u16) << 12) | ((rs2 as u16) << 2)
    }

    /// Quadrant-1 CI: rd[11:7], funct3[15:13], imm[5]=h12, imm[4:0]=h6:2.
    /// c.addi(f3=0) / c.li(f3=2) / c.lui(f3=3, rd!=2, imm!=0).
    pub(crate) fn c_q1_ci(f3: u32, rd: u8, imm: i32) -> u16 {
        let u = (imm as u16) & 0x3F;
        0b01
            | ((rd as u16) << 7)
            | ((f3 as u16) << 13)
            | ((u & 0x20) << 7)
            | ((u & 0x1F) << 2)
    }

    /// Quadrant-0 CL/CS: rd'/rs2' [4:2]+8, rs1' [9:7]+8,
    /// uimm = (h12:10) | (h6:5) << 3, byte offset = uimm * 8.
    pub(crate) fn c_q0_ldst(f3: u32, r01: u8, base: u8, imm: i32) -> u16 {
        let u = (imm >> 3) as u16; // 0..=31
        0b00
            | ((f3 as u16) << 13)
            | (((u & 7) as u16) << 10)
            | (((base & 7) as u16) << 7)
            | (((r01 & 7) as u16) << 2)
            | (((u >> 3) as u16) << 5)
    }

    /// c.j: 11-bit signed offset, bit-scattered per decode_profile_c.
    pub(crate) fn c_j(off: i32) -> u16 {
        let o = (off as u16) & 0xFFF; // 12-bit imm, sign bit is imm[11]
        0b01
            | (5 << 13)
            | ((o & 0x800) << 1)
            | (((o >> 4) & 1) << 11)
            | (((o >> 8) & 0x3) << 9)
            | (((o >> 10) & 1) << 8)
            | (((o >> 6) & 1) << 7)
            | (((o >> 7) & 1) << 6)
            | (((o >> 1) & 0x7) << 3)
            | (((o >> 5) & 1) << 2)
    }

    /// c.beqz/c.bnez: rs1' [9:7]+8, 9-bit signed offset (±256 B).
    pub(crate) fn c_bz(f3: u32, rs1: u8, off: i32) -> u16 {
        let o = (off as u16) & 0x1FF; // 9-bit imm, sign bit imm[8] -> h12
        0b01
            | ((f3 as u16) << 13)
            | ((o & 0x100) << 4)
            | (((o >> 3) & 0x3) << 10)
            | (((o >> 6) & 0x3) << 5)
            | (((o >> 1) & 0x3) << 3)
            | (((o >> 5) & 1) << 2)
            | (((rs1 & 7) as u16) << 7)
    }

    // branch funct3: BEQ=0 BNE=1 BLT=4 BGE=5 BLTU=6 BGEU=7
    pub(crate) fn branch_word(f3: u32, rs1: u8, rs2: u8, imm: u32) -> u32 {
        0b1100011
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (f3 << 12)
            | (((imm >> 12) & 1) << 31)
            | (((imm >> 5) & 0x3F) << 25)
            | (((imm >> 1) & 0xF) << 8)
            | (((imm >> 11) & 1) << 7)
    }

    pub(crate) fn jal_word(rd: u8, imm: u32) -> u32 {
        0b1101111
            | ((rd as u32) << 7)
            | (((imm >> 1) & 0x3FF) << 21)
            | (((imm >> 11) & 1) << 20)
            | (((imm >> 12) & 0xFF) << 12)
            | (((imm >> 20) & 1) << 31)
    }

    /// Conditional branch, B-type ±4 KiB. Targets beyond that range use a
    /// trampoline: the inverted conditional skips a full-range `jal`.
    /// Which sites need the trampoline is discovered by `finish` and fed
    /// back through `long_branch_ids` (iterative relaxation; lowering is
    /// deterministic so seq ids are stable across passes).
    pub(crate) fn branch(&mut self, f3: u32, rs1: u8, rs2: u8, label: usize) {
        let seq = self.branch_seq;
        self.branch_seq += 1;
        let long = self.long_branch_ids.contains(&seq);
        // Zero-compare branches compress to c.beqz/c.bnez when rs1 is in the
        // window. Backward offsets resolve immediately; forward references
        // are emitted optimistically as a 2-byte slot and relaxed to the
        // full B-type form by `finish` when they miss the ±256 B window
        // (which in turn relaxes to the trampoline past ±4 KiB).
        if self.rvc && !long && rs2 == ZERO && rvc_win(rs1) && (f3 == 0 || f3 == 1) {
            let cf3 = if f3 == 0 { 6 } else { 7 }; // c.beqz=6, c.bnez=7
            if !self.wide_c_ids.contains(&seq) {
                let at = self.here();
                if let Some(t) = self.labels[label] {
                    let off = t as i64 - at as i64;
                    if (-256..=254).contains(&off) && off & 1 == 0 {
                        self.half(Self::c_bz(cf3, rs1, off as i32));
                        return;
                    }
                    if !(-4096..=4094).contains(&off) || off & 1 != 0 {
                        // far backward: trampoline right away
                        self.word(Self::branch_word(f3 ^ 1, rs1, rs2, 8));
                        self.word(Self::jal_word(ZERO, (t - (at + 4)) as u32));
                        return;
                    }
                    self.word(Self::branch_word(f3, rs1, rs2, off as u32));
                    return;
                }
                self.half(Self::c_bz(cf3, rs1, 0));
                self.fixups.push(Fixup { at, label, kind: FixKind::CBranchZ, seq, aux: 0 });
                return;
            }
        }
        let at = self.here();
        if !long {
            if let Some(t) = self.labels[label] {
                // backward: direct when in range
                let off = t as i64 - at as i64;
                if (-4096..=4094).contains(&off) && off & 1 == 0 {
                    self.word(Self::branch_word(f3, rs1, rs2, off as u32));
                    return;
                }
            }
            self.word(Self::branch_word(f3, rs1, rs2, 0));
            self.fixups.push(Fixup { at, label, kind: FixKind::Branch, seq, aux: 0 });
            return;
        }
        // trampoline: inverted conditional skips the long jal
        self.word(Self::branch_word(f3 ^ 1, rs1, rs2, 8));
        if let Some(t) = self.labels[label] {
            let off = t - (at + 4);
            self.word(Self::jal_word(ZERO, off as u32));
        } else {
            self.word(Self::jal_word(ZERO, 0));
            self.fixups.push(Fixup { at: at + 4, label, kind: FixKind::Jal, seq, aux: 0 });
        }
    }
    pub(crate) fn beq(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(0, rs1, rs2, l);
    }
    pub(crate) fn bne(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(1, rs1, rs2, l);
    }
    pub(crate) fn bgeu(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(7, rs1, rs2, l);
    }
    pub(crate) fn bltu(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(6, rs1, rs2, l);
    }

    pub(crate) fn jal_(&mut self, rd: u8, label: usize) {
        let seq = self.jal_seq;
        self.jal_seq += 1;
        // c.j relaxation ids live in a namespace separate from branch ids
        // (both share `wide_c_ids`): a widened branch must never widen an
        // innocent c.j that happens to carry the same seq number — a real
        // size bug this exact sharing shipped for a while.
        let wseq = seq | JAL_C_ID_BIT;
        // rd=0 jumps compress to c.j (±2 KiB). Forward references are again
        // optimistic 2-byte slots relaxed by `finish`.
        if self.rvc && rd == ZERO && !self.wide_c_ids.contains(&wseq) {
            let at = self.here();
            if let Some(t) = self.labels[label] {
                let off = t as i64 - at as i64;
                if (-2048..=2046).contains(&off) && off & 1 == 0 {
                    self.half(Self::c_j(off as i32));
                    return;
                }
            } else {
                self.half(Self::c_j(0));
                self.fixups.push(Fixup { at, label, kind: FixKind::CJ, seq: wseq, aux: 0 });
                return;
            }
        }
        let at = self.here();
        self.word(Self::jal_word(rd, 0));
        self.fixups.push(Fixup { at, label, kind: FixKind::Jal, seq: usize::MAX, aux: 0 });
    }
    pub(crate) fn ret(&mut self) {
        if self.rvc {
            self.half(Self::c_q2(4, 0, RA, 0)); // c.jr ra (c.ret)
            return;
        }
        self.word(0b1100111u32 | ((RA as u32) << 15)); // jalr x0, ra, 0
    }
    pub(crate) fn ecall(&mut self) {
        self.word(0x0000_0073);
    }
    pub(crate) fn pcrel_addr(&mut self, rd: u8, label: usize) {
        let at = self.here();
        self.word(0b0010111u32 | ((rd as u32) << 7)); // auipc rd, 0 (patched)
        self.word(0b0010011u32 | ((rd as u32) << 7) | ((rd as u32) << 15)); // addi rd, rd, 0
        self.fixups.push(Fixup { at, label, kind: FixKind::PcRelHiLo, seq: usize::MAX, aux: 0 });
    }

    /// `addi rd, base, <lit - base>`: literal address relative to a
    /// function-wide pool-base register (2-byte c.addi when the offset is
    /// small), resolved by `finish` once the pool layout is known.
    pub(crate) fn addi_label_off(&mut self, rd: u8, base_reg: u8, base_label: usize, lit_label: usize) {
        let at = self.here();
        // addi rd, base, 0 (patched)
        self.word(0b0010011u32 | ((rd as u32) << 7) | ((base_reg as u32) << 15));
        self.fixups.push(Fixup {
            at,
            label: lit_label,
            kind: FixKind::PoolOff,
            seq: usize::MAX,
            aux: base_label,
        });
    }

    /// `ld rd, <lit - base>(base_reg)`: load a pooled 8-byte constant
    /// relative to the function-wide pool-base register.
    pub(crate) fn ld_label_off(&mut self, rd: u8, base_reg: u8, base_label: usize, lit_label: usize) {
        let at = self.here();
        // ld rd, 0(base_reg) (imm patched by finish)
        self.word(0b0000011u32 | ((rd as u32) << 7) | (3 << 12) | ((base_reg as u32) << 15));
        self.fixups.push(Fixup {
            at,
            label: lit_label,
            kind: FixKind::PoolLd,
            seq: usize::MAX,
            aux: base_label,
        });
    }

    pub(crate) fn r(&mut self, f7: u32, rs2: u8, rs1: u8, f3: u32, rd: u8, opcode: u32) {
        self.word(
            (f7 << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((rd as u32) << 7)
                | opcode,
        );
    }
    pub(crate) fn i(&mut self, imm: i32, rs1: u8, f3: u32, rd: u8, opcode: u32) {
        assert!(
            (-2048..=2047).contains(&imm),
            "rv64 proto: i-type imm {imm} out of range"
        );
        let imm = (imm as u32) & 0xFFF;
        self.word((imm << 20) | ((rs1 as u32) << 15) | (f3 << 12) | ((rd as u32) << 7) | opcode);
    }
    pub(crate) fn s(&mut self, imm: i32, rs2: u8, rs1: u8, f3: u32) {
        assert!(
            (-2048..=2047).contains(&imm),
            "rv64 proto: s-type imm {imm} out of range"
        );
        let imm = (imm as u32) & 0xFFF;
        self.word(
            ((imm >> 5) << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((imm & 0x1F) << 7)
                | 0b0100011,
        );
    }
    pub(crate) fn addi(&mut self, rd: u8, rs1: u8, imm: i32) {
        if self.rvc && rd != ZERO && (-32..=31).contains(&imm) {
            if rs1 == ZERO {
                self.half(Self::c_q1_ci(2, rd, imm)); // c.li
                return;
            }
            if rd == rs1 {
                self.half(Self::c_q1_ci(0, rd, imm)); // c.addi
                return;
            }
        }
        self.i(imm, rs1, 0, rd, 0b0010011);
    }
    pub(crate) fn ld(&mut self, rd: u8, rs1: u8, imm: i32) {
        if self.rvc && rvc_win(rd) && rvc_win(rs1) && (0..=248).contains(&imm) && imm % 8 == 0 {
            self.half(Self::c_q0_ldst(3, rd, rs1, imm)); // c.ld
            return;
        }
        self.i(imm, rs1, 3, rd, 0b0000011);
    }
    /// P3-KERNELS M2-d: 4-byte-stride rodata list loads (u32-packed
    /// tables). lwu zero-extends, so a table whose values all fit u32
    /// loads identically to the u64 form. No RVC compressed form exists
    /// for lwu (c.lw is RV32-only) — always the 4B base encoding.
    pub(crate) fn lwu(&mut self, rd: u8, rs1: u8, imm: i32) {
        self.i(imm, rs1, 6, rd, 0b0000011);
    }
    /// P3-IDIOMS: 2-byte BE-assembly groups (hacd200 :629 shape). lhu
    /// ZERO-extends (f3=5 — f3=1 would be the sign-extending lh!). No
    /// RVC compressed form (c.lh does not exist in RVC) — always 4B.
    pub(crate) fn lhu(&mut self, rd: u8, rs1: u8, imm: i32) {
        self.i(imm, rs1, 5, rd, 0b0000011);
    }
    pub(crate) fn sd(&mut self, rs2: u8, rs1: u8, imm: i32) {
        if self.rvc && rvc_win(rs2) && rvc_win(rs1) && (0..=248).contains(&imm) && imm % 8 == 0 {
            self.half(Self::c_q0_ldst(7, rs2, rs1, imm)); // c.sd
            return;
        }
        self.s(imm, rs2, rs1, 3);
    }
    pub(crate) fn lbu(&mut self, rd: u8, rs1: u8, imm: i32) {
        self.i(imm, rs1, 4, rd, 0b0000011);
    }
    pub(crate) fn sb(&mut self, rs2: u8, rs1: u8, imm: i32) {
        self.s(imm, rs2, rs1, 0);
    }
    /// B2 task 4: 2-byte element stores (u16-element list blocks). No RVC
    /// compressed form — always the 4B base encoding.
    pub(crate) fn sh(&mut self, rs2: u8, rs1: u8, imm: i32) {
        self.s(imm, rs2, rs1, 1);
    }
    /// B2 task 4: 4-byte element stores (u32-element list blocks). No RVC
    /// compressed form — always the 4B base encoding.
    pub(crate) fn sw(&mut self, rs2: u8, rs1: u8, imm: i32) {
        self.s(imm, rs2, rs1, 2);
    }
    pub(crate) fn mv(&mut self, rd: u8, rs: u8) {
        if self.rvc && rd != ZERO && rs != ZERO {
            self.half(Self::c_q2(4, 0, rd, rs)); // c.mv
            return;
        }
        self.addi(rd, rs, 0);
    }
    pub(crate) fn add(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if self.rvc && rd == rs1 && rd != ZERO && rs2 != ZERO {
            self.half(Self::c_q2(4, 1, rd, rs2)); // c.add
            return;
        }
        self.r(0, rs2, rs1, 0, rd, 0b0110011);
    }
    /// RR arithmetic compressible as c.sub/c.xor/c.or/c.and (quadrant-1
    /// funct3=4, bits[11:10]=11; `sel`: 00=sub 01=xor 10=or 11=and).
    pub(crate) fn rr_win(&mut self, sel: u32, rd: u8, rs1: u8, rs2: u8) -> bool {
        if self.rvc && rd == rs1 && rd != ZERO && rvc_win(rd) && rvc_win(rs2) {
            let h = 0b01u16
                | (4 << 13)
                | (0b11 << 10)
                | (((sel & 3) as u16) << 5)
                | (((rd & 7) as u16) << 7)
                | (((rs2 & 7) as u16) << 2);
            self.half(h);
            return true;
        }
        false
    }
    pub(crate) fn sub(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(0, rd, rs1, rs2) {
            self.r(0x20, rs2, rs1, 0, rd, 0b0110011);
        }
    }
    pub(crate) fn xor(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(1, rd, rs1, rs2) {
            self.r(0, rs2, rs1, 4, rd, 0b0110011);
        }
    }
    pub(crate) fn or_(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(2, rd, rs1, rs2) {
            self.r(0, rs2, rs1, 6, rd, 0b0110011);
        }
    }
    pub(crate) fn and_(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(3, rd, rs1, rs2) {
            self.r(0, rs2, rs1, 7, rd, 0b0110011);
        }
    }
    /// andi (c.andi when rd is in the window and imm fits imm6).
    pub(crate) fn andi(&mut self, rd: u8, rs1: u8, imm: i32) {
        if self.rvc && rd == rs1 && rvc_win(rd) && (-32..=31).contains(&imm) {
            // quadrant-1 CB: f3=4, bits[11:10]=10, imm6 scattered like c.andi
            let u = (imm as u16) & 0x3F;
            let h = 0b01u16
                | (4 << 13)
                | (0b10 << 10)
                | ((u & 0x20) << 7)
                | ((u & 0x1F) << 2)
                | (((rd & 7) as u16) << 7);
            self.half(h);
            return;
        }
        self.i(imm, rs1, 7, rd, 0b0010011);
    }

    pub(crate) fn slli_(&mut self, rd: u8, rs1: u8, shamt: u32) {
        if self.rvc && rd == rs1 && rd != ZERO && (1..=63).contains(&shamt) {
            self.half(Self::c_q2(0, shamt >> 5, rd, shamt as u8 & 0x1F)); // c.slli
            return;
        }
        self.word(
            (shamt << 20) | ((rs1 as u32) << 15) | (1 << 12) | ((rd as u32) << 7) | 0b0010011,
        );
    }
    /// srli by immediate (c.srli when rd is in the window).
    pub(crate) fn srli_(&mut self, rd: u8, rs1: u8, shamt: u32) {
        if self.rvc && rd == rs1 && rvc_win(rd) && (1..=63).contains(&shamt) {
            let h = 0b01u16
                | (4 << 13)
                | (0b00 << 10)
                | (((shamt & 0x1F) as u16) << 2)
                | (((shamt >> 5) as u16) << 12)
                | (((rd & 7) as u16) << 7);
            self.half(h); // c.srli
            return;
        }
        self.word(
            (shamt << 20) | ((rs1 as u32) << 15) | (5 << 12) | ((rd as u32) << 7) | 0b0010011,
        );
    }

    /// Load a constant into `rd` as a positive 64-bit value.
    pub(crate) fn li(&mut self, rd: u8, v: u64) {
        assert!(rd != ZERO, "li into x0");
        if v > 0xFFFF_FFFF {
            // 64-bit constant: hi half shifted up, or the low half in.
            let hi32 = v >> 32;
            let lo32 = v & 0xFFFF_FFFF;
            self.li_pair(rd, hi32);
            self.slli_(rd, rd, 32);
            if lo32 != 0 {
                self.li(self.sc2, lo32);
                self.r(0, self.sc2, rd, 6, rd, 0b0110011); // or rd, rd, sc2
            }
            return;
        }
        if v <= 2047 {
            self.addi(rd, ZERO, v as i32);
            return;
        }
        let hi = ((v + 0x800) >> 12) as u32;
        if hi < 0x8_0000 {
            self.li_pair(rd, v);
            return;
        }
        // v ≥ 0x7FFF_F800 (rounded hi would overflow 20 bits): two 16-bit
        // halves; the temp must differ from the destination.
        let lo16 = (v & 0xFFFF) as u64;
        let hi16 = v >> 16;
        let tmp = if rd == self.sc1 { self.sc2 } else { self.sc1 };
        self.li_pair(rd, hi16);
        self.slli_(rd, rd, 16);
        match lo16 {
            0 => {}
            0..=2047 => self.i(lo16 as i32, rd, 6, rd, 0b0010011), // ori
            _ => {
                self.li_pair(tmp, lo16);
                self.r(0, tmp, rd, 6, rd, 0b0110011); // or rd, rd, tmp
            }
        }
    }

    /// lui+addi pair for constants whose rounded hi fits 20 bits (< 2^31).
    pub(crate) fn li_pair(&mut self, rd: u8, v: u64) {
        let hi = ((v + 0x800) >> 12) as u32;
        let lo_i = v as i64 - ((hi as u64) << 12) as i64; // [-2048, 2047]
        if hi != 0 {
            if self.rvc && rd != ZERO && rd != SP && hi <= 31 {
                // c.lui (rd != x2, nonzero 6-bit signed hi; sext is positive
                // for hi <= 31 so the constant stays non-negative).
                self.half(Self::c_q1_ci(3, rd, hi as i32));
            } else {
                self.word((hi << 12) | ((rd as u32) << 7) | 0b0110111); // lui
            }
            if lo_i != 0 {
                self.addi(rd, rd, lo_i as i32);
            }
        } else {
            self.addi(rd, ZERO, lo_i as i32);
        }
    }

    pub(crate) fn finish(mut self) -> Result<Vec<u8>, FinishErr> {
        let mut overflow = Vec::new();
        let mut wide = Vec::new();
        for f in &self.fixups {
            // P3-GUARDTAIL GT-c: a never-placed label is a compiler bug —
            // surface it through the error channel instead of panicking
            // (doc 66 §1.4: user-reachable panics must not abort the
            // process; the message lands via the FinishErr::Msg arm at the
            // relaxation loop, which rethrows it with `rv64 lower:` intact)
            let target = self.labels[f.label].ok_or_else(|| {
                FinishErr::Msg(format!(
                    "rv64 lower: internal: fixup label {} never placed (compiler bug)",
                    f.label
                ))
            })?;
            let off = target as i64 - f.at as i64;
            // 2-byte slots first: relax to full-width on window overflow.
            match f.kind {
                FixKind::CBranchZ => {
                    if !(-256..=254).contains(&off) || off & 1 != 0 {
                        wide.push(f.seq);
                        continue;
                    }
                    let h = u16::from_le_bytes(self.buf[f.at..f.at + 2].try_into().unwrap());
                    let f3 = ((h >> 13) & 7) as u32;
                    let rs1 = (((h >> 7) & 7) + 8) as u8;
                    let patched = Self::c_bz(f3, rs1, off as i32);
                    self.buf[f.at..f.at + 2].copy_from_slice(&patched.to_le_bytes());
                    continue;
                }
                FixKind::CJ => {
                    if !(-2048..=2046).contains(&off) || off & 1 != 0 {
                        wide.push(f.seq);
                        continue;
                    }
                    let patched = Self::c_j(off as i32);
                    self.buf[f.at..f.at + 2].copy_from_slice(&patched.to_le_bytes());
                    continue;
                }
                _ => {}
            }
            let word = u32::from_le_bytes(self.buf[f.at..f.at + 4].try_into().unwrap());
            let patched = match f.kind {
                FixKind::Branch => {
                    if !(-4096..=4094).contains(&off) || off & 1 != 0 {
                        overflow.push(f.seq);
                        continue;
                    }
                    let imm = off as u32 & 0x1FFF;
                    word
                        | (((imm >> 12) & 1) << 31)
                        | (((imm >> 5) & 0x3F) << 25)
                        | (((imm >> 1) & 0xF) << 8)
                        | (((imm >> 11) & 1) << 7)
                }
                FixKind::Jal => {
                    if !(-0x10_0000..=0xF_FFFE).contains(&off) || off & 1 != 0 {
                        return Err(FinishErr::Msg(format!(
                            "rv64 proto: jal displacement {off} out of range (kernel body too large)"
                        )));
                    }
                    let imm = off as u32 & 0x1F_FFFF;
                    word
                        | (((imm >> 1) & 0x3FF) << 21)
                        | (((imm >> 11) & 1) << 20)
                        | (((imm >> 12) & 0xFF) << 12)
                        | (((imm >> 20) & 1) << 31)
                }
                FixKind::PcRelHiLo => {
                    let hi = ((off + 0x800) >> 12) as u32 & 0xFF_FFF;
                    let lo = off - ((hi as i64) << 12);
                    let rd = ((word >> 7) & 0x1F) as u32;
                    let w_lo = ((lo as u32 & 0xFFF) << 20) | (rd << 15) | (rd << 7) | 0b0010011;
                    self.buf[f.at + 4..f.at + 8].copy_from_slice(&w_lo.to_le_bytes());
                    (hi << 12) | (rd << 7) | 0b0010111
                }
                FixKind::PoolOff => {
                    // GT-c: same error-channel conversion as the fixup label
                    // above (user-reachable panic → Err, doc 66 §1.4)
                    let base = self.labels[f.aux].ok_or_else(|| {
                        FinishErr::Msg(
                            "rv64 lower: internal: pool base label never placed (compiler bug)"
                                .to_string(),
                        )
                    })?;
                    let rel = target as i64 - base as i64;
                    if !(-2048..=2047).contains(&rel) {
                        return Err(FinishErr::Msg(format!(
                            "rv64 proto: pool-base offset {rel} out of range"
                        )));
                    }
                    // clear ONLY the imm[11:0] field (bits 31:20); a mask
                    // reaching below bit 20 would wipe rs1/funct3 (a real bug
                    // this exact patch shipped for a few minutes)
                    word & !(0xFFFu32 << 20) | (((rel as u32) & 0xFFF) << 20)
                }
                FixKind::PoolLd => {
                    // GT-c: same error-channel conversion as PoolOff above
                    let base = self.labels[f.aux].ok_or_else(|| {
                        FinishErr::Msg(
                            "rv64 lower: internal: pool base label never placed (compiler bug)"
                                .to_string(),
                        )
                    })?;
                    let rel = target as i64 - base as i64;
                    if !(-2048..=2047).contains(&rel) {
                        return Err(FinishErr::Msg(format!(
                            "rv64 proto: pool-base constant offset {rel} out of range"
                        )));
                    }
                    word & !(0xFFFu32 << 20) | (((rel as u32) & 0xFFF) << 20)
                }
                FixKind::CBranchZ | FixKind::CJ => unreachable!("handled above"),
            };
            self.buf[f.at..f.at + 4].copy_from_slice(&patched.to_le_bytes());
        }
        if !wide.is_empty() {
            // Sizes only grow across passes, so escalating the c-slots first
            // is monotone and converges.
            return Err(FinishErr::WideCOverflow(wide));
        }
        if !overflow.is_empty() {
            return Err(FinishErr::BranchOverflow(overflow));
        }
        Ok(self.buf)
    }
}
