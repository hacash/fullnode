//! Opcode-behavior pinning matrix (防漂移矩阵) for the calc RV64IM interpreter.
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
//!   - LOAD/STORE/AMO are `todo!()` (no memory subsystem).
//! Those cases run ours vs ckb-vm only.

#[cfg(feature = "execute")]
#[allow(dead_code)]
mod pin {
    use softcore_rv64::config;
    use softcore_rv64::prelude::{bv, BitVector};
    use softcore_rv64::raw::{self, ast};
    use softcore_rv64::{new_core, Core, Trap};
    use vm::calc::container::CalcContainer;
    use vm::calc::decode::{decode_at, BrKind, ImmAluOp, Inst, RegAluOp, ShiftKind};
    use vm::calc::interp::run_with_state;
    use vm::calc::verify::verify_container;

    const MEM_KIB: u32 = 8;
    const INPUT_MAX: u16 = 512;
    const OUTPUT_MAX: u16 = 512;
    const CB: u64 = vm::calc::CALC_CODE_BASE;

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
    }
    const W: fn(u32) -> WordSpec = WordSpec::W;

    /// What softcore should execute for this case (the first under-test
    /// instruction only).
    enum ScAst {
        /// Map the first word through OUR decoder, then translate the decoded
        /// `Inst` into a softcore ast.
        Word,
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
        /// value). Replaces the default ours-vs-softcore comparison when the
        /// full program overwrites the register the single instruction wrote.
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

    // ------------------------------------------------------------------
    // softcore adapter
    // ------------------------------------------------------------------

    fn sreg(n: u8) -> raw::regidx {
        raw::regidx::Regidx(BitVector::new(n as u64))
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
            Inst::UnAlu { .. }
            | Inst::BitImm { .. }
            | Inst::Slliuw { .. }
            | Inst::Fence
            | Inst::Trap => return None,
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

    /// Which register a softcore-executed form writes (for the oracle
    /// comparison); `None` = control-flow only (or writes x0).
    fn sc_written(k: &ScAst, inst: Option<&Inst>) -> Option<u8> {
        match k {
            ScAst::Word => match inst? {
                // Zbb/Zbc forms never reach the softcore oracle (mapped to
                // None above); treat any unexpected hit as unwritten.
                Inst::UnAlu { .. }
                | Inst::BitImm { .. }
                | Inst::Slliuw { .. }
                | Inst::Fence
                | Inst::Trap => return None,
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
            }
        }
        if !a0_test {
            for w in enc_li(10, 0) {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        // HALT ecall (exit 0, empty output). Snapshot ra into x30 so jal
        // link tests can still observe it after halt clobbers a0/a1/a7.
        bytes.extend_from_slice(&enc_i(0x13, 0, 30, 1, 0).to_le_bytes()); // addi x30, x1, 0
        bytes.extend_from_slice(&enc_i(0x13, 0, 11, 0, 0).to_le_bytes()); // addi a1, x0, 0
        bytes.extend_from_slice(&enc_i(0x13, 0, 17, 0, 0).to_le_bytes()); // addi a7, x0, 0
        bytes.extend_from_slice(&0x0000_0073u32.to_le_bytes()); // ecall HALT
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
            let ndb = align16(CB + bytes.len() as u64);
            if ndb == db {
                break (bytes, db);
            }
            db = ndb;
        };
        let container = CalcContainer {
            profile: vm::calc::CALC_PROFILE_RV64IM,
            abi: 1,
            mem_kib: MEM_KIB,
            entry: CB as u32,
            code: bytes,
            rodata: vec![],
            init: case.init_mem.clone(),
            bss_len: 0,
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
        use ckb_vm::{CoreMachine, DefaultCoreMachine, DefaultMachineBuilder, FlatMemory, Memory, Syscalls};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        type Ck = DefaultCoreMachine<u64, FlatMemory<u64>>;
        struct HaltEcall { halted: Arc<AtomicBool> }
        impl Syscalls<Ck> for HaltEcall {
            fn initialize(&mut self, _m: &mut Ck) -> Result<(), ckb_vm::Error> { Ok(()) }
            fn ecall(&mut self, m: &mut Ck) -> Result<bool, ckb_vm::Error> {
                if m.registers()[17] == vm::calc::ECALL_HALT {
                    self.halted.store(true, Ordering::SeqCst);
                    return Ok(true);
                }
                Ok(false)
            }
        }
        let lay = verify_container(&b.container).expect("verify").layout_snapshot();
        let core =
            Ck::new_with_memory(ckb_vm::ISA_IMC, ckb_vm::machine::VERSION2, u64::MAX, lay.mem_len as usize);
        let sys = HaltEcall { halted: Arc::new(AtomicBool::new(false)) };
        let halted = sys.halted.clone();
        let mut machine = DefaultMachineBuilder::new(core).syscall(Box::new(sys)).build();
        {
            let m = machine.memory_mut();
            m.store_bytes(CB, &b.container.code).unwrap();
            m.store_bytes(lay.rodata_base, &b.container.rodata).unwrap();
            m.store_bytes(lay.data_base, &b.container.init).unwrap();
        }
        machine.set_register(10, lay.input_base);
        machine.set_register(11, 0);
        machine.set_register(12, lay.output_base);
        machine.set_register(13, lay.output_max);
        machine.set_register(2, lay.stack_top);
        machine.update_pc(CB);
        machine.commit_pc();
        let mut decoder = ckb_vm::decoder::build_decoder::<u64>(machine.isa(), machine.version());
        let mut steps = 0u64;
        loop {
            if halted.load(Ordering::SeqCst) {
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
            let decoded = decode_at(bytes, b.test_pc as usize, vm::calc::CALC_PROFILE_RV64IM).expect(case.name);
            let pc = CB + b.test_pc;
            let (a, next) = match sc {
                ScAst::Word => {
                    // decode_at returns PC-relative immediates; inst_to_ast
                    // subtracts `pc` for jal/branch, so pass the code-relative
                    // test_pc (usually 0 for the first under-test word).
                    let a = inst_to_ast(&decoded.0, b.test_pc)
                        .expect("case must call no_sc() for instructions softcore cannot execute");
                    let next = match decoded.0 {
                        Inst::Jal { target, .. } => Some(pc.wrapping_add(target as u64)),
                        // Branch: taken/not-taken is data-dependent and
                        // softcore runs the real condition; only the register
                        // pins are compared for branch cases.
                        Inst::Branch { .. } => None,
                        Inst::Jalr { .. } => None, // dynamic target
                        _ => Some(pc + decoded.1 as u64),
                    };
                    (a, next)
                }
            };
            let ilen = decoded.1 as u64;
            let (regs_s, next_pc) = softcore_exec(a, &b.init, pc, ilen);
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
            // one 4-byte prologue word (li x9) puts the auipc at CODE_BASE+4
            Case::new("auipc pc+1<<12").reg(9, 1).code(&[W(enc_u(0x17, 8, 1))]).expect(8, CB + 4 + 0x1000),
            Case::new("auipc negative imm").reg(9, 1).code(&[W(enc_u(0x17, 8, 0xFFFFF))]).expect(8, (CB as i64 + 4 - 4096) as u64),
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
                .expects(&[(5, CB + 4), (9, 0)]),
            // jal x0 must NOT write a link register: observed through a copy
            // of x1 taken BEFORE the epilogue reload masks it.
            Case::new("jal x0 no link")
                .code(&[
                    W(enc_i(0x13, 0, 9, 0, 99)), // addi x9, x0, 99
                    W(enc_i(0x13, 0, 5, 1, 0)),  // addi x5, x1, 0 (copy ra)
                    W(enc_jal(0, 4)),            // -> epilogue at 12
                ])
                .expect(5, 0),
            // jalr x6, x5, 0: x5 is the absolute address of the addi (after
            // the 8-byte lui+addi prologue that materializes CODE_BASE+12).
            Case::new("jalr dynamic target")
                .reg(5, CB + 12)
                .code(&[W(enc_jalr(6, 5, 0)), W(enc_i(0x13, 0, 9, 0, 99))])
                .expects(&[(6, CB + 12), (9, 99)]),
            // jalr clears bit 0: prologue is 8 bytes (lui+addi for CB+17),
            // jalr at +8, addi at +12, nop at +16. (CB+17)&!1 = CB+16 (nop).
            Case::new("jalr clears bit0")
                .reg(5, CB + 17)
                .code(&[
                    W(enc_jalr(6, 5, 0)),
                    W(enc_i(0x13, 0, 9, 0, 99)),
                    W(enc_i(0x13, 0, 0, 0, 0)), // nop
                ])
                .expects(&[(6, CB + 12), (9, 0)]),
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
}

