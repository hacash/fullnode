#![allow(unused_imports)]

use super::super::*;
use crate::calc::decode::{BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
use crate::calc::{CalcFnObj, CalcRunLimits};
use crate::fitshc::compile_body;
use crate::lang::Tokenizer;

use super::helpers::{HIP5_FITSH, compile, compile_with_input, decode_stream, decode_stream_p, run_e2e, run_e2e_mode, run_input_mode};


    /// The debug engine must stay byte-identical to the production entry
    /// (the P0 experiment tooling reads the side-channel; see
    /// docs.loc/rv64im/10-a0-maxlive-and-licm.md).
    #[test]
    fn dbg_engine_matches_production() {
        let bodies: Vec<(&str, bool)> = vec![
            ("var s = 0u64\nvar i = 0u64\nwhile i < 10 { s += i\ni += 1 }\nreturn s as bytes", false),
            (
                "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nreturn a as bytes",
                true,
            ),
            (
                "var v = byte(data, 0)\nvar t = 7u64\nif v == 2 { t = 20 }\nif v == 17 { t = 30 }\nif v == 39 { t = 40 }\nreturn t as bytes",
                true,
            ),
            (HIP5_FITSH, true),
        ];
        for (bi, (body, has_input)) in bodies.iter().enumerate() {
            let (ir, _) = if *has_input {
                compile_with_input(body)
            } else {
                compile(body)
            };
            for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                let prod = lower_body_mode_arena(&ir, *has_input, mode, ARENA_SIZE_MAX).unwrap();
                let (dbg_lo, dbg) =
                    lower_body_mode_arena_dbg(&ir, *has_input, mode, ARENA_SIZE_MAX).unwrap();
                assert_eq!(
                    prod.code, dbg_lo.code,
                    "body #{bi} {mode:?}: dbg engine code diverged"
                );
                assert_eq!(prod.pool, dbg_lo.pool, "body #{bi} {mode:?}: pool diverged");
                // timeline sanity: every slot's events are idx-ordered; the
                // walk consumed the same node count the lowering cursor did
                for evs in &dbg.timeline.ev {
                    assert!(
                        evs.windows(2).all(|w| w[0].idx <= w[1].idx),
                        "body #{bi}: event order"
                    );
                }
                assert!(dbg.timeline.stmt_total >= 1);
            }
        }
    }


    /* ---- encoder round-trips (consensus decoder as the oracle) ---- */

    #[test]
    fn assembler_encodings_decode() {
        let mut a = Asm::new();
        let l = a.new_label();
        a.addi(5, 0, -1);
        a.li(6, 2047);
        a.li(7, 2048);
        a.li(28, 0x1234_5678);
        a.li(29, 0xFFFF_8000);
        a.li(30, 0xFFFF_FFFF);
        a.li(16, 0x7FFF_FFFF);
        a.r(0, 6, 5, 0, 7, 0b0110011); // add
        a.r(1, 6, 5, 3, 7, 0b0110011); // mulhu
        a.r(1, 6, 5, 5, 7, 0b0110011); // divu
        a.r(0x20, 6, 5, 0, 7, 0b0110011); // sub
        a.r(0, 6, 5, 3, 7, 0b0110011); // sltu (f3=3!)
        a.ld(5, 8, -16);
        a.sd(5, 8, 2047);
        a.lbu(5, 8, 0);
        a.sb(5, 8, 0);
        a.srli_(5, 6, 31);
        a.slli_(5, 6, 63);
        a.add(5, 6, 7);
        a.bne(5, 6, l);
        a.bgeu(5, 6, l);
        a.bltu(5, 6, l);
        a.jal_(0, l);
        a.pcrel_addr(5, l);
        a.place(l);
        a.ret();
        a.ecall();
        let buf = a.finish().unwrap();
        let insts = decode_stream(&buf);
        assert!(insts.len() >= 26);
        assert!(matches!(insts[0], Inst::ImmAlu { op: ImmAluOp::Addi, rd: 5, imm: -1, .. }));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Add, rd: 7, rs1: 5, rs2: 6 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Mulhu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Divu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Sub, .. })));
        // sltu must decode with the CORRECT rs1 (guards the f3=3 vs 11 bug)
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Sltu, rd: 7, rs1: 5, rs2: 6 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Load { op: LoadOp::Ld, rd: 5, rs1: 8, imm: -16 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Store { op: StoreOp::Sd, rs1: 8, rs2: 5, imm: 2047 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Load { op: LoadOp::Lbu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Store { op: StoreOp::Sb, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Branch { kind: BrKind::Ne, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Branch { kind: BrKind::Geu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Branch { kind: BrKind::Ltu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Auipc { rd: 5, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Ecall)));
    }


    /// RVC encoders round-tripped through the consensus `decode_profile_c`.
    /// The encoder bit layouts were derived by inverting the decoder; this
    /// test pins them against it directly.
    #[test]
    fn rvc_encodings_decode() {
        let mut a = Asm::new_rvc();
        // backward c.j (label placed before the jump)
        let back = a.new_label();
        a.place(back);
        a.addi(5, 5, 4);
        a.jal_(ZERO, back);
        // forward jump: optimistic 2-byte c.j slot, patched by `finish`
        let fwd = a.new_label();
        a.jal_(ZERO, fwd);
        a.place(fwd);
        // c.li / c.addi / c.mv / c.add (full 5-bit rd forms, any register)
        a.addi(10, ZERO, 7); // c.li a0, 7
        a.addi(30, 30, -3); // c.addi t5, -3
        a.mv(14, 10); // c.mv a4, a0
        a.add(14, 14, 10); // c.add a4, a0
        // window-restricted forms
        a.ld(12, 8, 16); // c.ld a2, 16(s0)
        a.sd(12, 8, 24); // c.sd a2, 24(s0)
        a.slli_(12, 12, 3); // c.slli
        a.srli_(12, 12, 2); // c.srli (window)
        a.sub(12, 12, 10); // c.sub (window rr)
        a.xor(12, 12, 11); // c.xor
        a.or_(12, 12, 10); // c.or
        a.and_(12, 12, 11); // c.and
        // c.beqz / c.bnez (backward, window rs1)
        let lb = a.new_label();
        a.place(lb);
        a.beq(12, ZERO, lb);
        a.bne(12, ZERO, lb);
        // non-compressible stays 4-byte: forward beq, non-window regs
        let lf = a.new_label();
        a.beq(5, ZERO, lf);
        a.place(lf);
        a.beq(5, 6, lb);
        a.slli_(28, 28, 2); // c.slli works on any rd
        a.ret(); // c.jr ra
        a.li(10, 100000); // lui+addi -> c.lui + c.addi? (100000 hi = 24)
        let buf = a.finish().unwrap();
        let insts = decode_stream_p(&buf, crate::calc::container::CALC_PROFILE_RV64IMC);
        let find = |f: &dyn Fn(&Inst) -> bool| insts.iter().filter(|i| f(i)).count();
        // c.j backward + forward (both 2-byte now)
        assert_eq!(find(&|i| matches!(i, Inst::Jal { rd: 0, .. })), 2, "jal forms");
        // c.li/c.mv/c.add/c.addi decode to their uncompressed equivalents
        assert!(find(&|i| matches!(i, Inst::ImmAlu { op: ImmAluOp::Addi, rd: 10, rs1: 0, imm: 7 })) >= 1);
        assert!(find(&|i| matches!(i, Inst::ImmAlu { op: ImmAluOp::Addi, rd: 30, imm: -3, .. })) >= 1);
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Add, rd: 14, rs1: 0, rs2: 10 })) >= 1, "c.mv");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Add, rd: 14, rs1: 14, rs2: 10 })) >= 1, "c.add");
        assert!(find(&|i| matches!(i, Inst::Load { op: LoadOp::Ld, rd: 12, rs1: 8, imm: 16 })) == 1, "c.ld");
        assert!(find(&|i| matches!(i, Inst::Store { op: StoreOp::Sd, rs1: 8, rs2: 12, imm: 24 })) == 1, "c.sd");
        assert!(find(&|i| matches!(i, Inst::ShiftImm { kind: ShiftKind::Sll, rd: 12, shamt: 3, word: false, .. })) == 1, "c.slli");
        assert!(find(&|i| matches!(i, Inst::ShiftImm { kind: ShiftKind::Srl, rd: 12, shamt: 2, word: false, .. })) == 1, "c.srli");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Sub, rd: 12, rs1: 12, rs2: 10 })) == 1, "c.sub");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Xor, rd: 12, rs1: 12, rs2: 11 })) == 1, "c.xor");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Or, rd: 12, rs1: 12, rs2: 10 })) == 1, "c.or");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::And, rd: 12, rs1: 12, rs2: 11 })) == 1, "c.and");
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Eq, rs1: 12, rs2: 0, .. })) == 1, "c.beqz");
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Ne, rs1: 12, rs2: 0, .. })) == 1, "c.bnez");
        // forward beq + beq rs1,rs2 stay full-width
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Eq, rs1: 5, rs2: 0, .. })) == 1);
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Eq, rs1: 5, rs2: 6, .. })) == 1);
        assert!(find(&|i| matches!(i, Inst::Jalr { rd: 0, rs1: 1, imm: 0 })) == 1, "c.jr");
        // c.slli on non-window rd (x28) still compresses
        assert!(find(&|i| matches!(i, Inst::ShiftImm { kind: ShiftKind::Sll, rd: 28, shamt: 2, word: false, .. })) == 1);
        // 100000 = hi 0x19<<12 + 0x610 -> c.lui + addi(0x610 > 31 -> full-width)
        assert!(find(&|i| matches!(i, Inst::Lui { rd: 10, val: 0x18000, .. })) == 1, "c.lui");
        // verify the stream is 2-byte aligned overall
        assert_eq!(buf.len() % 2, 0);
    }


    /// The three lowering modes must produce identical results through the
    /// consensus engine (RVC path decode + execute end to end).
    #[test]
    fn rvc_modes_agree_e2e() {
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            // li ladder
            for v in [0u64, 2047, 65535, 100000, 0x7FFF_FFFF, 1000000007, u64::MAX] {
                let body = format!("return {v}u64 as bytes");
                let out = run_e2e_mode(&body, &[], mode);
                assert_eq!(out, v.to_be_bytes().to_vec(), "li({v}) mode {mode:?}");
            }
            // checked arithmetic + faults
            let out = run_e2e_mode("return ((3000000000u64 * 3) % 1000000007) as bytes", &[], mode);
            assert_eq!(out, (3000000000u64 * 3 % 1000000007).to_be_bytes().to_vec());
            // loop with break/continue
            let body = "var s = 0u64\nvar i = 0u64\nwhile i < 100 { i += 1\nif i % 3 == 0 { continue }\nif i > 50 { break }\ns += i }\nreturn s as bytes";
            let out = run_e2e_mode(body, &[], mode);
            let want: u64 = (1..=50u64).filter(|i| i % 3 != 0).sum();
            assert_eq!(out, want.to_be_bytes().to_vec(), "loop mode {mode:?}");
            // bytes chain — size() is U16 in the interpreter, so `as bytes`
            // is the 2-byte BE view (task-1 A2; the pinned u64 form follows)
            let body = "return size(\"abcdef\" ++ \"1234\") as bytes";
            let out = run_e2e_mode(body, &[], mode);
            assert_eq!(out, 10u16.to_be_bytes().to_vec());
            let body = "return (size(\"abcdef\" ++ \"1234\") as u64) as bytes";
            let out = run_e2e_mode(body, &[], mode);
            assert_eq!(out, 10u64.to_be_bytes().to_vec());
        }
        // deep-input remap mode: input bytes plumbing
        let body = "var s = 0u64\nvar i = 0u64\nwhile i < size(data) { s += byte(data, i)\ni += 1 }\nreturn s as bytes";
        let input: Vec<u8> = (0u8..=39).collect();
        let out = run_input_mode(body, &input, Rv64Mode::ImcRemap);
        assert_eq!(out, input.iter().map(|&b| b as u64).sum::<u64>().to_be_bytes().to_vec());
    }


    #[test]
    fn li_values_execute_via_e2e() {
        // li correctness is pinned by returning each constant as bytes.
        for v in [
            0u64, 1, 2047, 2048, 4095, 65535, 0x12345, 0xFFFFFF, 0x7FFF_FFFF, 0xFFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFF, 0x8000_0000_0000_0000, 1000000007,
        ] {
            let body = format!("return {v}u64 as bytes");
            let out = run_e2e(&body, &[]);
            assert_eq!(out, v.to_be_bytes().to_vec(), "li({v}) round trip");
        }
    }
