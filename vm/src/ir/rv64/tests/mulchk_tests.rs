#![allow(unused_imports)]

use super::super::*;
use crate::calc::decode::{BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
use crate::calc::{CalcFnObj, CalcRunLimits};
use crate::fitshc::compile_body;
use crate::lang::Tokenizer;

use super::helpers::{li_i, mv_r};


    /// Review counter-example (acceptance review of 03374a6): a base
    /// register write BETWEEN the copy and the check used to escape the
    /// backward-scan veto (the region is visited while `base` is still
    /// None). The check then tests the pre-write value while a later
    /// same-base site reads the post-write one — trap-set inclusion
    /// breaks, traps can be swallowed. The copy->check forward rescan
    /// must veto: None.
    #[test]
    fn a19_mulchk_base_write_between_copy_and_check_vetoes() {
        // li k6,6; mv d,r; li r,v2; mulhu sc,d,k6; bne sc -> 90
        let insts = [
            li_i(11, 6),
            mv_r(10, 25),
            li_i(25, 99), // base redefined after the copy, before the check
            Inst::RegAlu { op: RegAluOp::Mulhu, rd: 31, rs1: 10, rs2: 11 },
            Inst::Branch { kind: BrKind::Ne, rs1: 31, rs2: 0, target: 2880 },
        ];
        assert_eq!(mul_check_base_const(&insts, 3, 10, Some(11)), None);
    }


    /// Positive control (guards against over-tightening): same shape
    /// without the in-region base write recognizes (r, 6).
    #[test]
    fn a19_mulchk_clean_site_recognized() {
        let insts = [
            li_i(11, 6),
            mv_r(10, 25),
            Inst::RegAlu { op: RegAluOp::Mulhu, rd: 31, rs1: 10, rs2: 11 },
        ];
        assert_eq!(mul_check_base_const(&insts, 2, 10, Some(11)), Some((25, 6)));
    }


    /// The pow2/srl form (t = None) shares the same hole and the same
    /// fix: an in-region base write must veto.
    #[test]
    fn a19_mulchk_srl_form_base_write_vetoes() {
        // mv d,r; li r,v2; srli sc,d,61; bne sc -> 90
        let insts = [
            mv_r(10, 25),
            li_i(25, 3), // base redefined between copy and check
            Inst::ShiftImm { kind: ShiftKind::Srl, rd: 31, rs1: 10, shamt: 61, word: false },
            Inst::Branch { kind: BrKind::Ne, rs1: 31, rs2: 0, target: 2880 },
        ];
        assert_eq!(mul_check_base_const(&insts, 2, 10, None), None);
        // control without the rewrite: recognized, k comes from the caller
        let clean = [
            mv_r(10, 25),
            Inst::ShiftImm { kind: ShiftKind::Srl, rd: 31, rs1: 10, shamt: 61, word: false },
        ];
        assert_eq!(mul_check_base_const(&clean, 1, 10, None), Some((25, 0)));
    }


    /// A base write BEFORE the copy is harmless (both sites' copies read
    /// the post-write value) and must NOT veto.
    #[test]
    fn a19_mulchk_base_write_before_copy_allowed() {
        let insts = [
            li_i(25, 7), // base defined earlier — fine
            li_i(11, 10),
            mv_r(10, 25),
            Inst::RegAlu { op: RegAluOp::Mulhu, rd: 31, rs1: 10, rs2: 11 },
        ];
        assert_eq!(mul_check_base_const(&insts, 3, 10, Some(11)), Some((25, 10)));
    }
