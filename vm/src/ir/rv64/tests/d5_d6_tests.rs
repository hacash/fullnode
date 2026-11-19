//! D5/D6 (docs.loc/fitsh-rv64/survey/fin-control.md §5):
//! * D5 — the prologue's heap-region zeroing collapses into a counted
//!   5-instruction loop; write content (zero) and address order (ascending
//!   8-byte words over [heap_flag_off, heap_off + heap_bytes)) are
//!   byte-identical to the old static unroll of (8 + seg·SEGLEN)/8 sd
//!   stores. These tests pin the SHAPE (insts no longer grow ~32/segment)
//!   and the EXECUTION (the zero-filled block, its far end included, reads
//!   back exactly on the consensus interpreter in all three modes).
//! * D6 — a while body whose tail cannot fall out (break / continue /
//!   return / throw / abort) makes the trailing backedge `jal l_cond`
//!   physically unreachable; the emitter now drops it. The normal form
//!   keeps its backedge (its count is unchanged); a continue-tail body is
//!   BYTE-IDENTICAL to the normal form (the continue's own jal lands at
//!   exactly the backedge's site); break/return tails lose the dead jump.

use super::super::*;
use super::helpers::{compile, lower_mode, run_e2e_mode};
use crate::calc::{CalcFnObj, CalcRunLimits};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/* ==================================================================== */
/* D5: heap zeroing counted loop                                         */
/* ==================================================================== */

fn lower_heap(body: &str, mode: Rv64Mode) -> Rv64Lowered {
    let (ir, _) = compile(body);
    // the default arena (12 KiB) fits the 64 KiB pack envelope with the
    // run_e2e io ceilings; the heap block itself lives in the S0 frame
    // window, not the arena
    lower_body_mode_arena_heap(&ir, false, mode, ARENA_SIZE, SpaceCap::new(0).heap_grow_segment)
        .unwrap_or_else(|e| panic!("rv64 lowering failed (mode {mode:?}): {e}\nbody:\n{body}"))
}

/// Run a heap body through the packed-container consensus interpreter.
fn run_heap_mode(body: &str, mode: Rv64Mode) -> Vec<u8> {
    let lo = lower_heap(body, mode);
    let cap = SpaceCap::new(0);
    let container = match mode {
        Rv64Mode::Im => lo.pack_segments(&cap, 64, 5120, 1280),
        _ => lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC),
    }
    .unwrap_or_else(|e| panic!("pack failed (mode {mode:?}): {e}"));
    let fno = CalcFnObj::build_v1(container).unwrap();
    let exec = fno.run(&[], CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
    match exec.result {
        Ok(out) => out,
        Err(e) => panic!("kernel aborted: {e:?} exit={}", exec.exit_code),
    }
}

#[test]
fn d5_heap_zeroing_insts_slope_is_constant() {
    // The prologue's heap zeroing must not scale with the reserved
    // segment count: the counted loop is the same 5 instructions at seg=1
    // and seg=7, and the whole grow-statement delta (flag store + loop)
    // is a small constant. The old static unroll added 32 sd per segment
    // (delta 35 at seg=1 rising to 227 at seg=7 — every assertion below
    // would fail against it).
    let base = "return 0u64 as bytes";
    let segs = [1u8, 2, 4, 7];
    for mode in MODES {
        let base_insts = lower_heap(base, mode).stats.insts;
        let mut deltas = Vec::new();
        for &seg in &segs {
            let body = format!("heap_grow({seg})\nreturn 0u64 as bytes");
            let d = lower_heap(&body, mode).stats.insts - base_insts;
            assert!(
                d <= 9,
                "D5: heap_grow({seg}) delta {d} too large in {mode:?} — the zeroing regressed to a static unroll"
            );
            deltas.push(d);
        }
        let spread = *deltas.iter().max().unwrap() - *deltas.iter().min().unwrap();
        assert!(
            spread <= 2,
            "D5: inst growth across seg {:?} is {spread} in {mode:?} — the zeroing scales with the segment count",
            deltas
        );
    }
}

#[test]
fn d5_heap_zeroed_block_reads_back_golden() {
    // The loop writes exactly what the unroll wrote: ascending 8-byte
    // zero words over the whole reservation. Every mode must read the
    // golden bytes back through the consensus interpreter — the head of
    // the block, the far end (the loop's last stored word), and a write
    // that straddles a word boundary near the far end.
    for mode in MODES {
        // head of the block: 8 zero bytes at offset 0
        for seg in [1u8, 2, 4, 7] {
            let body = format!("heap_grow({seg})\nreturn heap_read(0, 8)");
            let out = run_heap_mode(&body, mode);
            assert_eq!(out, vec![0u8; 8], "mode {mode:?} seg {seg}: zero-fill head");
        }
        // far end of the block: the last 8 bytes of the reservation
        for seg in [1u8, 2, 4, 7] {
            let end = seg as usize * 256;
            let body = format!("heap_grow({seg})\nreturn heap_read({}, 8)", end - 8);
            let out = run_heap_mode(&body, mode);
            assert_eq!(out, vec![0u8; 8], "mode {mode:?} seg {seg}: zero-fill far end");
        }
        // a U8 write just before the far end, read back across the word
        // boundary: 4 zero bytes, the written byte, 3 more zeros
        for seg in [1u8, 2, 4, 7] {
            let w = seg as usize * 256 - 104;
            let body = format!(
                "heap_grow({seg})\nheap_write({w}, 171)\nreturn heap_read({}, 8)",
                w - 4
            );
            let out = run_heap_mode(&body, mode);
            assert_eq!(
                out,
                vec![0, 0, 0, 0, 171, 0, 0, 0],
                "mode {mode:?} seg {seg}: far-end write/read-back"
            );
        }
    }
}

/* ==================================================================== */
/* D6: while dead back-edge elimination                                  */
/* ==================================================================== */

#[test]
fn d6_continue_tail_is_byte_identical_to_normal_tail() {
    // Normal form keeps its backedge (its count is unchanged), and the
    // continue form's own `jal l_cond` lands at exactly the backedge's
    // site — with the dead duplicate removed the two streams are
    // byte-identical in every mode (before D6 the continue form carried
    // one extra jump).
    let normal = "var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n}\nreturn i as bytes";
    let cont = "var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    continue\n}\nreturn i as bytes";
    for mode in MODES {
        let a = lower_mode(normal, mode);
        let b = lower_mode(cont, mode);
        assert_eq!(
            (a.stats.insts, &a.code[..]),
            (b.stats.insts, &b.code[..]),
            "mode {mode:?}: continue-tail while must equal the normal form"
        );
        // both loop to completion: 3 as 8 BE bytes
        let out = run_e2e_mode(normal, &[], mode);
        assert_eq!(out, 3u64.to_be_bytes(), "mode {mode:?}: normal form result");
        let out = run_e2e_mode(cont, &[], mode);
        assert_eq!(out, 3u64.to_be_bytes(), "mode {mode:?}: continue form result");
    }
}

#[test]
fn d6_break_tail_drops_the_dead_backedge() {
    // The break form's tail emits one jump (to l_end) at the very site
    // the normal form's backedge occupies — same instruction count as
    // the normal form, one FEWER than pre-D6. Semantics: the break exits
    // on the first trip (i = 1).
    let normal = "var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n}\nreturn i as bytes";
    let brk = "var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    break\n}\nreturn i as bytes";
    for mode in MODES {
        let a = lower_mode(normal, mode);
        let b = lower_mode(brk, mode);
        assert_eq!(
            a.stats.insts, b.stats.insts,
            "mode {mode:?}: break tail must be exactly one jump like the normal backedge (pre-D6: +1)"
        );
        let out = run_e2e_mode(brk, &[], mode);
        assert_eq!(out, 1u64.to_be_bytes(), "mode {mode:?}: break form result");
    }
}

#[test]
fn d6_return_tail_drops_the_dead_backedge() {
    // A return-tail body cannot fall through, so its backedge is dead.
    // Pinned by difference against the same body with the return guarded
    // by an `if` (an IRIF tail CAN fall through, so the guarded variant
    // keeps its backedge): the guarded variant differs by EXACTLY the
    // guard's condition materialization (ld + addi + fused branch = 3
    // insts) plus the retained backedge jump — the D6 saving. Pre-D6 the
    // return-tail form also carried a backedge and the difference was 3.
    let ret = "var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    return i as bytes\n}\nreturn 9u64 as bytes";
    let guarded = "var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    if i < 4u64 {\n        return i as bytes\n    }\n}\nreturn 9u64 as bytes";
    for mode in MODES {
        let a = lower_mode(ret, mode).stats.insts;
        let b = lower_mode(guarded, mode).stats.insts;
        // guard materialization: Im/Imc load the frame slot (ld + addi +
        // fused branch); ImcRemap banks `i` in a register, so the load
        // disappears (addi + branch). The last term is always the
        // retained backedge — the D6 saving. Pre-D6 the diffs were 3/2.
        let want = match mode {
            Rv64Mode::ImcRemap => 3,
            _ => 4,
        };
        assert_eq!(
            b - a,
            want,
            "mode {mode:?}: guarded-return variant = return-tail + guard cond + backedge"
        );
        let out = run_e2e_mode(ret, &[], mode);
        assert_eq!(out, 1u64.to_be_bytes(), "mode {mode:?}: return-tail result");
        let out = run_e2e_mode(guarded, &[], mode);
        assert_eq!(out, 1u64.to_be_bytes(), "mode {mode:?}: guarded-return result");
    }
}

#[test]
fn d6_unreachable_backedge_predicate_four_tails() {
    // The D6 gate is `ends_in_unconditional_jump` on the while body —
    // the same proof the if-join and the fallthrough exit use. Pin its
    // verdict on the four tail shapes straight on the compiled IR.
    let cases: [(&str, bool); 5] = [
        ("var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n}", false),
        ("var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    break\n}", true),
        ("var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    continue\n}", true),
        ("var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    return i as bytes\n}", true),
        ("var i = 0u64\nwhile i < 3u64 {\n    i += 1u64\n    throw 7u64\n}", true),
    ];
    for (src, want) in cases {
        let (ir, _) = compile(src);
        let mut found = 0;
        for n in &ir.subs {
            if !matches!(Bytecode::try_from_u8(n.bytecode()), Ok(Bytecode::IRWHILE)) {
                continue;
            }
            let w = n
                .as_any()
                .downcast_ref::<IRNodeDouble>()
                .expect("while node shape");
            assert_eq!(
                ends_in_unconditional_jump(&*w.suby),
                want,
                "ends_in_unconditional_jump verdict for:\n{src}"
            );
            found += 1;
        }
        assert_eq!(found, 1, "exactly one while expected in:\n{src}");
    }
}
