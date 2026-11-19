#![allow(unused_imports)]


/* ================================================================== */
/* ---- P1 u256 library: differential tests vs the interpreter ------- */
/*                                                                    */
/* The library's emit helpers are exercised through the REAL pipeline: */
/* a synthetic kernel is emitted with the production `Lower` (all      */
/* three modes), packed into a container, verified by the consensus    */
/* decoder, and executed by the calc engine — its result or GuestExit  */
/* code is then diffed against the interpreter's own u256 helpers in   */
/* `vm/src/interpreter/instruction.rs` (the behavior ground truth):    */
/* random plus boundary inputs, divide-by-zero → exit 90, carry        */
/* edges (0, 1, u128::MAX, quotient-overflow), every rounding policy.  */
    use super::super::*;
    use crate::calc::container::{CalcContainer, CODE_BASE, CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC};
    use crate::calc::interp::{run, CalcAbort};
    use crate::calc::verify::verify_container_with_cap;
    use crate::interpreter::{
        add_u256, add_u256_u128, cmp_u256, div_u256_by_u128_to_u128, div_u256_by_u129_to_u128,
        div_u256_by_u129_with_round, mul_wide_u128, round_quot_u128_with_policy, sub_u256_u128,
    };
    use crate::rt::{FinRoundPolicy as Pol, ItrErr, ItrErrCode, SpaceCap};
    use std::collections::HashMap;

    const CYCLES: u64 = 20_000_000;
    const TEST_ERR: fn() -> ItrErr = || ItrErr::new(ItrErrCode::Arithmetic, "u256lib test");

    /// Build a synthetic kernel around `body` (which emits the operand
    /// loads, the library calls and the result stores), then pack/verify/
    /// run it. `Ok` carries the halt output; `Err(code)` carries the guest
    /// exit code (90 = the fault_arith face).
    fn run_kernel_out(
        mode: Rv64Mode,
        input: &[u8],
        out_len: usize,
        body: impl Fn(&mut Lower),
    ) -> Result<Vec<u8>, u64> {
        // Same branch-relaxation loop as `lower_body_mode_arena_impl`:
        // lowering is deterministic, so re-emitting with the failed site
        // ids pinned to the wide forms converges (the Imc/ImcRemap c.bz /
        // c.j windows can miss on the ~2 KiB division routines).
        let mut long_ids = std::collections::HashSet::new();
        let mut wide_ids = std::collections::HashSet::new();
        let code = loop {
            let lists = ListPlanData::default();
            let nilp = NilPlan { tracked: vec![], occ: HashMap::new(), tag_bytes: false };
            let mut lw = Lower::new_mode_arena_bank(
                &[],
                &[],
                mode,
                ARENA_SIZE,
                8,
                WebPlan {
                    banked: vec![],
                    homes: vec![],
                    init_zero: vec![],
                    segs_debug: vec![],
                    no_split_debug: vec![],
                },
                None,
                &[],
                &nilp,
                &lists,
                &[],
                &[],
                false,
                true,
                0,
                None,
                false,
                // D2/D3: this harness drives the library helpers directly,
                // so the shared-routine gates are forced ON (the gate's
                // production path is body_routine_site_counts' job)
                (true, true, true, true, false, false, false, false),
            );
            lw.asm.long_branch_ids = long_ids.clone();
            lw.asm.wide_c_ids = wide_ids.clone();
            // frame base + IO shadows (S2 = input base, S3 = output base —
            // the same roles the real prologue gives them)
            lw.asm.addi(S0, SP, -(lw.frame_bytes as i32));
            lw.asm.mv(S2, A0);
            lw.asm.mv(S3, 12);
            body(&mut lw);
            // success halt: a0 = 0 (exit code), a1 = out_len, a7 = 0
            lw.asm.addi(A0, ZERO, 0);
            lw.asm.li(A1, out_len as u64);
            lw.asm.li(A7, 0);
            lw.asm.ecall();
            // the shared u256 routines, then the fault thunks + trap
            // epilogue (the exact epilogue shape of
            // `lower_body_mode_arena_impl`)
            if let Some(l) = lw.u256div_lbl {
                lw.asm.place(l);
                lw.emit_u256_divmod_u128_routine();
            }
            if let Some(l) = lw.u256div129_lbl {
                lw.asm.place(l);
                lw.emit_u256_divmod_u129_routine();
            }
            if let Some(l) = lw.u256mul_lbl {
                lw.asm.place(l);
                lw.emit_u256_mul_wide_routine();
            }
            if let Some(l) = lw.u256round_hu_lbl {
                lw.asm.place(l);
                lw.emit_u256_round_halfup_routine();
            }
            if let Some(l) = lw.u256round_he_lbl {
                lw.asm.place(l);
                lw.emit_u256_round_halfeven_routine();
            }
            lw.asm.place(lw.fault_arith);
            lw.asm.li(A0, EXIT_ARITH);
            lw.asm.jal_(ZERO, lw.trap_label);
            lw.asm.place(lw.trap_label);
            lw.asm.addi(A1, ZERO, 0);
            lw.asm.li(A7, 0);
            lw.asm.ecall();
            match lw.asm.finish() {
                Ok(code) => break code,
                Err(FinishErr::BranchOverflow(sites)) => {
                    let before = long_ids.len();
                    long_ids.extend(sites);
                    if long_ids.len() == before {
                        panic!("u256 test kernel: branch relaxation did not converge");
                    }
                }
                Err(FinishErr::WideCOverflow(sites)) => {
                    let before = wide_ids.len();
                    wide_ids.extend(sites);
                    if wide_ids.len() == before {
                        panic!("u256 test kernel: c-slot relaxation did not converge");
                    }
                }
                Err(FinishErr::Msg(m)) => panic!("u256 test kernel did not assemble: {m}"),
            }
        };
        let profile = match mode {
            Rv64Mode::Im => CALC_PROFILE_RV64IM,
            _ => CALC_PROFILE_RV64IMC,
        };
        let c = CalcContainer::from_code(profile, 16, CODE_BASE as u32, code, 128, 64);
        let module = verify_container_with_cap(&c, &SpaceCap::new(0))
            .unwrap_or_else(|e| panic!("consensus verify rejected the u256 test kernel: {e}"));
        match run(&module, &c, input, CYCLES, None) {
            Ok(r) => Ok(r.output),
            Err((CalcAbort::GuestExit(code), _)) => Err(code),
            Err((a, _)) => panic!("u256 test kernel aborted: {a:?}"),
        }
    }

    /* ---- input/output packing (little-endian u64 limbs) ---- */

    fn limbs128(v: u128) -> Vec<u8> {
        let mut b = (v as u64).to_le_bytes().to_vec();
        b.extend_from_slice(&((v >> 64) as u64).to_le_bytes());
        b
    }

    fn limb8(v: u64) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    fn out128(bytes: &[u8]) -> u128 {
        assert_eq!(bytes.len(), 16, "expected a 16-byte u128 output");
        (u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as u128)
            | ((u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as u128) << 64)
    }

    /// Push+load `k` u128 operands from the input window at `off..`,
    /// returning their (lo, hi) pool-register pairs.
    fn load_pairs(lw: &mut Lower, off: &mut i32, k: usize) -> Vec<(u8, u8)> {
        let mut out = Vec::new();
        for _ in 0..k {
            let lo = lw.push_vreg();
            lw.asm.ld(lo, S2, *off);
            let hi = lw.push_vreg();
            lw.asm.ld(hi, S2, *off + 8);
            *off += 16;
            out.push((lo, hi));
        }
        out
    }

    fn sd128(lw: &mut Lower, pair: (u8, u8), base_off: i32) {
        lw.asm.sd(pair.0, S3, base_off);
        lw.asm.sd(pair.1, S3, base_off + 8);
    }

    /// Deterministic splitmix64.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn u128(&mut self) -> u128 {
            ((self.next() as u128) << 64) | (self.next() as u128)
        }
    }


    /* ---- 1. mul_wide: 128×128 → 256 ---- */

    #[test]
    fn u256_mul_wide_matches_interpreter() {
        let mut vals: Vec<(u128, u128)> = vec![
            (0, 0),
            (0, 1),
            (1, 0),
            (1, 1),
            (u128::MAX, u128::MAX),
            (u128::MAX, 1),
            (1, u128::MAX),
            (u128::MAX, 2),
            (u64::MAX as u128, u64::MAX as u128),
            (1u128 << 64, 1u128 << 64),
            (1u128 << 127, 1u128 << 127),
            (0xDEADBEEFCAFEBABE_1234567890ABCDEF, 0x0123456789ABCDEF_FEDCBA9876543210),
        ];
        let mut rng = Rng(0x2026_1117);
        for _ in 0..60 {
            vals.push((rng.u128(), rng.u128()));
        }
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for &(a, b) in &vals {
                let mut input = limbs128(a);
                input.extend(limbs128(b));
                let out = run_kernel_out(mode, &input, 32, |lw| {
                    let mut off = 0;
                    let ps = load_pairs(lw, &mut off, 2);
                    let n = lw.emit_u256_mul_wide(ps[0], ps[1]).unwrap();
                    for (i, &r) in n.iter().enumerate() {
                        lw.asm.sd(r, S3, (i as i32) * 8);
                    }
                })
                .unwrap_or_else(|c| panic!("mul_wide must not fault (exit {c})"));
                let lo = out128(&out[0..16]);
                let hi = out128(&out[16..32]);
                let (whi, wlo) = mul_wide_u128(a, b);
                assert_eq!((hi, lo), (whi, wlo), "mul_wide({a}, {b}) mode {mode:?}");
            }
        }
    }

    /* ---- 2. add/sub u128 into a 256-bit value ---- */

    #[test]
    fn u256_addsub_u128_matches_interpreter() {
        let mut rng = Rng(0xBEEF);
        let mut cases: Vec<(u128, u128, u128)> = vec![
            (0, 0, 0),
            (0, 0, 1),
            (u128::MAX, 0, u128::MAX),
            (u128::MAX, u128::MAX, 0),
            (u128::MAX, u128::MAX, u128::MAX),
            (u128::MAX - 1, 1, 1), // carry chain into hi
            (0, 1, u128::MAX),     // lo overflows into hi
            (1, 0, 2),             // hi borrows for sub
            (0, 1, 1),             // sub exact
            (0, 0, 1),             // sub underflow → 90
            (1, 1, u128::MAX),     // 256-bit add max carry edge
        ];
        for _ in 0..50 {
            cases.push((rng.u128(), rng.u128(), rng.u128()));
        }
        for add in [true, false] {
            for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                for &(hi, lo, z) in &cases {
                    let want = if add {
                        add_u256_u128(hi, lo, z)
                    } else {
                        sub_u256_u128(hi, lo, z)
                    };
                    let mut input = limbs128(lo);
                    input.extend(limbs128(hi));
                    input.extend(limbs128(z));
                    let got = run_kernel_out(mode, &input, 32, |lw| {
                        let mut off = 0;
                        let ps = load_pairs(lw, &mut off, 3);
                        let n = [ps[0].0, ps[0].1, ps[1].0, ps[1].1];
                        if add {
                            lw.emit_u256_add_u128(&n, ps[2]);
                        } else {
                            lw.emit_u256_sub_u128(&n, ps[2]);
                        }
                        sd128(lw, (n[0], n[1]), 0);
                        sd128(lw, (n[2], n[3]), 16);
                    });
                    match (&want, got) {
                        (Some((whi, wlo)), Ok(out)) => {
                            assert_eq!(out128(&out[0..16]), *wlo, "{} lo ({} {} {}) {mode:?}", if add { "add" } else { "sub" }, hi, lo, z);
                            assert_eq!(out128(&out[16..32]), *whi, "{} hi ({} {} {}) {mode:?}", if add { "add" } else { "sub" }, hi, lo, z);
                        }
                        (None, Err(90)) => {}
                        (None, Err(c)) => panic!("expected 90, got {c}"),
                        (Some(_), Err(c)) => panic!("unexpected fault {c} on ({hi}, {lo}, {z})"),
                        _ => panic!("impossible combination"),
                    }
                }
            }
        }
    }

    /* ---- 3. 256 ÷ 128 ---- */

    #[test]
    fn u256_divmod_u128_matches_interpreter() {
        let mut rng = Rng(0x0D1);
        let mut cases: Vec<(u128, u128, u128)> = vec![
            (0, 0, 0),              // ÷0 → 90
            (1, 1, 0),              // ÷0 → 90
            (0, 0, 1),              // 0/1 = 0
            (0, 1, 1),              // 1/1
            (0, u128::MAX, 1),
            (0, u128::MAX, u128::MAX),
            (1, 0, 1),              // hi == d → 90 (quotient overflows)
            (1, 0, 2),              // hi < d, q = 2^127
            (1, u128::MAX, u128::MAX), // hi == d → 90
            (2, 0, 1),              // hi > d → 90
            (u128::MAX, u128::MAX, u128::MAX),
            (u128::MAX - 1, 12345, 987654321),
            (1u128 << 100, 0, 1u128 << 50),
        ];
        for _ in 0..60 {
            cases.push((rng.u128(), rng.u128(), if rng.next() % 8 == 0 { 0 } else { rng.u128() }));
        }
        // guarantee plenty of hi < d cases (the non-fault face)
        for _ in 0..30 {
            let d = (rng.u128() | 1) | (1u128 << 127);
            cases.push((rng.next() as u128, rng.u128(), d));
        }
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for &(hi, lo, d) in &cases {
                let want = div_u256_by_u128_to_u128(hi, lo, d);
                let mut input = limbs128(lo);
                input.extend(limbs128(hi));
                input.extend(limbs128(d));
                let got = run_kernel_out(mode, &input, 32, |lw| {
                    let mut off = 0;
                    let ps = load_pairs(lw, &mut off, 3);
                    for _ in 0..6 {
                        lw.pop_reg();
                    }
                    lw.emit_u256_divmod_u128(true, &[ps[0].0, ps[0].1, ps[1].0, ps[1].1], ps[2])
                        .unwrap();
                    let regs: Vec<u8> = lw
                        .vstack
                        .iter()
                        .map(|&l| loc_reg(l))
                        .collect();
                    let q = (regs[regs.len() - 4], regs[regs.len() - 3]);
                    let r = (regs[regs.len() - 2], regs[regs.len() - 1]);
                    sd128(lw, q, 0);
                    sd128(lw, r, 16);
                });
                match (&want, got) {
                    (Some((wq, wr)), Ok(out)) => {
                        assert_eq!(out128(&out[0..16]), *wq, "quo ({hi}, {lo}) / {d} {mode:?}");
                        assert_eq!(out128(&out[16..32]), *wr, "rem ({hi}, {lo}) % {d} {mode:?}");
                    }
                    (None, Err(90)) => {}
                    (None, Err(c)) => panic!("expected 90, got {c}"),
                    (Some(_), Err(c)) => panic!("unexpected fault {c} on ({hi}, {lo}) / {d}"),
                    _ => panic!("impossible combination"),
                }
            }
        }
    }

    /* ---- 4. 256 ÷ 129 ---- */

    #[test]
    fn u256_divmod_u129_matches_interpreter() {
        let mut rng = Rng(0x129);
        let mut cases: Vec<(u128, u128, u128, u128)> = vec![
            // (n_hi, n_lo, d_hi, d_lo)
            (0, 0, 0, 0),          // ÷0 → 90
            (0, 1, 0, 0),          // ÷0 → 90
            (0, 0, 0, 1),          // 0 / 1
            (0, 1, 0, 1),          // 1 / 1
            (0, 1, 1, 0),          // 1 / 2^128
            (u128::MAX, u128::MAX, 1, u128::MAX), // max / (2^128+MAX) — quotient 1 edge? floor((2^256-1)/(2^129-1)) = 1
            (1, 0, 1, 0),          // 2^128 / 2^128 = 1
            (2, 0, 1, 0),          // 2^129 / 2^128 = 2
            (0, u128::MAX, 1, u128::MAX), // < d → q 0
            (1, 0, 0, 1),          // delegated path: 2^128 / 1 → hi >= d → 90
            (0, u128::MAX, 0, 1),  // delegated: MAX / 1
            (0, 123, 2, 5),        // d_hi > 1 → 90
            (1, 1, 1, 0),
            (u128::MAX, 7, 1, 3),
        ];
        for _ in 0..50 {
            let dbit = [0u128, 0, 1, 1, 2][(&rng.next() % 5) as usize]; // bias to valid, keep the >1 face
            cases.push((rng.u128(), rng.u128(), dbit, rng.u128()));
        }
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for &(hi, lo, dh, dl) in &cases {
                let want = div_u256_by_u129_to_u128(hi, lo, dh, dl);
                let mut input = limbs128(lo);
                input.extend(limbs128(hi));
                input.extend(limbs128(dl));
                input.extend(limb8(dh as u64));
                let got = run_kernel_out(mode, &input, 40, |lw| {
                    let mut off = 0;
                    let ps = load_pairs(lw, &mut off, 3);
                    let dbit = lw.push_vreg();
                    lw.asm.ld(dbit, S2, off);
                    for _ in 0..7 {
                        lw.pop_reg();
                    }
                    lw.emit_u256_divmod_u129(
                        true,
                        &[ps[0].0, ps[0].1, ps[1].0, ps[1].1],
                        ps[2],
                        dbit,
                    )
                    .unwrap();
                    let regs: Vec<u8> = lw
                        .vstack
                        .iter()
                        .map(|&l| loc_reg(l))
                        .collect();
                    let q = (regs[regs.len() - 5], regs[regs.len() - 4]);
                    let r = (regs[regs.len() - 3], regs[regs.len() - 2]);
                    let rh = regs[regs.len() - 1];
                    sd128(lw, q, 0);
                    sd128(lw, r, 16);
                    lw.asm.sd(rh, S3, 32);
                });
                match (&want, got) {
                    (Some((wq, wrh, wrl)), Ok(out)) => {
                        assert_eq!(out128(&out[0..16]), *wq, "quo ({hi}, {lo}) / ({dh}, {dl}) {mode:?}");
                        assert_eq!(out128(&out[16..32]), *wrl, "rem lo ({hi}, {lo}) / ({dh}, {dl}) {mode:?}");
                        assert_eq!(out[32..40], limb8(*wrh as u64), "rem hi ({hi}, {lo}) / ({dh}, {dl}) {mode:?}");
                    }
                    (None, Err(90)) => {}
                    (None, Err(c)) => panic!("expected 90, got {c} on ({hi}, {lo}) / ({dh}, {dl})"),
                    (Some(_), Err(c)) => panic!("unexpected fault {c} on ({hi}, {lo}) / ({dh}, {dl})"),
                    _ => panic!("impossible combination"),
                }
            }
        }
    }

    /* ---- 5. rounding policies on a u128 quotient ---- */

    #[test]
    fn u256_round_u128_matches_interpreter() {
        let policies = [
            Pol::Exact,
            Pol::Floor,
            Pol::Ceil,
            Pol::HalfUp,
            Pol::HalfEven,
        ];
        let mut rng = Rng(0x0F0F);
        for &pol in &policies {
            let mut cases: Vec<(u128, u128, u128)> = vec![
                (0, 0, 1),                       // rem 0 everywhere
                (u128::MAX, 0, 1),
                (u128::MAX - 1, 1, 2),           // ceil/halfup → MAX (fits)
                (u128::MAX, 1, 2),               // round-up carry → 90
                (5, 0, 3),
                (5, 1, 3),                       // tie up (halfup)
                (4, 1, 2),                       // tie even stays
                (3, 1, 2),                       // tie odd rounds up
                (0, 1, 2),                       // 0.5 → halfup 1
                (0, u128::MAX - 1, u128::MAX),   // rem = MAX-1, d = MAX
                (0, u128::MAX / 2, u128::MAX),   // rem = d/2-… half-even below-half shortcut edge
                (0, u128::MAX / 2 + 1, u128::MAX), // rem above half (msb shortcut path)
                (7, 3, 8),                       // exact rem
            ];
            for _ in 0..40 {
                let d = rng.u128() | 1;
                let q = rng.u128();
                let r = rng.u128() % d;
                cases.push((q, r, d));
            }
            for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                for &(q, r, d) in &cases {
                    let want = round_quot_u128_with_policy(q, r, d, pol, TEST_ERR);
                    let mut input = limbs128(q);
                    input.extend(limbs128(r));
                    input.extend(limbs128(d));
                    let got = run_kernel_out(mode, &input, 16, |lw| {
                        let mut off = 0;
                        let ps = load_pairs(lw, &mut off, 3);
                        for _ in 0..6 {
                            lw.pop_reg();
                        }
                        lw.emit_u128_round_quot(pol, ps[0], ps[1], ps[2]).unwrap();
                        sd128(lw, ps[0], 0);
                    });
                    match (&want, got) {
                        (Ok(w), Ok(out)) => {
                            assert_eq!(out128(&out), *w, "round {pol:?} ({q}, {r}, {d}) {mode:?}")
                        }
                        (Err(_), Err(90)) => {}
                        (Err(_), Err(c)) => panic!("expected 90, got {c}"),
                        (Ok(_), Err(c)) => panic!("unexpected fault {c}: round {pol:?} ({q}, {r}, {d})"),
                        _ => panic!("impossible combination"),
                    }
                }
            }
        }
    }


    /* ---- 6. rounding policies on a 256÷129 remainder ----
     *
     * Oracle: the interpreter's own `div_u256_by_u129_with_round`, fed a
     * numerator engineered as N = q·d + rem (rem < d), so its division
     * yields exactly (q, rem) and the policy tail decides — the same
     * (q, rem, d) the kernel's round helper consumes directly. Cases
     * whose product does not fit 256 bits are unreachable through a real
     * division and skipped. */

    #[test]
    fn u256_round_u129_matches_interpreter() {
        let policies = [
            Pol::Exact,
            Pol::Floor,
            Pol::Ceil,
            Pol::HalfUp,
            Pol::HalfEven,
        ];
        let mut rng = Rng(0x5EED);
        for &pol in &policies {
            for parity in [0u64, 1u64] {
                let mut cases: Vec<(u128, u128, u64, u128, u64)> = vec![
                    // (q, rem_lo, rem_hi, d_lo, d_hi)
                    (0, 0, 0, 1, 0),                    // rem 0
                    (u128::MAX - 1, 1, 0, 2, 0),        // round-up still fits
                    (u128::MAX, 1, 0, 2, 0),            // round-up carry → 90
                    // ties against the 129-bit divisor 2^128 (d_hi 1, d_lo 0):
                    // rem = 2^127 doubles to exactly 2^128
                    (4, 1u128 << 127, 0, 0, 1),         // even q stays (parity 0)
                    (5, 1u128 << 127, 0, 0, 1),         // odd q rounds up
                    (4, (1u128 << 127) + 1, 0, 0, 1),   // above half → up
                    (9, (1u128 << 127) - 1, 0, 0, 1),   // below half → stays
                    (7, 5, 1, 9, 1),                    // rem_hi = 1 (dbl_hi = 2)
                ];
                for _ in 0..30 {
                    let dhi = if rng.next() & 1 == 0 { 0u64 } else { 1u64 };
                    let dlo = rng.u128();
                    // keep rem < d so the engineered division is exact
                    let (rhi, rlo) = if dhi == 1 {
                        if rng.next() & 1 == 0 {
                            (0u64, if dlo == 0 { 0 } else { rng.u128() % dlo })
                        } else if dlo > 0 {
                            (1u64, rng.u128() % dlo)
                        } else {
                            (0u64, 0)
                        }
                    } else {
                        (0u64, if dlo == 0 { 0 } else { rng.u128() % dlo })
                    };
                    cases.push((rng.u128(), rlo, rhi, dlo, dhi));
                }
                for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                    for &(q, rl, rh, dl, dh) in &cases {
                        // N = q·d + rem as 256 bits (interpreter helpers)
                        let (hi1, lo1) = mul_wide_u128(q, dl);
                        let Some((n_hi, n_lo)) = add_u256(hi1, lo1, q * (dh as u128), 0)
                        else {
                            continue; // q·d alone overflows — unreachable face
                        };
                        let Some((n_hi, n_lo)) = add_u256(n_hi, n_lo, rh as u128, rl) else {
                            continue;
                        };
                        let want = div_u256_by_u129_with_round(
                            n_hi, n_lo, dh as u128, dl, pol, parity as u128, TEST_ERR,
                        );
                        let mut input = limbs128(q);
                        input.extend(limbs128(rl));
                        input.extend(limb8(rh));
                        input.extend(limbs128(dl));
                        input.extend(limb8(dh));
                        input.extend(limb8(parity));
                        // pool discipline: q(2) + rem_lo(2) + rem_hi(1) +
                        // d_lo(2) = 7 pool regs; d_hi and the parity live
                        // in the fixed S4/S5 (never pool, never scratch)
                        let got = run_kernel_out(mode, &input, 16, |lw| {
                            let mut off = 0;
                            let ps = load_pairs(lw, &mut off, 2); // q, rem_lo
                            let rh = lw.push_vreg();
                            lw.asm.ld(rh, S2, off);
                            off += 8;
                            let d = load_pairs(lw, &mut off, 1)[0]; // d_lo
                            lw.asm.ld(S4, S2, off); // d_hi
                            off += 8;
                            lw.asm.ld(S5, S2, off); // parity
                            for _ in 0..7 {
                                lw.pop_reg();
                            }
                            lw.emit_u129_round_quot(pol, ps[0], ps[1], rh, d, S4, Some(S5))
                                .unwrap();
                            sd128(lw, ps[0], 0);
                        });
                        match (&want, got) {
                            (Ok(w), Ok(out)) => assert_eq!(
                                out128(&out), *w,
                                "round129 {pol:?} (q={q}, rl={rl}, rh={rh}, dl={dl}, dh={dh}, p={parity}) {mode:?}"
                            ),
                            (Err(_), Err(90)) => {}
                            (Err(_), Err(c)) => panic!("expected 90, got {c}"),
                            (Ok(_), Err(c)) => panic!(
                                "unexpected fault {c}: round129 {pol:?} (q={q}, rl={rl}, rh={rh}, dl={dl}, dh={dh}, p={parity})"
                            ),
                                                    _ => panic!("impossible combination"),
                        }
                    }
                }
            }
        }
    }

    /* ---- 7. composite chain: mul_wide → add into den → 256÷129 → round
     *        (the MULDIV-den-add / wavg2 shape the T4/T26 tasks lower) --- */

    #[test]
    fn u256_mul_addden_div_round_chain_matches_interpreter() {
        let pol = Pol::HalfUp;
        let mut rng = Rng(0xC4A1);
        let mut cases: Vec<(u128, u128, u128, u128)> = vec![
            (0, 0, 1, 1),
            (1, 1, 1, 1),
            (u128::MAX, u128::MAX, u128::MAX, u128::MAX),
            (u128::MAX, 2, 0, 1),
            (1u128 << 100, 1u128 << 100, 1u128 << 30, 3),
        ];
        for _ in 0..40 {
            cases.push((rng.u128(), rng.u128(), rng.u128(), rng.u128()));
        }
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for &(x, y, base, rhs) in &cases {
                // interpreter expectation, composed from the ground-truth
                // helpers exactly like muldiv_den_addsub_checked's add arm
                let want: Result<u128, ()> = (|| {
                    let (hi, lo) = mul_wide_u128(x, y);
                    let (dhi, dlo) = add_u256_u128(0, base, rhs).ok_or(())?;
                    div_u256_by_u129_with_round(hi, lo, dhi, dlo, pol, 0, TEST_ERR).map_err(|_| ())
                })();
                let mut input = limbs128(x);
                input.extend(limbs128(y));
                input.extend(limbs128(base));
                input.extend(limbs128(rhs));
                let got = run_kernel_out(mode, &input, 16, |lw| {
                    // pool plan (8 regs): the divisor lives ACROSS the
                    // division (the round still needs it), so base(2) +
                    // dbit(1) take the BOTTOM slots and everything above
                    // is transient: [base, h0] + q(2) + r(2) + rh(1) = 8
                    // exactly; the product limbs are consumed into the
                    // workspace before the divmod helper pushes
                    let mut off = 32;
                    let den = load_pairs(lw, &mut off, 1)[0]; // base
                    let h0 = lw.push_vreg();
                    lw.asm.addi(h0, ZERO, 0);
                    let h1 = lw.push_vreg();
                    lw.asm.addi(h1, ZERO, 0);
                    let rhs = load_pairs(lw, &mut off, 1)[0];
                    lw.emit_u256_add_u128(&[den.0, den.1, h0, h1], rhs);
                    // pop transients top-first: rhs pair, then the dead
                    // den limb h1 — [base, h0] stays at the bottom
                    lw.pop_reg();
                    lw.pop_reg();
                    lw.pop_reg();
                    let mut off = 0;
                    let xy = load_pairs(lw, &mut off, 2); // x, y
                    let m = lw.emit_u256_mul_wide(xy[0], xy[1]).unwrap();
                    // the product limbs live on in xy's registers; free
                    // BOTH slots so the divmod pushes land there (the
                    // helper stages N into the workspace FIRST)
                    for _ in 0..4 {
                        lw.pop_reg();
                    }
                    lw.emit_u256_divmod_u129(true, &m, den, h0).unwrap();
                    let regs: Vec<u8> = lw.vstack.iter().map(|&l| loc_reg(l)).collect();
                    let q = (regs[regs.len() - 5], regs[regs.len() - 4]);
                    let r = (regs[regs.len() - 3], regs[regs.len() - 2]);
                    let rh = regs[regs.len() - 1];
                    for _ in 0..5 {
                        lw.pop_reg();
                    }
                    // den lo pair + h0 still hold the divisor untouched
                    lw.emit_u129_round_quot(pol, q, r, rh, den, h0, None).unwrap();
                    sd128(lw, q, 0);
                });
                match (&want, got) {
                    (Ok(w), Ok(out)) => {
                        assert_eq!(out128(&out), *w, "chain ({x}, {y}, {base}, {rhs}) {mode:?}")
                    }
                    (Err(()), Err(90)) => {}
                    (Err(()), Err(c)) => panic!("expected 90, got {c} on ({x}, {y}, {base}, {rhs})"),
                    (Ok(_), Err(c)) => panic!(
                        "unexpected fault {c}: chain ({x}, {y}, {base}, {rhs}) {mode:?}"
                    ),
                                    _ => panic!("impossible combination"),
                }
            }
        }
    }

    /* ---- 8. the u256 compare predicate (cmp_u256) ---- */

    #[test]
    fn u256_cmp_matches_interpreter() {
        let mut rng = Rng(0xC0FE);
        let mut cases: Vec<(u128, u128, u128, u128)> = vec![
            (0, 0, 0, 0),
            (0, 1, 0, 0),
            (0, 0, 0, 1),
            (1, 0, 0, u128::MAX),
            (u128::MAX, u128::MAX, u128::MAX, u128::MAX - 1),
            (0, u128::MAX, 1, 0),
            (0, 5, 0, 5),
            (3, 0, 2, u128::MAX),
        ];
        for _ in 0..40 {
            cases.push((rng.u128(), rng.u128(), rng.u128(), rng.u128()));
        }
        use std::cmp::Ordering;
        let ops: [(Bytecode, fn(std::cmp::Ordering) -> bool); 6] = [
            (Bytecode::LT, |o: Ordering| o.is_lt()),
            (Bytecode::LE, |o: Ordering| o.is_le()),
            (Bytecode::GT, |o: Ordering| o.is_gt()),
            (Bytecode::GE, |o: Ordering| o.is_ge()),
            (Bytecode::EQ, |o: Ordering| o.is_eq()),
            (Bytecode::NEQ, |o: Ordering| o.is_ne()),
        ];
        for &(op, f) in &ops {
            for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                for &(ahi, alo, bhi, blo) in &cases {
                    let want = f(cmp_u256(ahi, alo, bhi, blo));
                    let mut input = limbs128(alo);
                    input.extend(limbs128(ahi));
                    input.extend(limbs128(blo));
                    input.extend(limbs128(bhi));
                    let out = run_kernel_out(mode, &input, 8, |lw| {
                        let mut off = 0;
                        let ps = load_pairs(lw, &mut off, 4);
                        for _ in 0..8 {
                            lw.pop_reg();
                        }
                        // the 0/1 result goes to the fixed S4 (the 4 operand
                        // pairs already saturate the 8-reg pool)
                        lw.emit_u256_cmp(
                            op,
                            S4,
                            &[ps[0].0, ps[0].1, ps[1].0, ps[1].1],
                            &[ps[2].0, ps[2].1, ps[3].0, ps[3].1],
                        );
                        lw.asm.sd(S4, S3, 0);
                    })
                    .unwrap_or_else(|c| panic!("cmp must not fault, got {c}"));
                    assert_eq!(
                        out[0] != 0,
                        want,
                        "cmp {op:?} ({ahi}, {alo}) vs ({bhi}, {blo}) {mode:?}"
                    );
                }
            }
        }
    }
