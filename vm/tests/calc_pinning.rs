//! Opcode-behavior pinning matrix (防漂移矩阵) for the calc RV64IMC interpreter.
//!
//! Every case carries hard-coded expected values that were cross-validated, at
//! authoring time, against THREE independent engines:
//!
//!   1. our calc interpreter (`vm::calc`) — the code under test,
//!   2. softcore-rv64 0.6.0 — automated Rust translation of the official Sail
//!      RISC-V executable spec (instruction-level golden oracle),
//!   3. ckb-vm 0.24.13 — production-proven RISC-V VM (full-program level).
//!
//! The driver asserts ours == ckb-vm (full register file + memory) and both ==
//! hard-coded pins on every case, and additionally ours == softcore for the
//! first (under-test) instruction wherever softcore covers it. Any future
//! change in our interpreter — or an upgrade of a pinned oracle — that moves
//! one of these values fails the suite, so opcode behavior cannot drift
//! silently.
//!
//! softcore coverage limits (measured empirically against 0.6.0, see
//! `doc/calcfunc-rv64im-research.md` §oracle):
//!   - MULH/MULHU/MULHSU panic at runtime (prelude BitVector 128-bit bug),
//!   - LOAD/STORE/AMO are `todo!()` (no memory subsystem),
//!   - its public decoder never emits C_* asts; we hand-build them from the
//!     same logical fields the case was written with (execute-side pinning).
//! Those cases run ours vs ckb-vm only.

#[cfg(feature = "execute")]
mod pin {
    use softcore_rv64::config;
    use softcore_rv64::prelude::{bv, BitVector};
    use softcore_rv64::raw::{self, ast};
    use softcore_rv64::{new_core, Core, Trap};
    use vm::calc::container::CalcContainer;
    use vm::calc::decode::{decode_at, BrKind, ImmAluOp, Inst, RegAluOp, ShiftKind};
    use vm::calc::interp::run_with_state;
    use vm::calc::verify::verify_container;

    const MEM_KIB: u16 = 8;
    const INPUT_MAX: u16 = 512;
    const OUTPUT_MAX: u16 = 512;

    fn align16(x: u64) -> u64 {
        (x + 15) & !15
    }

    /// 64-byte data pattern: `0x80 + i` (high bits set so lb/lh/lw sign
    /// extension is observable).
    fn pattern(i: usize) -> u8 {
        (0x80u32.wrapping_add(i as u32)) as u8
    }
    fn pattern_bytes(n: usize) -> Vec<u8> {
        (0..n).map(pattern).collect()
    }

    // ------------------------------------------------------------------
    // Case model
    // ------------------------------------------------------------------

    #[derive(Clone, Copy)]
    enum WordSpec {
        /// One 32-bit word.
        W(u32),
        /// One 16-bit compressed halfword.
        C(u16),
    }
    const W: fn(u32) -> WordSpec = WordSpec::W;
    const C: fn(u16) -> WordSpec = WordSpec::C;

    /// What softcore should execute for this case (the first under-test
    /// instruction only).
    enum ScAst {
        /// Map the first word through OUR decoder, then translate the decoded
        /// `Inst` into a softcore ast.
        Word,
        /// Hand-built compressed form; payloads are the logical fields the
        /// case was written with.
        CAddi(u8, i64),
        CLi(u8, i64),
        CLui(u8, i64),
        CMv(u8, u8),
        CAdd(u8, u8),
        CSub(u8, u8),
        CAnd(u8, u8),
        COr(u8, u8),
        CXor(u8, u8),
        CSubw(u8, u8),
        CAddw(u8, u8),
        CSlli(u8, u8),
        CSrli(u8, u8),
        CSrai(u8, u8),
        CAndi(u8, i64),
        CAddiw(u8, i64),
        CAddi16sp(i64),
        CAddi4spn(u8, i64),
        CJr(u8),
        CJalr(u8),
        CJ(i64),
        CBeqz(u8, i64),
        CBnez(u8, i64),
    }

    struct Case {
        name: &'static str,
        /// Register initializers, materialized as a `li` prologue in the
        /// container code (our interpreter takes no external register state,
        /// and ckb-vm runs the very same code bytes).
        init: Vec<(u8, u64)>,
        /// Register whose value is the data-section base (resolved after
        /// assembly; `u64::MAX` sentinel replaced by the builder).
        db_reg: Option<u8>,
        /// Test code (the FIRST word is the instruction under test).
        words: Vec<WordSpec>,
        /// Bytes written into the data section (at `data_base`).
        init_mem: Vec<u8>,
        /// softcore execution plan (`None` = softcore cannot cover it).
        sc: Option<ScAst>,
        /// The test instructions write x10 (a0): the sentinel-return
        /// validation uses a0 as the output length, so the builder must NOT
        /// append `li x10, 0` before the final ret.
        a0_test: bool,
        /// Hard-coded registers after the full program (ours & ckb).
        expect_regs: Vec<(u8, u64)>,
        /// Hard-coded memory bytes, offsets relative to `data_base`.
        expect_mem: Vec<(u64, u8)>,
        /// softcore register comparison override: (register, hard-coded
        /// value). Replaces the default ours-vs-softcore comparison for cases
        /// where the full program overwrites the register the single
        /// instruction wrote (e.g. the c.jalr link lands in x1 which the
        /// epilogue reloads).
        sc_reg_pin: Option<(u8, u64)>,
    }

    impl Case {
        fn new(name: &'static str) -> Case {
            Case {
                name,
                init: Vec::new(),
                db_reg: None,
                words: Vec::new(),
                init_mem: Vec::new(),
                sc: Some(ScAst::Word),
                a0_test: false,
                sc_reg_pin: None,
                expect_regs: Vec::new(),
                expect_mem: Vec::new(),
            }
        }
        fn reg(mut self, r: u8, v: u64) -> Case {
            self.init.push((r, v));
            self
        }
        fn regs(mut self, rs: &[(u8, u64)]) -> Case {
            self.init.extend_from_slice(rs);
            self
        }
        fn code(mut self, words: &[WordSpec]) -> Case {
            self.words.extend_from_slice(words);
            self
        }
        fn mem(self, n: usize) -> Case {
            Case { init_mem: pattern_bytes(n), ..self }
        }
        fn no_sc(self) -> Case {
            Case { sc: None, ..self }
        }
        fn keeps_a0(self) -> Case {
            Case { a0_test: true, ..self }
        }
        fn sc_pin(self, r: u8, v: u64) -> Case {
            Case { sc_reg_pin: Some((r, v)), ..self }
        }
        fn sc_c(self, k: ScAst) -> Case {
            Case { sc: Some(k), ..self }
        }
        fn expect(mut self, r: u8, v: u64) -> Case {
            self.expect_regs.push((r, v));
            self
        }
        fn expects(mut self, rs: &[(u8, u64)]) -> Case {
            self.expect_regs.extend_from_slice(rs);
            self
        }
        fn mem_at(mut self, off: u64, b: u8) -> Case {
            self.expect_mem.push((off, b));
            self
        }
        /// Register `r` receives the data-base address (for x5-relative
        /// addressing in load/store cases).
        fn with_db(mut self, r: u8) -> Case {
            self.init.retain(|(rr, _)| *rr != r);
            // the entry's value is substituted with the resolved data base by
            // the builder (db_reg marker); it must exist for the prologue
            self.init.push((r, 0));
            self.db_reg = Some(r);
            self
        }
    }

    // ------------------------------------------------------------------
    // Encoders (self-contained mirrors; any mis-encode fails the pins)
    // ------------------------------------------------------------------

    fn enc_i(op: u32, f3: u32, rd: u8, rs1: u8, imm: i32) -> u32 {
        op | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (f3 << 12)
            | (((imm as u32) & 0xFFF) << 20)
    }
    fn enc_u(op: u32, rd: u8, imm20: u32) -> u32 {
        op | ((rd as u32) << 7) | ((imm20 & 0xFFFFF) << 12)
    }
    fn enc_r(op: u32, f7: u32, f3: u32, rd: u8, rs1: u8, rs2: u8) -> u32 {
        op | ((rd as u32) << 7) | (f3 << 12) | ((rs1 as u32) << 15)
            | ((rs2 as u32) << 20) | (f7 << 25)
    }
    fn enc_s(f3: u32, rs1: u8, rs2: u8, imm: i32) -> u32 {
        let imm = imm as u32;
        0x23 | (f3 << 12) | ((rs1 as u32) << 15) | ((rs2 as u32) << 20)
            | ((imm & 0x1F) << 7) | ((imm >> 5) << 25)
    }
    fn enc_b(f3: u32, rs1: u8, rs2: u8, off: i32) -> u32 {
        let off = off as u32;
        0x63 | (f3 << 12) | ((rs1 as u32) << 15) | ((rs2 as u32) << 20)
            | (((off >> 11) & 1) << 7)
            | (((off >> 1) & 0xF) << 8)
            | (((off >> 5) & 0x3F) << 25)
            | (((off >> 12) & 1) << 31)
    }
    fn enc_jal(rd: u8, off: i32) -> u32 {
        let off = off as u32;
        0x6F | ((rd as u32) << 7)
            | (((off >> 12) & 0xFF) << 12)
            | (((off >> 11) & 1) << 20)
            | (((off >> 1) & 0x3FF) << 21)
            | (((off >> 20) & 1) << 31)
    }
    fn enc_jalr(rd: u8, rs1: u8, imm: i32) -> u32 {
        0x67 | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (((imm as u32) & 0xFFF) << 20)
    }
    /// `li rd, val` as 1–2 standard instructions (lui+addi with the inverted
    /// low-half trick), or a single addi/nop for small values.
    fn enc_li(rd: u8, val: u64) -> Vec<u32> {
        if val == 0 {
            return vec![enc_i(0x13, 0, rd, 0, 0)];
        }
        if val < 0x1000 {
            // registers like x2/x10 hold VM-set values at reset; always load
            // from x0
            return vec![enc_i(0x13, 0, rd, 0, val as i32)];
        }
        let lo = (val & 0xFFF) as i64;
        let lo = if lo >= 0x800 { lo - 0x1000 } else { lo };
        let hi = ((val as i64).wrapping_sub(lo) >> 12) & 0xFFFFF;
        // lui sign-extends its 20-bit immediate: the fast path is only valid
        // when the value's upper 32 bits match the sign of `hi`.
        let hi_sign_ext = (hi >> 19) & 1 == 1;
        let upper_ok = (val >> 32) == if hi_sign_ext { 0xFFFF_FFFF } else { 0 };
        if upper_ok {
            let mut v = vec![enc_u(0x37, rd, hi as u32)];
            if lo != 0 {
                v.push(enc_i(0x13, 0, rd, rd, lo as i32));
            }
            return v;
        }
        // General 64-bit constant: compose four 16-bit chunks with x31 as
        // scratch (no case pins x31). rd = ((((c3<<16)|c2)<<16|c1)<<16)|c0
        const SCR: u8 = 31;
        let mut v = vec![enc_i(0x13, 0, rd, 0, 0)];
        for sh in [48i32, 32, 16, 0] {
            let c = (val >> sh) & 0xFFFF;
            v.push(enc_i(0x13, 1, rd, rd, 16)); // slli rd, rd, 16
            v.push(enc_i(0x13, 0, SCR, 0, (c >> 6) as i32)); // addi scr, x0, c>>6
            v.push(enc_i(0x13, 1, SCR, SCR, 6)); // slli scr, scr, 6
            v.push(enc_i(0x13, 0, SCR, SCR, (c & 0x3F) as i32)); // addi scr, scr, c&63
            v.push(enc_r(0x33, 0, 6, rd, rd, SCR)); // or rd, rd, scr
        }
        v
    }
    fn ret() -> u32 {
        0x00008067 // jalr x0, 0(ra)
    }

    // RVC encoders. Any field mistake shows up as a pin/oracle failure, so
    // these are validated by construction as well.
    fn c_addi(rd: u8, imm: i64) -> u16 {
        (0b000u16 << 13) | ((((imm >> 5) & 1) as u16) << 12) | ((rd as u16) << 7)
            | (((imm & 0x1F) as u16) << 2) | 0b01
    }
    fn c_li(rd: u8, imm: i64) -> u16 {
        (0b010u16 << 13) | ((((imm >> 5) & 1) as u16) << 12) | ((rd as u16) << 7)
            | (((imm & 0x1F) as u16) << 2) | 0b01
    }
    fn c_lui(rd: u8, imm6: i64) -> u16 {
        (0b011u16 << 13) | ((((imm6 >> 5) & 1) as u16) << 12) | ((rd as u16) << 7)
            | (((imm6 & 0x1F) as u16) << 2) | 0b01
    }
    fn c_mv(rd: u8, rs2: u8) -> u16 {
        (0b100u16 << 13) | ((rd as u16) << 7) | ((rs2 as u16) << 2) | 0b10
    }
    fn c_add(rd: u8, rs2: u8) -> u16 {
        (0b100u16 << 13) | (1 << 12) | ((rd as u16) << 7) | ((rs2 as u16) << 2) | 0b10
    }
    fn c_alu(f2: u16, rd: u8, rs2: u8) -> u16 {
        // c.sub/c.xor/c.or/c.and: 100 0 11 rs1' f2 rs2' 01
        (0b100u16 << 13) | (0b11 << 10) | (((rd - 8) as u16) << 7) | (f2 << 5)
            | (((rs2 - 8) as u16) << 2) | 0b01
    }
    fn c_sub(rd: u8, rs2: u8) -> u16 {
        c_alu(0b00, rd, rs2)
    }
    fn c_xor(rd: u8, rs2: u8) -> u16 {
        c_alu(0b01, rd, rs2)
    }
    fn c_or(rd: u8, rs2: u8) -> u16 {
        c_alu(0b10, rd, rs2)
    }
    fn c_and(rd: u8, rs2: u8) -> u16 {
        c_alu(0b11, rd, rs2)
    }
    fn c_subw(rd: u8, rs2: u8) -> u16 {
        // 100 1 11 rs1' 00 rs2' 01
        (0b100u16 << 13) | (1 << 12) | (0b11 << 10) | (((rd - 8) as u16) << 7)
            | (((rs2 - 8) as u16) << 2) | 0b01
    }
    fn c_addw(rd: u8, rs2: u8) -> u16 {
        // 100 1 11 rs1' 01 rs2' 01
        (0b100u16 << 13) | (1 << 12) | (0b11 << 10) | (((rd - 8) as u16) << 7)
            | (1 << 5) | (((rs2 - 8) as u16) << 2) | 0b01
    }
    fn c_shift(f2: u16, rd: u8, shamt: u8) -> u16 {
        // c.srli/c.srai: 100 0 00|01 rs1' shamt 01 (shamt[5] in bit 12 on RV64)
        (0b100u16 << 13) | (((shamt >> 5) as u16 & 1) << 12) | (f2 << 10)
            | (((rd - 8) as u16) << 7) | (((shamt & 0x1F) as u16) << 2) | 0b01
    }
    fn c_srli(rd: u8, shamt: u8) -> u16 {
        c_shift(0b00, rd, shamt)
    }
    fn c_srai(rd: u8, shamt: u8) -> u16 {
        c_shift(0b01, rd, shamt)
    }
    fn c_slli(rd: u8, shamt: u8) -> u16 {
        // quadrant 2: 000 . shamt[5] rd shamt[4:0] . 10
        (0b000u16 << 13) | (((shamt >> 5) as u16 & 1) << 12) | ((rd as u16) << 7)
            | (((shamt & 0x1F) as u16) << 2) | 0b10
    }
    fn c_andi(rd: u8, imm: i64) -> u16 {
        // 100 0 10 rs1' imm[5] imm[4:0] 01
        (0b100u16 << 13) | (0b10 << 10) | (((rd - 8) as u16) << 7)
            | ((((imm >> 5) & 1) as u16) << 12) | (((imm & 0x1F) as u16) << 2) | 0b01
    }
    fn c_addiw(rd: u8, imm: i64) -> u16 {
        (0b001u16 << 13) | ((((imm >> 5) & 1) as u16) << 12) | ((rd as u16) << 7)
            | (((imm & 0x1F) as u16) << 2) | 0b01
    }
    fn c_addi16sp(imm: i64) -> u16 {
        // 011 . nzimm[9] . 00010 . nzimm[4] nzimm[6] nzimm[8:7] nzimm[5] . 01
        let f = ((imm / 16) as u64) & 0x3F; // nzimm[9:4]
        (0b011u16 << 13) | 0b01 | (2 << 7)
            | ((((f >> 5) & 1) as u16) << 12)
            | (((f & 1) as u16) << 6)
            | ((((f >> 2) & 1) as u16) << 5)
            | ((((f >> 4) & 1) as u16) << 4)
            | ((((f >> 3) & 1) as u16) << 3)
            | ((((f >> 1) & 1) as u16) << 2)
    }
    fn c_addi4spn(rd: u8, imm: i64) -> u16 {
        // 000 . nzuimm[5:4] nzuimm[9:6] nzuimm[2] nzuimm[3] rd' . 00, where
        // nzuimm field bit i is immediate bit i+2 (f = imm >> 2).
        let f = ((imm / 4) as u64) & 0xFF;
        (0b000u16 << 13)
            | ((((f >> 2) & 0b11) as u16) << 11) // imm[5:4]
            | ((((f >> 4) & 0xF) as u16) << 7)   // imm[9:6]
            | (((f & 1) as u16) << 6)            // imm[2]
            | ((((f >> 1) & 1) as u16) << 5)     // imm[3]
            | (((rd - 8) as u16) << 2)
    }
    fn c_j(off: i64) -> u16 {
        // 101 . imm[11] imm[4] imm[9:8] imm[10] imm[6] imm[7] imm[3:1] imm[5] . 01
        let o = off as u64;
        (0b101u16 << 13) | 0b01
            | ((((o >> 11) & 1) as u16) << 12)
            | ((((o >> 4) & 1) as u16) << 11)
            | ((((o >> 8) & 0b11) as u16) << 9)
            | ((((o >> 10) & 1) as u16) << 8)
            | ((((o >> 6) & 1) as u16) << 7)
            | ((((o >> 7) & 1) as u16) << 6)
            | ((((o >> 1) & 0b111) as u16) << 3)
            | ((((o >> 5) & 1) as u16) << 2)
    }
    fn c_jr(rs1: u8) -> u16 {
        (0b100u16 << 13) | ((rs1 as u16) << 7) | 0b10
    }
    fn c_jalr(rs1: u8) -> u16 {
        (0b100u16 << 13) | (1 << 12) | ((rs1 as u16) << 7) | 0b10
    }
    fn c_bez(f3: u16, rs1: u8, off: i64) -> u16 {
        // 110/111 . imm[8] imm[4:3] rs1' imm[7:6] imm[2:1] imm[5] . 01
        let o = off as u64;
        (f3 << 13) | 0b01
            | ((((o >> 8) & 1) as u16) << 12)
            | ((((o >> 3) & 0b11) as u16) << 10)
            | (((rs1 - 8) as u16) << 7)
            | ((((o >> 6) & 0b11) as u16) << 5)
            | ((((o >> 1) & 0b11) as u16) << 3)
            | ((((o >> 5) & 1) as u16) << 2)
    }
    fn c_beqz(rs1: u8, off: i64) -> u16 {
        c_bez(0b110, rs1, off)
    }
    fn c_bnez(rs1: u8, off: i64) -> u16 {
        c_bez(0b111, rs1, off)
    }
    fn c_lwsp(rd: u8, off: i64) -> u16 {
        // 010 . offset[5] rd offset[4:2] offset[7:6] . 10
        let o = off as u64;
        (0b010u16 << 13) | 0b10
            | ((((o >> 5) & 1) as u16) << 12)
            | ((rd as u16) << 7)
            | ((((o >> 2) & 0b111) as u16) << 4)
            | ((((o >> 6) & 0b11) as u16) << 2)
    }
    fn c_swsp(rs2: u8, off: i64) -> u16 {
        // 110 . offset[5:2] offset[7:6] rs2 . 10
        let o = off as u64;
        (0b110u16 << 13) | 0b10
            | ((((o >> 2) & 0xF) as u16) << 9)
            | ((((o >> 6) & 0b11) as u16) << 7)
            | ((rs2 as u16) << 2)
    }
    fn c_ldsp(rd: u8, off: i64) -> u16 {
        // 011 . offset[5] rd offset[4:3] offset[8:6] . 10
        let o = off as u64;
        (0b011u16 << 13) | 0b10
            | ((((o >> 5) & 1) as u16) << 12)
            | ((rd as u16) << 7)
            | ((((o >> 3) & 0b11) as u16) << 5)
            | ((((o >> 6) & 0b111) as u16) << 2)
    }
    fn c_sdsp(rs2: u8, off: i64) -> u16 {
        // 111 . offset[5:3] offset[8:6] rs2 . 10
        let o = off as u64;
        (0b111u16 << 13) | 0b10
            | ((((o >> 3) & 0b111) as u16) << 10)
            | ((((o >> 6) & 0b111) as u16) << 7)
            | ((rs2 as u16) << 2)
    }
    fn c_ls(f3: u16, rd: u8, rs1: u8, off: i64) -> u16 {
        // CL/CS formats are QUADRANT 0: c.lw/c.sw: 010|110 . imm[5:3] rs1'
        // imm[2] rd' . 00; c.ld/c.sd: 011|111 . imm[5:3] rs1' imm[7:6] rd' . 00
        let o = off as u64;
        (f3 << 13) | 0b00
            | ((((o >> 3) & 0b111) as u16) << 10)
            | (((rs1 - 8) as u16) << 7)
            | (if f3 & 1 == 0 {
                // lw/sw family: imm[2] at bit 6
                ((((o >> 2) & 1) as u16) << 6)
            } else {
                // ld/sd family: imm[7:6] at bits 6:5
                ((((o >> 6) & 0b11) as u16) << 5)
            })
            | (((rd - 8) as u16) << 2)
    }
    fn c_lw(rd: u8, rs1: u8, off: i64) -> u16 {
        c_ls(0b010, rd, rs1, off)
    }
    fn c_sw(rd: u8, rs1: u8, off: i64) -> u16 {
        c_ls(0b110, rd, rs1, off)
    }
    fn c_ld(rd: u8, rs1: u8, off: i64) -> u16 {
        c_ls(0b011, rd, rs1, off)
    }
    fn c_sd(rd: u8, rs1: u8, off: i64) -> u16 {
        c_ls(0b111, rd, rs1, off)
    }

    // ------------------------------------------------------------------
    // softcore adapter
    // ------------------------------------------------------------------

    fn sreg(n: u8) -> raw::regidx {
        raw::regidx::Regidx(BitVector::new(n as u64))
    }
    fn screg(n: u8) -> raw::cregidx {
        raw::cregidx::Cregidx(BitVector::new((n - 8) as u64))
    }
    fn smul(high: bool, s1: bool, s2: bool) -> raw::mul_op {
        raw::mul_op { high, signed_rs1: s1, signed_rs2: s2 }
    }

    /// Translate OUR decoded `Inst` into a softcore ast. Returns `None` for
    /// the measured softcore gaps (MULH family panic, loads/stores todo).
    fn inst_to_ast(inst: &Inst, pc: u64) -> Option<ast> {
        Some(match inst {
            // Zbb/Zbc ops are outside this instruction-level pinning set
            // (covered by the differential suite); map them to None.
            Inst::UnAlu { .. } | Inst::BitImm { .. } | Inst::Slliuw { .. } | Inst::Fence => {
                return None
            }
            Inst::Lui { rd, val } => {
                ast::UTYPE((bv::<20>(((val >> 12) as u64) & 0xFFFFF), sreg(*rd), raw::uop::LUI))
            }
            Inst::Auipc { rd, val } => {
                ast::UTYPE((bv::<20>(((val >> 12) as u64) & 0xFFFFF), sreg(*rd), raw::uop::AUIPC))
            }
            Inst::Jal { rd, target } => {
                ast::JAL((bv::<21>((target - pc as i64) as u64), sreg(*rd)))
            }
            Inst::Jalr { rd, rs1, imm } => {
                ast::JALR((bv::<12>((*imm as u64) & 0xFFF), sreg(*rs1), sreg(*rd)))
            }
            Inst::Branch { kind, rs1, rs2, target } => {
                let op = match kind {
                    BrKind::Eq => raw::bop::BEQ,
                    BrKind::Ne => raw::bop::BNE,
                    BrKind::Lt => raw::bop::BLT,
                    BrKind::Ge => raw::bop::BGE,
                    BrKind::Ltu => raw::bop::BLTU,
                    BrKind::Geu => raw::bop::BGEU,
                };
                ast::BTYPE((bv::<13>((target - pc as i64) as u64), sreg(*rs2), sreg(*rs1), op))
            }
            Inst::Load { .. } | Inst::Store { .. } | Inst::Ecall => return None,
            Inst::ImmAlu { op, rd, rs1, imm } => match op {
                ImmAluOp::Addiw => {
                    ast::ADDIW((bv::<12>((*imm as u64) & 0xFFF), sreg(*rs1), sreg(*rd)))
                }
                _ => {
                    let iop = match op {
                        ImmAluOp::Addi => raw::iop::ADDI,
                        ImmAluOp::Slti => raw::iop::SLTI,
                        ImmAluOp::Sltiu => raw::iop::SLTIU,
                        ImmAluOp::Xori => raw::iop::XORI,
                        ImmAluOp::Ori => raw::iop::ORI,
                        ImmAluOp::Andi => raw::iop::ANDI,
                        ImmAluOp::Addiw => unreachable!(),
                    };
                    ast::ITYPE((bv::<12>((*imm as u64) & 0xFFF), sreg(*rs1), sreg(*rd), iop))
                }
            },
            Inst::ShiftImm { kind, rd, rs1, shamt, word } => {
                if *word {
                    let sop = match kind {
                        ShiftKind::Ror => return None,
                        ShiftKind::Sll => raw::sopw::SLLIW,
                        ShiftKind::Srl => raw::sopw::SRLIW,
                        ShiftKind::Sra => raw::sopw::SRAIW,
                    };
                    ast::SHIFTIWOP((bv::<5>(*shamt as u64), sreg(*rs1), sreg(*rd), sop))
                } else {
                    let sop = match kind {
                        ShiftKind::Ror => return None,
                        ShiftKind::Sll => raw::sop::SLLI,
                        ShiftKind::Srl => raw::sop::SRLI,
                        ShiftKind::Sra => raw::sop::SRAI,
                    };
                    ast::SHIFTIOP((bv::<6>(*shamt as u64), sreg(*rs1), sreg(*rd), sop))
                }
            }
            Inst::RegAlu { op, rd, rs1, rs2 } => match op {
                // Zbb/Zbc: outside the softcore pinning set (differential
                // suite covers them).
                RegAluOp::Adduw
                | RegAluOp::Sh1add
                | RegAluOp::Sh2add
                | RegAluOp::Sh3add
                | RegAluOp::Sh1adduw
                | RegAluOp::Sh2adduw
                | RegAluOp::Sh3adduw
                | RegAluOp::Andn
                | RegAluOp::Orn
                | RegAluOp::Xnor
                | RegAluOp::Rol
                | RegAluOp::Ror
                | RegAluOp::Rolw
                | RegAluOp::Rorw
                | RegAluOp::Min
                | RegAluOp::Minu
                | RegAluOp::Max
                | RegAluOp::Maxu
                | RegAluOp::Bset
                | RegAluOp::Bclr
                | RegAluOp::Binv
                | RegAluOp::Bext
                | RegAluOp::Clmul
                | RegAluOp::Clmulh
                | RegAluOp::Clmulr => return None,
                RegAluOp::Add => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::ADD)),
                RegAluOp::Sub => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::SUB)),
                RegAluOp::Sll => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::SLL)),
                RegAluOp::Slt => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::SLT)),
                RegAluOp::Sltu => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::SLTU)),
                RegAluOp::Xor => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::XOR)),
                RegAluOp::Srl => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::SRL)),
                RegAluOp::Sra => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::SRA)),
                RegAluOp::Or => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::OR)),
                RegAluOp::And => ast::RTYPE((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::rop::AND)),
                RegAluOp::Mul => {
                    ast::MUL((sreg(*rs2), sreg(*rs1), sreg(*rd), smul(false, true, true)))
                }
                // MULH/MULHU/MULHSU panic inside softcore 0.6.0 (prelude
                // BitVector 128-bit bug) — excluded from the oracle set.
                RegAluOp::Mulh | RegAluOp::Mulhu | RegAluOp::Mulhsu => return None,
                RegAluOp::Div => ast::DIV((sreg(*rs2), sreg(*rs1), sreg(*rd), true)),
                RegAluOp::Divu => ast::DIV((sreg(*rs2), sreg(*rs1), sreg(*rd), false)),
                RegAluOp::Rem => ast::REM((sreg(*rs2), sreg(*rs1), sreg(*rd), true)),
                RegAluOp::Remu => ast::REM((sreg(*rs2), sreg(*rs1), sreg(*rd), false)),
                RegAluOp::Addw => ast::RTYPEW((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::ropw::ADDW)),
                RegAluOp::Subw => ast::RTYPEW((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::ropw::SUBW)),
                RegAluOp::Sllw => ast::RTYPEW((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::ropw::SLLW)),
                RegAluOp::Srlw => ast::RTYPEW((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::ropw::SRLW)),
                RegAluOp::Sraw => ast::RTYPEW((sreg(*rs2), sreg(*rs1), sreg(*rd), raw::ropw::SRAW)),
                RegAluOp::Mulw => ast::MULW((sreg(*rs2), sreg(*rs1), sreg(*rd))),
                RegAluOp::Divw => ast::DIVW((sreg(*rs2), sreg(*rs1), sreg(*rd), true)),
                RegAluOp::Divuw => ast::DIVW((sreg(*rs2), sreg(*rs1), sreg(*rd), false)),
                RegAluOp::Remw => ast::REMW((sreg(*rs2), sreg(*rs1), sreg(*rd), true)),
                RegAluOp::Remuw => ast::REMW((sreg(*rs2), sreg(*rs1), sreg(*rd), false)),
            },
        })
    }

    fn c_ast_to_softcore(k: &ScAst) -> ast {
        match k {
            ScAst::CAddi(rd, imm) => ast::C_ADDI((bv::<6>(*imm as u64), sreg(*rd))),
            ScAst::CLi(rd, imm) => ast::C_LI((bv::<6>(*imm as u64), sreg(*rd))),
            ScAst::CLui(rd, imm6) => ast::C_LUI((bv::<6>(*imm6 as u64), sreg(*rd))),
            ScAst::CMv(rd, rs2) => ast::C_MV((sreg(*rd), sreg(*rs2))),
            ScAst::CAdd(rd, rs2) => ast::C_ADD((sreg(*rd), sreg(*rs2))),
            ScAst::CSub(rd, rs2) => ast::C_SUB((screg(*rd), screg(*rs2))),
            ScAst::CAnd(rd, rs2) => ast::C_AND((screg(*rd), screg(*rs2))),
            ScAst::COr(rd, rs2) => ast::C_OR((screg(*rd), screg(*rs2))),
            ScAst::CXor(rd, rs2) => ast::C_XOR((screg(*rd), screg(*rs2))),
            ScAst::CSubw(rd, rs2) => ast::C_SUBW((screg(*rd), screg(*rs2))),
            ScAst::CAddw(rd, rs2) => ast::C_ADDW((screg(*rd), screg(*rs2))),
            ScAst::CSlli(rd, sh) => ast::C_SLLI((bv::<6>(*sh as u64), sreg(*rd))),
            ScAst::CSrli(rd, sh) => ast::C_SRLI((bv::<6>(*sh as u64), screg(*rd))),
            ScAst::CSrai(rd, sh) => ast::C_SRAI((bv::<6>(*sh as u64), screg(*rd))),
            ScAst::CAndi(rd, imm) => ast::C_ANDI((bv::<6>(*imm as u64), screg(*rd))),
            ScAst::CAddiw(rd, imm) => ast::C_ADDIW((bv::<6>(*imm as u64), sreg(*rd))),
            ScAst::CAddi16sp(imm) => ast::C_ADDI16SP(bv::<6>(((imm / 16) as u64) & 0x3F)),
            ScAst::CAddi4spn(rd, imm) => {
                ast::C_ADDI4SPN((screg(*rd), bv::<8>(((imm / 4) as u64) & 0xFF)))
            }
            ScAst::CJr(rs1) => ast::C_JR(sreg(*rs1)),
            ScAst::CJalr(rs1) => ast::C_JALR(sreg(*rs1)),
            ScAst::CJ(off) => ast::C_J(bv::<11>((off / 2) as u64)),
            ScAst::CBeqz(rs1, off) => {
                ast::C_BEQZ((bv::<8>(((off / 2) as u64) & 0xFF), screg(*rs1)))
            }
            ScAst::CBnez(rs1, off) => {
                ast::C_BNEZ((bv::<8>(((off / 2) as u64) & 0xFF), screg(*rs1)))
            }
            ScAst::Word => unreachable!("Word is handled by inst_to_ast"),
        }
    }

    /// Which register a softcore-executed form writes (for the oracle
    /// comparison); `None` = control-flow only (or writes x0).
    fn sc_written(k: &ScAst, inst: Option<&Inst>) -> Option<u8> {
        match k {
            ScAst::Word => match inst? {
                // Zbb/Zbc forms never reach the softcore oracle (mapped to
                // None above); treat any unexpected hit as unwritten.
                Inst::UnAlu { .. } | Inst::BitImm { .. } | Inst::Slliuw { .. } | Inst::Fence => {
                    return None
                }
                Inst::Lui { rd, .. }
                | Inst::Auipc { rd, .. }
                | Inst::Jal { rd, .. }
                | Inst::Jalr { rd, .. }
                | Inst::Load { rd, .. }
                | Inst::ImmAlu { rd, .. }
                | Inst::ShiftImm { rd, .. }
                | Inst::RegAlu { rd, .. } => Some(*rd),
                Inst::Branch { .. } | Inst::Store { .. } | Inst::Ecall => None,
            },
            ScAst::CAddi(rd, _)
            | ScAst::CLi(rd, _)
            | ScAst::CLui(rd, _)
            | ScAst::CMv(rd, _)
            | ScAst::CAdd(rd, _)
            | ScAst::CSub(rd, _)
            | ScAst::CAnd(rd, _)
            | ScAst::COr(rd, _)
            | ScAst::CXor(rd, _)
            | ScAst::CSubw(rd, _)
            | ScAst::CAddw(rd, _)
            | ScAst::CSlli(rd, _)
            | ScAst::CSrli(rd, _)
            | ScAst::CSrai(rd, _)
            | ScAst::CAndi(rd, _)
            | ScAst::CAddiw(rd, _)
            | ScAst::CAddi4spn(rd, _) => Some(*rd),
            ScAst::CAddi16sp(_) => Some(2),
            ScAst::CJalr(_) => Some(1),
            ScAst::CJr(_) | ScAst::CJ(_) | ScAst::CBeqz(..) | ScAst::CBnez(..) => None,
        }
    }

    /// Execute ONE instruction on softcore at `pc` with the given register
    /// state; returns the full register file and the next PC.
    ///
    /// The Sail step loop maintains `nextPC = PC + inst_len` BEFORE executing
    /// (JAL's link register reads `get_next_pc()`), so the caller must
    /// pre-set it exactly like the step loop would.
    fn softcore_exec(a: ast, init: &[(u8, u64)], pc: u64, len: u64) -> ([u64; 32], u64) {
        let mut core: Core = new_core(config::U74);
        core.reset();
        for (r, v) in init {
            core.set(sreg(*r), *v);
        }
        core.PC = bv(pc);
        core.nextPC = bv(pc + len);
        let t = core.execute(a);
        assert!(matches!(t, Trap::None), "softcore trapped: {t:?}");
        let mut regs = [0u64; 32];
        for (i, r) in regs.iter_mut().enumerate() {
            *r = core.get(sreg(i as u8));
        }
        (regs, core.nextPC.bits())
    }

    // ------------------------------------------------------------------
    // drivers
    // ------------------------------------------------------------------

    struct Built {
        container: CalcContainer,
        test_pc: u64,
        init: Vec<(u8, u64)>,
        data_base: u64,
    }

    fn assemble(init: &[(u8, u64)], words: &[WordSpec], a0_test: bool) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        for (r, v) in init {
            for w in enc_li(*r, *v) {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        for spec in words {
            match *spec {
                WordSpec::W(v) => bytes.extend_from_slice(&v.to_le_bytes()),
                WordSpec::C(h) => bytes.extend_from_slice(&(h).to_le_bytes()),
            }
        }
        if !a0_test {
            // sentinel-return validates a0 as the output length
            for w in enc_li(10, 0) {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        // Cases whose flow clobbers x1 (jal/c.jalr links) must still return:
        // snapshot x1 into x30 (unpinned scratch), reload the sentinel, ret.
        // The c.jalr case pins x30 to observe the written link value.
        bytes.extend_from_slice(&enc_i(0x13, 0, 30, 1, 0).to_le_bytes()); // addi x30, x1, 0
        for w in enc_li(1, vm::calc::CALC_RETURN_SENTINEL) {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        bytes.extend_from_slice(&ret().to_le_bytes());
        bytes
    }

    fn build(case: &Case) -> Built {
        // The db register needs the data base, which depends only on the code
        // length; iterate to the fixed point (always ≤ 2 rounds here).
        let mut db = 0u64;
        let (bytes, db) = loop {
            let eff: Vec<(u8, u64)> = case
                .init
                .iter()
                .map(|(r, v)| {
                    if Some(*r) == case.db_reg {
                        (*r, db)
                    } else {
                        (*r, *v)
                    }
                })
                .collect();
            let bytes = assemble(&eff, &case.words, case.a0_test);
            let ndb = align16(bytes.len() as u64);
            if ndb == db {
                break (bytes, db);
            }
            db = ndb;
        };
        let container = CalcContainer {
            abi: 1,
            mem_kib: MEM_KIB,
            entry: 0,
            code: bytes,
            rodata: vec![],
            init: case.init_mem.clone(),
            input_max: INPUT_MAX,
            output_max: OUTPUT_MAX,
        };
        // The first under-test word sits right after the prologue.
        let mut prologue_bytes = 0usize;
        for (r, v) in &case.init {
            let v = if Some(*r) == case.db_reg { db } else { *v };
            prologue_bytes += enc_li(*r, v).len() * 4;
        }
        let module = verify_container(&container).expect(case.name);
        assert_eq!(module.layout_snapshot().data_base, db, "{}: data base mismatch", case.name);
        Built {
            container,
            test_pc: prologue_bytes as u64,
            init: case
                .init
                .iter()
                .map(|(r, v)| (*r, if Some(*r) == case.db_reg { db } else { *v }))
                .collect(),
            data_base: db,
        }
    }

    fn run_ours(case: &Case, b: &Built) -> ([u64; 32], Vec<u8>) {
        let module = verify_container(&b.container).expect("verify");
        let (res, regs, mem, _trace) = run_with_state(&module, &b.container, &[], 100_000);
        match res {
            Ok(_) => {}
            Err((a, cyc)) => panic!("{}: ours failed: {a:?} at {cyc}\ncode: {:x?}", case.name, b.container.code),
        }
        (regs, mem)
    }

    fn run_ckb(b: &Built) -> ([u64; 32], Vec<u8>) {
        use ckb_vm::{CoreMachine, DefaultCoreMachine, DefaultMachineBuilder, FlatMemory, Memory};
        type Ck = DefaultCoreMachine<u64, FlatMemory<u64>>;
        let lay = verify_container(&b.container).expect("verify").layout_snapshot();
        let core =
            Ck::new_with_memory(ckb_vm::ISA_IMC, ckb_vm::machine::VERSION2, u64::MAX, lay.mem_len as usize);
        let mut machine = DefaultMachineBuilder::new(core).build();
        {
            let m = machine.memory_mut();
            m.store_bytes(0, &b.container.code).unwrap();
            m.store_bytes(lay.rodata_base, &b.container.rodata).unwrap();
            m.store_bytes(lay.data_base, &b.container.init).unwrap();
        }
        machine.set_register(10, lay.input_base);
        machine.set_register(11, 0);
        machine.set_register(12, lay.output_base);
        machine.set_register(13, lay.output_max);
        machine.set_register(2, lay.stack_top);
        machine.set_register(1, vm::calc::CALC_RETURN_SENTINEL);
        machine.update_pc(0);
        machine.commit_pc();
        let mut decoder = ckb_vm::decoder::build_decoder::<u64>(machine.isa(), machine.version());
        let sentinel = vm::calc::CALC_RETURN_SENTINEL;
        let mut steps = 0u64;
        loop {
            if *machine.pc() == sentinel {
                break;
            }
            steps += 1;
            assert!(steps < 100_000, "ckb step bound exceeded (runaway program)");
            machine.step(&mut decoder).expect("ckb step");
        }
        let mut regs = [0u64; 32];
        regs.copy_from_slice(machine.registers());
        let mut mem = vec![0u8; lay.mem_len as usize];
        let bs = machine.memory_mut().load_bytes(0, lay.mem_len).expect("ckb mem");
        mem.copy_from_slice(&bs);
        (regs, mem)
    }

    fn check_case(case: Case) {
        let b = build(&case);
        if std::env::var("PIN_DEBUG").is_ok() {
            eprintln!("case {}: code={:x?}", case.name, b.container.code);
        }
        let (regs_o, mem_o) = run_ours(&case, &b);
        let (regs_c, mem_c) = run_ckb(&b);

        // ours == ckb on the FULL register file and memory
        assert_eq!(regs_o, regs_c, "{}: ours vs ckb-vm register divergence", case.name);
        assert_eq!(&mem_o[..], &mem_c[..], "{}: ours vs ckb-vm memory divergence", case.name);

        // hard-coded pins
        for (r, v) in &case.expect_regs {
            assert_eq!(
                regs_o[*r as usize], *v,
                "{}: x{} = {:#x}, expected {:#x} (pin drifted)",
                case.name, r, regs_o[*r as usize], v
            );
        }
        for (off, byte) in &case.expect_mem {
            let a = b.data_base + off;
            assert_eq!(
                mem_o[a as usize], *byte,
                "{}: mem[{:#x}] = {:#x}, expected {:#x} (pin drifted)",
                case.name, a, mem_o[a as usize], byte
            );
        }

        // softcore golden check: execute the first under-test instruction at
        // its real pc, compare the register it writes against ours and the
        // next pc against the static expectation.
        if let Some(sc) = &case.sc {
            let bytes = &b.container.code;
            let decoded = decode_at(bytes, b.test_pc as usize).expect(case.name);
            let (a, next) = match sc {
                ScAst::Word => {
                    let a = inst_to_ast(&decoded.0, b.test_pc)
                        .expect("case must call no_sc() for instructions softcore cannot execute");
                    let next = match decoded.0 {
                        Inst::Jal { target, .. } => Some(target as u64),
                        // Branch: taken/not-taken is data-dependent and
                        // softcore runs the real condition; only the register
                        // pins are compared for branch cases.
                        Inst::Branch { .. } => None,
                        Inst::Jalr { .. } => None, // dynamic target
                        _ => Some(b.test_pc + decoded.1 as u64),
                    };
                    (a, next)
                }
                k => {
                    let next = match k {
                        ScAst::CJr(_) | ScAst::CJalr(_) => {
                            // dynamic: the register value from the prologue
                            let rs1 = match k {
                                ScAst::CJr(r) | ScAst::CJalr(r) => *r,
                                _ => unreachable!(),
                            };
                            b.init
                                .iter()
                                .find(|(r, _)| *r == rs1)
                                .map(|(_, v)| *v)
                        }
                        ScAst::CJ(off) => Some((b.test_pc as i64 + off) as u64),
                        // CBeqz/CBnez: data-dependent, like BTYPE
                        ScAst::CBeqz(..) | ScAst::CBnez(..) => None,
                        _ => Some(b.test_pc + 2),
                    };
                    (c_ast_to_softcore(&k), next)
                }
            };
            let ilen = match sc {
                ScAst::Word => decoded.1 as u64,
                _ => 2,
            };
            let (regs_s, next_pc) = softcore_exec(a, &b.init, b.test_pc, ilen);
            if let Some((r, v)) = case.sc_reg_pin {
                assert_eq!(
                    regs_s[r as usize], v,
                    "{}: softcore x{} = {:#x}, pin {:#x}",
                    case.name, r, regs_s[r as usize], v
                );
            } else if let Some(rd) = sc_written(sc, Some(&decoded.0)) {
                if rd != 0 {
                    assert_eq!(
                        regs_s[rd as usize], regs_o[rd as usize],
                        "{}: softcore x{} = {:#x}, ours = {:#x} (oracle divergence)",
                        case.name, rd, regs_s[rd as usize], regs_o[rd as usize]
                    );
                }
            }
            if let Some(np) = next {
                assert_eq!(
                    next_pc, np,
                    "{}: softcore next pc {:#x}, expected {:#x}",
                    case.name, next_pc, np
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // the matrix
    // ------------------------------------------------------------------

    const NEG16: u64 = 0xFFFF_FFFF_FFFF_FFF0; // -16
    const MIN: u64 = 0x8000_0000_0000_0000;

    fn alu_regs() -> Vec<(u8, u64)> {
        vec![(5, NEG16), (6, 16), (7, MIN)]
    }

    #[test]
    fn pin_imm_alu() {
        let cases = vec![
            Case::new("addi -16+15").regs(&alu_regs()).code(&[W(enc_i(0x13, 0, 8, 5, 15))]).expect(8, u64::MAX),
            Case::new("addi MIN-1 wrap").regs(&alu_regs()).code(&[W(enc_i(0x13, 0, 8, 7, -1))]).expect(8, 0x7FFF_FFFF_FFFF_FFFF),
            Case::new("slti -16<0").regs(&alu_regs()).code(&[W(enc_i(0x13, 2, 8, 5, 0))]).expect(8, 1),
            Case::new("slti 16<0").regs(&alu_regs()).code(&[W(enc_i(0x13, 2, 8, 6, 0))]).expect(8, 0),
            Case::new("sltiu -16<-1").regs(&alu_regs()).code(&[W(enc_i(0x13, 3, 8, 5, -1))]).expect(8, 1),
            Case::new("xori -16^-1").regs(&alu_regs()).code(&[W(enc_i(0x13, 4, 8, 5, -1))]).expect(8, 0xF),
            Case::new("ori 0|0x7ff").regs(&alu_regs()).code(&[W(enc_i(0x13, 6, 8, 0, 0x7FF))]).expect(8, 0x7FF),
            Case::new("andi -16&0xff").regs(&alu_regs()).code(&[W(enc_i(0x13, 7, 8, 5, 0xFF))]).expect(8, 0xF0),
            Case::new("addiw -16+16").regs(&alu_regs()).code(&[W(enc_i(0x1B, 0, 8, 5, 16))]).expect(8, 0),
            Case::new("addiw 0+2047").regs(&alu_regs()).code(&[W(enc_i(0x1B, 0, 8, 0, 2047))]).expect(8, 2047),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_reg_alu() {
        let cases = vec![
            Case::new("add").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 0, 8, 5, 6))]).expect(8, 0),
            Case::new("sub 0-(-16)").regs(&alu_regs()).code(&[W(enc_r(0x33, 0x20, 0, 8, 0, 5))]).expect(8, 16),
            Case::new("slt").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 2, 8, 5, 6))]).expect(8, 1),
            Case::new("sltu huge>16").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 3, 8, 5, 6))]).expect(8, 0),
            Case::new("sltu 16<huge").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 3, 8, 6, 5))]).expect(8, 1),
            Case::new("and").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 7, 8, 5, 6))]).expect(8, 16),
            Case::new("or").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 6, 8, 5, 6))]).expect(8, NEG16),
            Case::new("xor").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 4, 8, 5, 6))]).expect(8, 0xFFFF_FFFF_FFFF_FFE0),
            Case::new("sll").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 1, 8, 6, 6))]).expect(8, 0x10_0000),
            Case::new("srl").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 5, 8, 5, 6))]).expect(8, 0xFFFF_FFFF_FFFF),
            Case::new("sra").regs(&alu_regs()).code(&[W(enc_r(0x33, 0x20, 5, 8, 5, 6))]).expect(8, u64::MAX),
            // shift amount is masked to 6 bits: MIN & 63 == 0
            Case::new("sll shamt masked to 0").regs(&alu_regs()).code(&[W(enc_r(0x33, 0, 1, 8, 6, 7))]).expect(8, 16),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_shift_imm() {
        let cases = vec![
            Case::new("slli 16<<3").regs(&alu_regs()).code(&[W(enc_i(0x13, 1, 8, 6, 3))]).expect(8, 128),
            Case::new("srli -16>>4").regs(&alu_regs()).code(&[W(enc_i(0x13, 5, 8, 5, 4))]).expect(8, 0x0FFF_FFFF_FFFF_FFFF),
            Case::new("srai -16>>4").regs(&alu_regs()).code(&[W(enc_i(0x13, 5, 8, 5, 0x400 | 4))]).expect(8, u64::MAX),
            Case::new("slliw -16<<1").regs(&alu_regs()).code(&[W(enc_i(0x1B, 1, 8, 5, 1))]).expect(8, 0xFFFF_FFFF_FFFF_FFE0),
            Case::new("srliw -16>>4").regs(&alu_regs()).code(&[W(enc_i(0x1B, 5, 8, 5, 4))]).expect(8, 0x0000_0000_0FFF_FFFF),
            Case::new("sraiw -16>>4").regs(&alu_regs()).code(&[W(enc_i(0x1B, 5, 8, 5, 0x400 | 4))]).expect(8, u64::MAX),
            // MIN << 63 shifts the only set bit out
            Case::new("slli MIN<<63").regs(&alu_regs()).code(&[W(enc_i(0x13, 1, 8, 7, 63))]).expect(8, 0),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_utype() {
        let cases = vec![
            Case::new("lui sign-extend").code(&[W(enc_u(0x37, 8, 0xDEAD0))]).expect(8, 0xFFFF_FFFF_DEAD_0000),
            Case::new("lui positive").code(&[W(enc_u(0x37, 8, 0x12345))]).expect(8, 0x1_2345_000),
            // one 4-byte prologue word (li x9) puts the auipc at pc=4
            Case::new("auipc pc+1<<12").reg(9, 1).code(&[W(enc_u(0x17, 8, 1))]).expect(8, 4 + 0x1000),
            Case::new("auipc negative imm").reg(9, 1).code(&[W(enc_u(0x17, 8, 0xFFFFF))]).expect(8, (4i64 - 4096) as u64),
        ];
        for c in cases {
            check_case(c);
        }
    }

    // ---- multiply / divide -------------------------------------------

    #[test]
    fn pin_mul() {
        let r = [(5u8, 7), (6, 13)];
        let big = [(5u8, 0x1234_5678_9ABC_DEF0), (6, 0x0FED_CBA9_8765_4321)];
        let cases = vec![
            Case::new("mul 7*13").regs(&r).code(&[W(enc_r(0x33, 1, 0, 8, 5, 6))]).expect(8, 91),
            Case::new("mulw 7*13").regs(&r).code(&[W(enc_r(0x3B, 1, 0, 8, 5, 6))]).expect(8, 91),
            Case::new("mul low big")
                .regs(&big)
                .code(&[W(enc_r(0x33, 1, 0, 8, 5, 6))])
                .expect(8, 0x1234_5678_9ABC_DEF0u64.wrapping_mul(0x0FED_CBA9_8765_4321)),
        ];
        for c in cases {
            check_case(c);
        }
    }

    /// MULH/MULHU/MULHSU: softcore panics (128-bit prelude bug) — ckb only.
    #[test]
    fn pin_mulh_ckb_only() {
        let neg = [(5u8, (-7i64) as u64), (6, 13)];
        let huge = [(5u8, 0xFFFF_FFFF_FFFF_FFF9), (6, 3)];
        let cases = vec![
            Case::new("mulh -7*13").no_sc().regs(&neg).code(&[W(enc_r(0x33, 1, 1, 8, 5, 6))]).expect(8, u64::MAX),
            Case::new("mulhu huge*3").no_sc().regs(&huge).code(&[W(enc_r(0x33, 1, 3, 8, 5, 6))]).expect(8, 2),
            Case::new("mulhsu -7*3").no_sc().regs(&neg).code(&[W(enc_r(0x33, 1, 2, 8, 5, 6))]).expect(8, u64::MAX),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_div_rem_special() {
        let cases = vec![
            // x/0 family
            Case::new("div 7/0").reg(5, 7).code(&[W(enc_r(0x33, 1, 4, 8, 5, 0))]).expect(8, u64::MAX),
            Case::new("rem 7%0").reg(5, 7).code(&[W(enc_r(0x33, 1, 6, 8, 5, 0))]).expect(8, 7),
            Case::new("divu 7/0").reg(5, 7).code(&[W(enc_r(0x33, 1, 5, 8, 5, 0))]).expect(8, u64::MAX),
            Case::new("remu 7%0").reg(5, 7).code(&[W(enc_r(0x33, 1, 7, 8, 5, 0))]).expect(8, 7),
            // overflow
            Case::new("div MIN/-1").reg(5, MIN).reg(6, (-1i64) as u64).code(&[W(enc_r(0x33, 1, 4, 8, 5, 6))]).expect(8, MIN),
            Case::new("rem MIN%-1").reg(5, MIN).reg(6, (-1i64) as u64).code(&[W(enc_r(0x33, 1, 6, 8, 5, 6))]).expect(8, 0),
            // W forms divide-by-zero (sign-extension semantics — the P0 fix)
            Case::new("divw 7/0").reg(5, 7).code(&[W(enc_r(0x3B, 1, 4, 8, 5, 0))]).expect(8, u64::MAX),
            Case::new("remw 7%0").reg(5, 7).code(&[W(enc_r(0x3B, 1, 6, 8, 5, 0))]).expect(8, 7),
            Case::new("divuw 7/0").reg(5, 7).code(&[W(enc_r(0x3B, 1, 5, 8, 5, 0))]).expect(8, u64::MAX),
            Case::new("remuw 7%0").reg(5, 7).code(&[W(enc_r(0x3B, 1, 7, 8, 5, 0))]).expect(8, 7),
            // W overflow: MIN32/-1 sign-extends
            Case::new("divw MIN32/-1").reg(5, 0x8000_0000).reg(6, (-1i64) as u64).code(&[W(enc_r(0x3B, 1, 4, 8, 5, 6))]).expect(8, 0xFFFF_FFFF_8000_0000),
            Case::new("remw MIN32%-1").reg(5, 0x8000_0000).reg(6, (-1i64) as u64).code(&[W(enc_r(0x3B, 1, 6, 8, 5, 6))]).expect(8, 0),
            // ordinary signed rounding
            Case::new("div 100/7").reg(5, 100).reg(6, 7).code(&[W(enc_r(0x33, 1, 4, 8, 5, 6))]).expect(8, 14),
            Case::new("rem 100%7").reg(5, 100).reg(6, 7).code(&[W(enc_r(0x33, 1, 6, 8, 5, 6))]).expect(8, 2),
            Case::new("div -100/7").reg(5, (-100i64) as u64).reg(6, 7).code(&[W(enc_r(0x33, 1, 4, 8, 5, 6))]).expect(8, (-14i64) as u64),
            Case::new("rem -100%7").reg(5, (-100i64) as u64).reg(6, 7).code(&[W(enc_r(0x33, 1, 6, 8, 5, 6))]).expect(8, (-2i64) as u64),
        ];
        for c in cases {
            check_case(c);
        }
    }

    // ---- branches / jumps ---------------------------------------------

    /// Shape: `b<op> x5, x6, +8 ; addi x9, x0, 99 ; ret`.
    fn br_case(name: &'static str, f3: u32, v1: u64, v2: u64, taken_x9: u64) -> Case {
        Case::new(name)
            .regs(&[(5, v1), (6, v2)])
            .code(&[W(enc_b(f3, 5, 6, 8)), W(enc_i(0x13, 0, 9, 0, 99))])
            .expect(9, taken_x9)
    }

    #[test]
    fn pin_branch() {
        let cases = vec![
            br_case("beq taken", 0, 7, 7, 0),
            br_case("beq not-taken", 0, 7, 8, 99),
            br_case("bne taken", 1, 7, 8, 0),
            br_case("blt taken", 4, (-1i64) as u64, 1, 0),
            br_case("blt not-taken", 4, 1, (-1i64) as u64, 99),
            br_case("bge taken", 5, 1, 1, 0),
            br_case("bltu taken", 6, 1, u64::MAX, 0),
            br_case("bgeu taken", 7, u64::MAX, 1, 0),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_jal_jalr() {
        // Link registers are x5/x6 (NOT x1/ra): the shared epilogue reloads
        // x1 to the return sentinel, which would mask a link write. The
        // epilogue layout is [a0 li; addi x30,x1,0; li x1,sentinel; ret].
        let cases = vec![
            // jal x5, +8: skips the addi, links x5 = pc+4
            Case::new("jal +8 links rd")
                .code(&[W(enc_jal(5, 8)), W(enc_i(0x13, 0, 9, 0, 99))])
                .expects(&[(5, 4), (9, 0)]),
            // jal x0 must NOT write a link register: observed through a copy
            // of x1 taken BEFORE the epilogue reload masks it.
            Case::new("jal x0 no link")
                .code(&[
                    W(enc_i(0x13, 0, 9, 0, 99)), // addi x9, x0, 99
                    W(enc_i(0x13, 0, 5, 1, 0)),  // addi x5, x1, 0 (copy ra)
                    W(enc_jal(0, 4)),            // -> epilogue at 12
                ])
                .expect(5, vm::calc::CALC_RETURN_SENTINEL),
            // jalr x6, x5, 0 with x5=8: dynamic target (the addi x9 at 8)
            Case::new("jalr dynamic target")
                .reg(5, 8)
                .code(&[W(enc_jalr(6, 5, 0)), W(enc_i(0x13, 0, 9, 0, 99))])
                .expects(&[(6, 8), (9, 99)]),
            // jalr clears bit 0 of (rs1+imm): 13 -> 12 (epilogue start)
            Case::new("jalr clears bit0")
                .reg(5, 13)
                .code(&[
                    W(enc_jalr(6, 5, 0)),
                    W(enc_i(0x13, 0, 9, 0, 99)),
                    W(enc_i(0x13, 0, 0, 0, 0)), // nop
                ])
                // the addi x9 at 8 is SKIPPED (13&!1 = 12 lands on the nop);
                // an uncleared bit0 would trap on the misaligned pc 13
                .expects(&[(6, 8), (9, 0)]),
        ];
        for c in cases {
            check_case(c);
        }
    }

    // ---- loads / stores (softcore has no memory subsystem) -------------

    #[test]
    fn pin_loads() {
        let cases = vec![
            Case::new("ld +8").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 3, 8, 5, 8))])
                .expect(8, 0x8F8E_8D8C_8B8A_8988),
            Case::new("lw +8 sign-extends").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 2, 8, 5, 8))])
                .expect(8, 0xFFFF_FFFF_8B8A_8988),
            Case::new("lwu +12").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 6, 8, 5, 12))])
                .expect(8, 0x8F8E_8D8C),
            Case::new("lw sext +12").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 2, 8, 5, 12))])
                .expect(8, 0xFFFF_FFFF_8F8E_8D8C),
            Case::new("lh +0").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 1, 8, 5, 0))])
                .expect(8, 0xFFFF_FFFF_FFFF_8180),
            Case::new("lhu +14").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 5, 8, 5, 14))])
                .expect(8, 0x8F8E),
            Case::new("lb +1").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 0, 8, 5, 1))])
                .expect(8, 0xFFFF_FFFF_FFFF_FF81),
            Case::new("lbu +1").mem(64).no_sc().with_db(5).code(&[W(enc_i(0x03, 4, 8, 5, 1))])
                .expect(8, 0x81),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_stores() {
        let v = 0x1122_3344_5566_7788;
        let cases = vec![
            Case::new("sd").mem(64).no_sc().with_db(5).reg(6, v).code(&[W(enc_s(3, 5, 6, 32))])
                .mem_at(32, 0x88).mem_at(33, 0x77).mem_at(34, 0x66).mem_at(35, 0x55)
                .mem_at(36, 0x44).mem_at(37, 0x33).mem_at(38, 0x22).mem_at(39, 0x11),
            Case::new("sw").mem(64).no_sc().with_db(5).reg(6, v).code(&[W(enc_s(2, 5, 6, 36))])
                .mem_at(36, 0x88).mem_at(37, 0x77).mem_at(38, 0x66).mem_at(39, 0x55)
                .mem_at(40, 0xA8), // untouched pattern neighbor (0x80+40)
            Case::new("sh").mem(64).no_sc().with_db(5).reg(6, v).code(&[W(enc_s(1, 5, 6, 38))])
                .mem_at(38, 0x88).mem_at(39, 0x77).mem_at(40, 0xA8),
            Case::new("sb").mem(64).no_sc().with_db(5).reg(6, v).code(&[W(enc_s(0, 5, 6, 39))])
                .mem_at(39, 0x88).mem_at(40, 0xA8),
        ];
        for c in cases {
            check_case(c);
        }
    }

    // ---- compressed register forms --------------------------------------

    #[test]
    fn pin_compressed_reg_ops() {
        let cases = vec![
            Case::new("c.addi +5").keeps_a0().sc_c(ScAst::CAddi(10, 5)).reg(10, 5)
                .code(&[C(c_addi(10, 5))]).expect(10, 10),
            Case::new("c.addi -1 wrap").sc_c(ScAst::CAddi(14, -1)).reg(14, 0)
                .code(&[C(c_addi(14, -1))]).expect(14, u64::MAX),
            Case::new("c.li -1").sc_c(ScAst::CLi(11, -1)).code(&[C(c_li(11, -1))]).expect(11, u64::MAX),
            Case::new("c.li 31").sc_c(ScAst::CLi(11, 31)).code(&[C(c_li(11, 31))]).expect(11, 31),
            Case::new("c.mv").sc_c(ScAst::CMv(12, 10)).reg(10, 77)
                .code(&[C(c_mv(12, 10))]).expect(12, 77),
            Case::new("c.add").sc_c(ScAst::CAdd(12, 10)).regs(&[(12, 5), (10, 6)])
                .code(&[C(c_add(12, 10))]).expect(12, 11),
            Case::new("c.sub").sc_c(ScAst::CSub(9, 10)).regs(&[(9, 10), (10, 3)])
                .code(&[C(c_sub(9, 10))]).expect(9, 7),
            Case::new("c.and").sc_c(ScAst::CAnd(9, 10)).regs(&[(9, 0xF0), (10, 0x3C)])
                .code(&[C(c_and(9, 10))]).expect(9, 0x30),
            Case::new("c.or").sc_c(ScAst::COr(9, 10)).regs(&[(9, 0xF0), (10, 0x0F)])
                .code(&[C(c_or(9, 10))]).expect(9, 0xFF),
            Case::new("c.xor").sc_c(ScAst::CXor(9, 10)).regs(&[(9, 0xFF), (10, 0x0F)])
                .code(&[C(c_xor(9, 10))]).expect(9, 0xF0),
            Case::new("c.slli").sc_c(ScAst::CSlli(9, 4)).reg(9, 0x10)
                .code(&[C(c_slli(9, 4))]).expect(9, 0x100),
            Case::new("c.srli").sc_c(ScAst::CSrli(9, 2)).reg(9, MIN)
                .code(&[C(c_srli(9, 2))]).expect(9, 0x2000_0000_0000_0000),
            Case::new("c.srai 63").sc_c(ScAst::CSrai(9, 63)).reg(9, MIN)
                .code(&[C(c_srai(9, 63))]).expect(9, u64::MAX),
            Case::new("c.andi -8").sc_c(ScAst::CAndi(9, -8)).reg(9, 0xFF)
                .code(&[C(c_andi(9, -8))]).expect(9, 0xF8),
            Case::new("c.lui -1<<12").sc_c(ScAst::CLui(12, -1))
                .code(&[C(c_lui(12, -1))]).expect(12, 0xFFFF_FFFF_FFFF_F000),
            Case::new("c.addiw wrap").keeps_a0().sc_c(ScAst::CAddiw(10, 1)).reg(10, u64::MAX)
                .code(&[C(c_addiw(10, 1))]).expect(10, 0),
            Case::new("c.addi16sp -64").sc_c(ScAst::CAddi16sp(-64)).reg(2, 0x1000)
                .code(&[C(c_addi16sp(-64))]).expect(2, 0xFC0),
            Case::new("c.addi4spn +32").sc_c(ScAst::CAddi4spn(13, 32)).reg(2, 0x1000)
                .code(&[C(c_addi4spn(13, 32))]).expect(13, 0x1020),
            // softcore RTYPEW::SUBW panics (sub_vec 64-bit assert on 32-bit
            // operands) — ckb only
            Case::new("c.subw").no_sc().regs(&[(9, 5), (10, 7)])
                .code(&[C(c_subw(9, 10))]).expect(9, (-2i64) as u64),
            Case::new("c.addw ovf").sc_c(ScAst::CAddw(9, 10)).regs(&[(9, 0x7FFF_FFFF), (10, 1)])
                .code(&[C(c_addw(9, 10))]).expect(9, 0xFFFF_FFFF_8000_0000),
        ];
        for c in cases {
            check_case(c);
        }
    }

    #[test]
    fn pin_compressed_flow() {
        let cases = vec![
            // addi x9, x0, 99 ; c.beqz x8, +6 -> taken skips to ret? no:
            // shape is [addi x9 (4B), branch/c.j (2B), ret] with the jump
            // landing on ret at +2 from itself.
            Case::new("c.j +2")
                .sc_c(ScAst::CJ(2))
                .code(&[W(enc_i(0x13, 0, 9, 0, 99)), C(c_j(2))])
                .expect(9, 99),
            // c.jr x5 (x5=16) jumping to the epilogue (a0 li at 16, after
            // prologue [li x5, li x8] + words [addi x9 (8-12), c.beqz x8,+4
            // (12-14) -> 16, c.jr x5 (14-16)]). x8=1 keeps the beqz
            // not-taken so the c.jr actually executes and the epilogue is
            // reachable on both static paths.
            Case::new("c.jr to epilogue")
                .sc_c(ScAst::CJr(5))
                .regs(&[(5, 16), (8, 1)])
                .code(&[
                    W(enc_i(0x13, 0, 9, 0, 99)),
                    C(c_beqz(8, 4)),
                    C(c_jr(5)),
                ])
                .expect(9, 99),
            // c.jalr x5 (rs1=x5=16) links ra=x1=pc+2=16. The epilogue copies
            // x1 into x30 before reloading it, so the link value is pinned on
            // x30; softcore compares x1 directly (sc_reg_pin).
            Case::new("c.jalr links ra")
                .sc_c(ScAst::CJalr(5))
                .sc_pin(1, 10)
                .regs(&[(5, 16), (8, 1)])
                .code(&[
                    W(enc_i(0x13, 0, 9, 0, 99)),
                    C(c_beqz(8, 4)),
                    C(c_jalr(5)),
                ])
                .expects(&[(9, 99), (30, 16)]),
            Case::new("c.beqz taken")
                .sc_c(ScAst::CBeqz(8, 6))
                .reg(8, 0)
                .code(&[C(c_beqz(8, 6)), W(enc_i(0x13, 0, 9, 0, 99))])
                .expect(9, 0),
            Case::new("c.beqz not-taken")
                .sc_c(ScAst::CBeqz(8, 6))
                .reg(8, 1)
                .code(&[C(c_beqz(8, 6)), W(enc_i(0x13, 0, 9, 0, 99))])
                .expect(9, 99),
            Case::new("c.bnez taken")
                .sc_c(ScAst::CBnez(8, 6))
                .reg(8, 7)
                .code(&[C(c_bnez(8, 6)), W(enc_i(0x13, 0, 9, 0, 99))])
                .expect(9, 0),
        ];
        for c in cases {
            check_case(c);
        }
    }

    /// Compressed memory forms: softcore todo!() — ckb only.
    #[test]
    fn pin_compressed_mem() {
        let cases = vec![
            Case::new("c.lwsp +8").no_sc().mem(64).with_db(2)
                .code(&[C(c_lwsp(8, 8))]).expect(8, 0xFFFF_FFFF_8B8A_8988),
            Case::new("c.ldsp +16").no_sc().mem(64).with_db(2)
                .code(&[C(c_ldsp(8, 16))]).expect(8, 0x9796_9594_9392_9190),
            Case::new("c.swsp +12").no_sc().mem(64).with_db(2).reg(10, 0xAABB_CCDD)
                .code(&[C(c_swsp(10, 12))])
                .mem_at(12, 0xDD).mem_at(13, 0xCC).mem_at(14, 0xBB).mem_at(15, 0xAA),
            Case::new("c.sdsp +24").no_sc().mem(64).with_db(2).reg(10, 0xAABB_CCDD_EEFF_0011)
                .code(&[C(c_sdsp(10, 24))])
                .mem_at(24, 0x11).mem_at(25, 0x00).mem_at(26, 0xFF).mem_at(27, 0xEE)
                .mem_at(28, 0xDD).mem_at(29, 0xCC).mem_at(30, 0xBB).mem_at(31, 0xAA),
            Case::new("c.lw 0(x10)").no_sc().mem(64).with_db(10)
                .code(&[C(c_lw(9, 10, 0))]).expect(9, 0xFFFF_FFFF_8382_8180),
            Case::new("c.ld 8(x10)").no_sc().mem(64).with_db(10)
                .code(&[C(c_ld(9, 10, 8))]).expect(9, 0x8F8E_8D8C_8B8A_8988),
            Case::new("c.sw 8(x10)").no_sc().mem(64).with_db(10).reg(9, 0x1234)
                .code(&[C(c_sw(9, 10, 8))])
                .mem_at(8, 0x34).mem_at(9, 0x12).mem_at(10, 0x00).mem_at(11, 0x00).mem_at(12, 0x8C),
            Case::new("c.sd 8(x10)").no_sc().mem(64).with_db(10).reg(9, 0x1234)
                .code(&[C(c_sd(9, 10, 8))])
                .mem_at(8, 0x34).mem_at(9, 0x12).mem_at(10, 0x00).mem_at(16, 0x90),
        ];
        for c in cases {
            check_case(c);
        }
    }
}
