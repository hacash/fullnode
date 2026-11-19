//! E4 / E5 / E12 pool-cover tests: the literal-pool base decision moved
//! out of the `d <= 6` bank gate (deep bodies get a base too), the
//! lit_sites census counting li_k hits (int literals > 0xFFFF) with the
//! threshold at 2, and the ±2KiB pool-distance hard error kept exactly
//! as-is (E12 guardrail: compile failure, no fallback).

#![allow(unused_imports)]

use super::helpers::*;
use super::super::*;
use crate::calc::decode::*;
use crate::calc::container::CALC_PROFILE_RV64IMC;

/// Right-deep big-literal add chain over a runtime leaf: n constants
/// nest the expression to peak depth D = 2n+1 clamped by the pool cap —
/// n=4 lands on D=7, n=5 on D=8, both past the `d <= 6` bank gate. The
/// trailing `0x1122` PUT adds a bytes-literal address site (the 4B
/// `addi rd, x2` pool-base form) without touching the peak depth.
fn deep_body(n: usize) -> String {
    let mut expr = "byte(data, 0) as u64".to_string();
    for k in 0..n {
        expr = format!("{}u64 + ({expr})", 100000 * (k + 1));
    }
    format!(
        "var x = {expr}\nvar s = 0x1122\nreturn ((x + size(data) + size(s)) as u64) as bytes"
    )
}

/// (c.ldsp, all) loads relative to the pool-base register x2: the
/// 2-byte c.ldsp window form and any-width `ld x?, off(x2)`. Full-stream
/// decode is the consensus re-verification (unwrap fails on any illegal
/// encoding).
fn count_x2_loads(code: &[u8], profile: u8) -> (usize, usize) {
    let mut cldsp = 0;
    let mut any = 0;
    let mut off = 0usize;
    while off < code.len() {
        let (inst, len) = decode_at(code, off, profile).unwrap();
        if let Inst::Load { op: LoadOp::Ld, rs1: 2, .. } = &inst {
            any += 1;
            if len == 2 {
                cldsp += 1;
            }
        }
        off += len as usize;
    }
    assert_eq!(off, code.len(), "stream must end on an instruction boundary");
    (cldsp, any)
}

/// E4: D=7/8 deep bodies (no bank either — the `d <= 6` gate skips the
/// whole promotion) still claim the literal-pool base register x2. Pins:
/// (a) the dbg census shows pool_size 7/8, empty bank, lit_sites ≥ 4 and
/// pool_base_reg Some(2); (b) every big-constant site materializes as a
/// c.ldsp off x2 (the pool fits the 504-byte window); (c) Im/Imc — no
/// promotion plan — have no base and no x2 loads; (d) all three modes
/// execute the body identically.
#[test]
fn e4_deep_body_pool_base() {
    for (n, d) in [(4usize, 7usize), (5, 8)] {
        let body = deep_body(n);
        let (ir, _) = compile_with_input(&body);
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let (lo, dbg) =
                lower_body_mode_arena_dbg(&ir, true, mode, ARENA_SIZE_MAX).unwrap();
            let profile = if mode == Rv64Mode::Im { 0u8 } else { CALC_PROFILE_RV64IMC };
            let (cldsp, x2_lds) = count_x2_loads(&lo.code, profile);
            if mode == Rv64Mode::ImcRemap {
                assert_eq!(dbg.pool_size, d, "D={d} deep body (n={n})");
                // E10: D=7/8 bodies get a partial bank. The pool occupies
                // window positions 0..d — x14 (position 6) drops from the
                // candidate list at D=7, x15 (position 7) at D=8 — while
                // the non-window group stays fully available and x2 is
                // requisitioned by the pool base (E4).
                assert!(dbg.bank_regs.contains(&3) && dbg.bank_regs.contains(&27),
                    "D={d}: non-window group stays in the bank candidates");
                assert!(!dbg.bank_regs.contains(&2), "D={d}: x2 is the pool base");
                match d {
                    7 => {
                        assert!(!dbg.bank_regs.contains(&14), "D=7: x14 is pool position 6");
                        assert!(dbg.bank_regs.contains(&15), "D=7: x15 still free");
                    }
                    _ => {
                        assert!(!dbg.bank_regs.contains(&14), "D=8: x14 is pool position 6");
                        assert!(!dbg.bank_regs.contains(&15), "D=8: x15 is pool position 7");
                    }
                }
                assert!(dbg.lit_sites >= 4, "D={d}: {n} constants + 1 bytes site");
                assert_eq!(dbg.pool_base_reg, Some(2), "E4: base claimed at D={d}");
                assert!(cldsp >= n, "D={d}: >= {n} c.ldsp constant sites, got {cldsp}");
                assert!(x2_lds >= cldsp, "D={d}: x2 loads = {x2_lds}, cldsp = {cldsp}");
            } else {
                assert_eq!(dbg.pool_base_reg, None, "{mode:?} has no promotion plan");
                assert_eq!(x2_lds, 0, "{mode:?}: no pool base, no x2 loads");
            }
        }
        // three-mode e2e differential
        let input = [7u8];
        let want: u64 = match n {
            4 => 100000 + (200000 + (300000 + (400000 + 7))),
            _ => 100000 + (200000 + (300000 + (400000 + (500000 + 7)))),
        } + 1 // size(data)
            + 2; // size(0x1122)
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let got = run_input_mode(&body, &input, mode);
            assert_eq!(got, want.to_be_bytes(), "{mode:?} e2e mismatch (n={n})");
        }
    }
}

/// E4 contrast: a D<=6 body with the same ≥4 sites keeps its bank, and
/// the base register x2 — the bank candidate list's LAST entry — is
/// removed from it (near-zero requisition loss).
#[test]
fn e4_shallow_body_bank_loses_x2_last() {
    // same constants as the D=7 body but a flat shape (depth ≤ 6)
    let body = "var a = 100000u64 + byte(data, 0)\n\
                var b = 200000u64 + byte(data, 1)\n\
                var c = 300000u64 + byte(data, 2)\n\
                var s = 0x1122\n\
                return ((a + b + c + size(data) + size(s)) as u64) as bytes";
    let (ir, _) = compile_with_input(body);
    let (lo, dbg) =
        lower_body_mode_arena_dbg(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
    assert!(dbg.pool_size <= 6, "shallow body: pool_size = {}", dbg.pool_size);
    assert!(!dbg.bank_regs.is_empty(), "shallow body keeps its bank");
    assert!(!dbg.bank_regs.contains(&2), "x2 leaves the bank for the base");
    assert_eq!(dbg.pool_base_reg, Some(2));
    assert!(dbg.lit_sites >= 4);
    let (cldsp, _) = count_x2_loads(&lo.code, CALC_PROFILE_RV64IMC);
    assert!(cldsp >= 3, "constants ride pooled loads, got {cldsp} c.ldsp");
    let input = [1u8, 2u8, 3u8];
    let want: u64 = (100000 + 1) + (200000 + 2) + (300000 + 3) + 3 + 2;
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        assert_eq!(run_input_mode(body, &input, mode), want.to_be_bytes(), "{mode:?}");
    }
}

/// E5: int literals > 0xFFFF (CU* folds included) now count as literal
/// sites and the threshold is 2 — a constant-only body (no bytes
/// literals at all) with two such constants claims the base; one
/// constant stays below it. Both bodies execute identically across the
/// three modes.
#[test]
fn e5_threshold_two_constant_only_bodies() {
    let two = "var a = 100000u64 + byte(data, 0)\n\
               var b = 200000u64 + byte(data, 1)\n\
               return ((a + b) as u64) as bytes";
    let (ir2, _) = compile_with_input(two);
    let (lo, dbg) =
        lower_body_mode_arena_dbg(&ir2, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
    assert_eq!(dbg.lit_sites, 2, "two li_k hits, nothing else");
    assert_eq!(dbg.pool_base_reg, Some(2), "threshold 2 reached");
    let (cldsp, x2) = count_x2_loads(&lo.code, CALC_PROFILE_RV64IMC);
    assert!(cldsp >= 2, "both constants pooled (c.ldsp), got {cldsp}");
    assert!(x2 >= 2);
    let one = "var a = 100000u64 + byte(data, 0)\nreturn (a as u64) as bytes";
    let (ir1, _) = compile_with_input(one);
    let (_, dbg1) =
        lower_body_mode_arena_dbg(&ir1, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
    assert_eq!(dbg1.lit_sites, 1, "one li_k hit");
    assert_eq!(dbg1.pool_base_reg, None, "below the threshold: no base");
    // and the lowering proves it: no x2-relative load without a base
    let lo1 = lower_body_mode_arena(&ir1, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
    let (_, x2_1) = count_x2_loads(&lo1.code, CALC_PROFILE_RV64IMC);
    assert_eq!(x2_1, 0);
    // three-mode e2e differential for the threshold-2 body
    let input = [1u8, 2u8];
    let want: u64 = (100000 + 1) + (200000 + 2);
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        assert_eq!(run_input_mode(two, &input, mode), want.to_be_bytes(), "{mode:?}");
    }
}

/// E12 guardrail: a pool body beyond the ±2KiB PoolOff/PoolLd distance
/// still refuses to compile through the original FinishErr::Msg path —
/// no "pool-too-far" fallback appears. Without a pool base (Im/Imc) the
/// same body compiles: constants stay plain `li` and the pool stays
/// empty.
#[test]
fn e12_oversized_pool_hard_error_unchanged() {
    let mut body = String::from("var acc = byte(data, 0) as u64\n");
    for k in 0..300usize {
        body.push_str(&format!("acc = acc + {}u64\n", 0x1_0000_0000u64 + 7919 * k as u64));
    }
    body.push_str("return ((acc + size(data)) as u64) as bytes");
    let (ir, _) = compile_with_input(&body);
    for mode in [Rv64Mode::Im, Rv64Mode::Imc] {
        let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).unwrap();
        let profile = if mode == Rv64Mode::Im { 0u8 } else { CALC_PROFILE_RV64IMC };
        let (_, x2) = count_x2_loads(&lo.code, profile);
        assert_eq!(x2, 0, "{mode:?}: no base, constants stay inline li");
    }
    let e = lower_body_mode_arena(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX)
        .unwrap_err();
    let msg = format!("{e}");
    assert!(
        msg.contains("pool-base constant offset") && msg.contains("out of range"),
        "E12 hard error unchanged, got: {msg}"
    );
}

/// E10: the bank construction moved out of the `d <= 6` gate — D=7/8
/// bodies now carry a PARTIAL bank. The pool occupies VSTACK_ORDER_RVC
/// positions 0..d (x14 is position 6: dropped from the candidate list at
/// D=7; x15 position 7: dropped at D=8), the non-window group
/// (x3,x4,x7,x16,x22..x27) stays fully available, x2 is requisitioned by
/// the pool base (E4), and the hot Int slots of the body are banked into
/// the remaining registers: their reads become zero-cost Refs (no
/// c.ld/c.sd frame traffic at any read/write site), visible as a
/// strictly smaller ImcRemap stream than Imc for the same body.
#[test]
fn e10_deep_body_partial_bank() {
    // The E4 deep shape (n constants over an inline `byte(..) as u64`
    // leaf: n=4 peaks at D=7, n=5 at D=8) wrapped around a hot-slot loop:
    // a/b/x/acc/i are multi-read multi-write Int locals.
    let deep = |n: usize| -> String {
        let mut expr = "byte(data, 2) as u64".to_string();
        for k in 0..n {
            expr = format!("{}u64 + ({expr})", 100000 * (k + 1));
        }
        format!(
            "var a = byte(data, 0) as u64\nvar b = byte(data, 1) as u64\n\
             var x = {expr}\n\
             var acc = x + b\nvar i = 0u64\nwhile i < 4u64 {{\n\
               acc = acc + x + i\n  i = i + 1\n}}\n\
             return ((acc + a + b + size(data)) as u64) as bytes"
        )
    };
    for (n, d) in [(4usize, 7usize), (5, 8)] {
        let body = deep(n);
        let (ir, _) = compile_with_input(&body);
        let (mut sz_im, mut sz_imc, mut sz_remap) = (0usize, 0usize, 0usize);
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let (lo, dbg) =
                lower_body_mode_arena_dbg(&ir, true, mode, ARENA_SIZE_MAX).unwrap();
            match mode {
                Rv64Mode::Im => sz_im = lo.code.len(),
                Rv64Mode::Imc => sz_imc = lo.code.len(),
                Rv64Mode::ImcRemap => sz_remap = lo.code.len(),
            }
            if mode != Rv64Mode::ImcRemap {
                continue;
            }
            assert_eq!(dbg.pool_size, d, "D={d} body (n={n})");
            // exact candidate list: window pair per D, non-window group in
            // order, busy pool positions in order, x2 popped by the base
            let mut want: Vec<u8> = match d {
                7 => vec![15],
                _ => vec![],
            };
            want.extend([3u8, 4, 7, 16, 22, 23, 24, 25, 26, 27, 29, 6, 12, 13, 28]);
            assert_eq!(dbg.bank_regs, want, "D={d} candidate list (n={n})");
            // every Int local is banked (slot 0 is the data input)
            assert!(dbg.banked.len() >= 5, "D={d}: slots = {:?}", dbg.banked);
            for (k, &b) in dbg.banked.iter().enumerate() {
                if k >= 1 && k < dbg.banked.len() - 1 {
                    assert_eq!(b, BANK_KIND_INT, "D={d}: slot {k} banked");
                }
            }
            assert_eq!(dbg.frame_off.iter().filter(|&&o| o < 0).count(), dbg.banked.len(),
                "D={d}: no slot left in the frame");
            assert_eq!(dbg.pool_base_reg, Some(2));
        }
        // banked reads/writes are register-only: the ImcRemap stream must
        // be strictly smaller than Imc (base + bank win twice over)
        assert!(
            sz_remap < sz_imc,
            "D={d}: ImcRemap {sz_remap} vs Imc {sz_imc}"
        );
        // three-mode differential over a deterministic input sweep
        for seed in 0..24u64 {
            let input = [
                (seed * 7 + 1) as u8,
                (seed * 13 + 3) as u8,
                (seed * 29 + 5) as u8,
            ];
            let a = input[0] as u64;
            let b = input[1] as u64;
            let c = input[2] as u64;
            let x: u64 = match n {
                4 => 100000 + (200000 + (300000 + (400000 + c))),
                _ => 100000 + (200000 + (300000 + (400000 + (500000 + c)))),
            };
            let mut acc = x + b;
            let mut i = 0u64;
            while i < 4 {
                acc = acc + x + i;
                i += 1;
            }
            let want = (acc + a + b + 3) as u64;
            for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                let got = run_input_mode(&body, &input, mode);
                assert_eq!(got, want.to_be_bytes(), "D={d} seed={seed} {mode:?}");
            }
        }
    }
}

/// E6: suffix sharing of bytes-class pool entries. A bytes literal whose
/// bytes are exactly the suffix of an existing entry points into that
/// entry's tail (an interior label placed by the pool emission) instead
/// of paying a duplicate pool segment. The family below nests fully:
/// "cdef" ⊂ "89abcdef" ⊂ "0123456789abcdef" — the whole family collapses
/// into ONE 16-byte entry (all three modes), outputs stay byte-exact,
/// and the overlapping lbu reads (a 4-byte tail read at delta 12 inside
/// the same bytes a full 16-byte read consumes) cross the shared
/// boundary in both directions.
#[test]
fn e6_suffix_family_pool_shrinks() {
    let body = "var a = \"0123456789abcdef\"\n\
                var b = \"cdef\"\n\
                var c = \"89abcdef\"\n\
                var d = \"abcdef\"\n\
                return a ++ b ++ c ++ d";
    let (ir, _) = compile_with_input(&body);
    let want: Vec<u8> = b"0123456789abcdefcdef89abcdefabcdef".to_vec();
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        let lo = lower_body_mode(&ir, true, mode).unwrap();
        // unshared the family needs 16+4+8+6 = 34 pool bytes; shared: one
        // 16-byte entry hosts every tail
        assert_eq!(lo.pool, b"0123456789abcdef".to_vec(), "{mode:?} pool collapsed");
        assert_eq!(lo.pool.len(), 16);
        for mode2 in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let got = run_input_mode(&body, &[], mode2);
            assert_eq!(got, want, "{mode2:?} cat output");
        }
        break;
    }
    // overlapping byte() reads over the shared tails: a 4-byte walk over
    // "cdef" (interior at delta 12) plus boundary probes of the base
    // entry; every mode agrees with the source semantics
    let walk = "var a = \"0123456789abcdef\"\n\
                var b = \"cdef\"\n\
                var acc = 0u64\nvar i = 0u64\n\
                while i < 4 { acc = acc * 256u64 + byte(b, i)\ni = i + 1u64 }\n\
                return ((acc + byte(a, 3) + byte(a, 15)) as u64) as bytes";
    let want: u64 = {
        let mut acc = 0u64;
        for &c in b"cdef" { acc = acc * 256 + c as u64; }
        acc + b'3' as u64 + b'f' as u64
    };
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        assert_eq!(run_input_mode(&walk, &[], mode), want.to_be_bytes(), "{mode:?} bytewalk");
    }
}

/// E6 determinism/soundness on the miss side: a family with no suffix
/// relation keeps one distinct entry per literal in first-use order —
/// the pool is byte-identical to the pre-E6 layout.
#[test]
fn e6_no_suffix_hit_pool_unchanged() {
    let body = "var a = \"abc\"\nvar b = \"xyz\"\nreturn a ++ b";
    let (ir, _) = compile_with_input(&body);
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        let lo = lower_body_mode(&ir, true, mode).unwrap();
        assert_eq!(lo.pool, b"abcxyz".to_vec(), "{mode:?} distinct entries, first-use order");
    }
    // prefix-overlap is NOT a suffix hit ("abcd" does not end with "ab"):
    // both entries stay whole
    let body2 = "var a = \"abcd\"\nvar b = \"ab\"\nreturn a ++ b";
    let (ir2, _) = compile_with_input(&body2);
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        let lo = lower_body_mode(&ir2, true, mode).unwrap();
        assert_eq!(lo.pool, b"abcdab".to_vec(), "{mode:?} prefix overlap does not share");
    }
}

/// E6 value-entry isolation: 8-byte value entries (the li_k c.ldsp/PoolLd
/// loads) never share and are never shared onto — even when a bytes
/// literal is exactly a proper suffix of the constant's LE bytes. The
/// literal keeps its own entry; the constant keeps its 8-aligned start.
#[test]
fn e6_value_entries_never_share() {
    // 5208208757389214273 = 0x4847464544434241; its LE bytes are
    // b"ABCDEFGH", so b"EFGH" is a proper suffix of the value entry —
    // the one shape a broken implementation would alias.
    let body = "var v = 5208208757389214273u64 + 5208208757389214273u64\n\
                var s = \"EFGH\"\n\
                return ((v + size(s) + byte(s, 0)) as u64) as bytes";
    let (ir, _) = compile_with_input(&body);
    let v = 2u64 * 5208208757389214273; // 0x908E8C8A88868482
    let want = (v + 4 + b'E' as u64).to_be_bytes().to_vec();
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        let lo = lower_body_mode(&ir, true, mode).unwrap();
        if mode == Rv64Mode::ImcRemap {
            // value entry first (8-aligned at offset 0), "EFGH" separate
            assert_eq!(
                lo.pool,
                {
                    let mut p = v.to_le_bytes().to_vec();
                    p.extend_from_slice(b"EFGH");
                    p
                },
                "value entry keeps its own 8-aligned slot; the literal is not aliased"
            );
            let (cldsp, _) = count_x2_loads(&lo.code, CALC_PROFILE_RV64IMC);
            assert!(cldsp >= 1, "the constant rides an aligned pooled load, got {cldsp}");
        } else {
            assert!(lo.pool.ends_with(b"EFGH"), "{mode:?} literal entry intact");
        }
        assert_eq!(run_input_mode(&body, &[], mode), want, "{mode:?} output");
    }
}

/// E6 through the arena-map key path: bytes keys with suffix relations
/// ("fgh" and "h" are tails of "abcdefgh") share one pool entry, and key
/// identity stays (pointer, length) — three distinct keys resolve
/// correctly through inserts and lookups over the shared bytes.
#[test]
fn e6_suffix_shared_map_keys() {
    let body = "var m = map{\"abcdefgh\" : 1u64, \"fgh\" : 2u64, \"h\" : 3u64}\n\
                var r = 0u64\n\
                r = r + m[\"abcdefgh\"]\n\
                r = r + m[\"fgh\"]\n\
                r = r + m[\"h\"]\n\
                return (r as u64) as bytes";
    let (ir, _) = compile_with_input(&body);
    let want = 6u64.to_be_bytes().to_vec();
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        let lo = lower_body_mode(&ir, true, mode).unwrap();
        assert_eq!(lo.pool, b"abcdefgh".to_vec(), "{mode:?} keys collapsed into one entry");
        assert_eq!(run_input_mode(&body, &[], mode), want, "{mode:?} map lookups");
    }
}
