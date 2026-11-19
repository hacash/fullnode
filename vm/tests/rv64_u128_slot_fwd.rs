//! C7 (impl/cat-u128fwd): u128-slot store→load forwarding.
//!
//! A u128 slot stores a (lo, hi) register pair into its 16-byte frame cell
//! (stmt.rs PUT wide arm); every read used to pay a fixed 2×ld. C7 forwards
//! the pair through the fwd_bytes PAIR table — a u128 slot is SlotTy::Int
//! (join_slot_widths records widths for Int slots only) and is never banked
//! (web.rs), so no other producer can write that entry, and the pair kill
//! discipline is exactly the fwd_bytes one: push_vreg re-hand (kill_fwd_reg)
//! and place() label barriers clear it wholesale.
//!
//! Every test here compares against the INTERPRETER (four-way: interp +
//! rv64 Im/Imc/ImcRemap) and pins the activation (−2 insts per forwarded
//! read: the fixed `ld lo` + `ld hi` pair disappears).

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{frontend, Interp, Rv64};
use vm::ir::rv64::{lower_body_mode_arena, Rv64Mode};

/// Four-way agreement: interpreter value/fault == all three rv64 modes.
/// `output_max` > 16 rides the bigout envelope (28 KiB arena — the 64 KiB
/// chain mem cap cannot hold the full arena with the larger output).
fn agree4_out(body: &str, inputs: &[Vec<u8>], output_max: u16) {
    let arena = if output_max > 16 { 0x7000 } else { vm::ir::rv64::ARENA_SIZE_MAX };
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    let rv = Rv64::try_build_out(body, output_max, arena)
        .unwrap_or_else(|e| panic!("{body}: rv64 build: {e}"));
    for input in inputs {
        let want = it.run(input);
        for (mode, got) in rv.run(input) {
            match (&want, &got) {
                (Ok(w), Ok(g)) => assert_eq!(w, g, "{body} [{mode:?}] input={input:?}"),
                (Err(w), Err(code)) => {
                    assert_eq!(
                        w.code as u64, *code,
                        "{body} [{mode:?}] input={input:?}: interp={want:?} rv64={got:?}"
                    )
                }
                _ => panic!("{body} [{mode:?}] input={input:?}: interp={want:?} rv64={got:?}"),
            }
        }
    }
}

fn agree4(body: &str, inputs: &[Vec<u8>]) {
    agree4_out(body, inputs, 16);
}

fn insts_of(body: &str) -> usize {
    let (ir, _) = frontend(body).unwrap_or_else(|e| panic!("{body}: frontend: {e}"));
    let lo = lower_body_mode_arena(&ir, true, Rv64Mode::Im, vm::ir::rv64::ARENA_SIZE_MAX)
        .unwrap_or_else(|e| panic!("{body}: lower: {e}"));
    lo.stats.insts
}

/// A u128-width slot producer (corpus g9.compound.u128 form: slot_w joins
/// to 128, the value is (hi = byte0, lo = byte1 · 2^8 ..) in the pair).
const U: &str = "var x = (byte(data, 0) as u128) * 18446744073709551616u128";

fn inputs3() -> Vec<Vec<u8>> {
    vec![vec![1, 2, 3, 4, 5, 6, 7, 8], vec![255, 0, 7, 9, 3, 1, 4, 1], vec![0, 0, 0, 1, 0, 9, 9, 9]]
}

/* ---- activation: the forwarded reads really leave the stream ---- */

/// Cascade pin: with PUT x followed by GET-only statements, EVERY GET of x
/// hits (each hit pushes refs — no register hand — and each pair PUT
/// re-establishes the entry), so the bodies differ ONLY by the one extra
/// pair store (2×sd). A broken hit shows up as +4 more (the 2×ld the miss
/// reintroduces).
#[test]
fn c7_cascade_hits_cost_two_stores() {
    let a = insts_of(&format!("{U}\nvar w = x\nreturn w as bytes"));
    let c = insts_of(&format!("{U}\nvar y = x\nvar z = x\nreturn z as bytes"));
    assert_eq!(c - a, 2, "cascade pair-forward delta {c}-{a} != 2");
}

/// Barrier pins: an intervening statement whose evaluation hands a pool
/// register (the byte() read) kills the entry — the read must reload
/// (2×ld back). The body pair differs by the killing statement itself
/// (its eval) plus the reintroduced load pair, so the delta is ≥ 3.
#[test]
fn c7_intervening_write_kills_forward() {
    let a = insts_of(&format!("{U}\nvar w = x\nreturn w as bytes"));
    let b = insts_of(&format!(
        "{U}\nvar t = byte(data, 1)\nvar w = x\nreturn w as bytes"
    ));
    assert!(b - a >= 3, "kill-then-read delta {b}-{a} < 3");
}

/// The u128 XOP paths never forward (they stage the old pair with
/// push_vreg, which kills any live entry, and re-store without setting a
/// new one — the pre-C7 behavior is unchanged), and the divmod-routine
/// path agrees with the interpreter.
#[test]
fn c7_xop_then_read_agrees() {
    agree4(
        "var x = (byte(data, 0) as u128) * 18446744073709551616u128\nx += 5u128\nreturn x as bytes",
        &inputs3(),
    );
    agree4(
        "var x = (byte(data, 0) as u128) * 18446744073709551616u128\nx /= 3u128\nreturn x as bytes",
        &inputs3(),
    );
    agree4(
        "var x = (byte(data, 0) as u128) * 18446744073709551616u128\nx *= 3u128\nreturn x as bytes",
        &inputs3(),
    );
}

/* ---- differential: every forwarding-relevant shape, four-way ---- */

#[test]
fn c7_u128_pair_fwd_differential() {
    // direct hit: GET immediately after the PUT (empty stack, bottom pair)
    agree4(&format!("{U}\nreturn x as bytes"), &inputs3());

    // cascade: two forwarded reads of the same pair, plus a re-forwarding
    // pair PUT in between
    agree4(
        &format!("{U}\nvar y = x\nvar z = x\nreturn ((z as bytes))"),
        &inputs3(),
    );

    // middle write of ANOTHER u128 slot: the kill discipline must reload x
    agree4(
        &format!(
            "{U}\nvar t = (byte(data, 2) as u128) + 1u128\nreturn ((x + t) as bytes)"
        ),
        &inputs3(),
    );

    // re-hand between PUT and GET (t's evaluation pushes): miss, reload
    agree4(
        &format!(
            "{U}\nvar t = (byte(data, 2) as u128)\nvar w = x\nreturn ((w + t) as bytes)"
        ),
        &inputs3(),
    );

    // label barrier: the if join's place() kills the entry wholesale
    agree4(
        &format!(
            "{U}\nif byte(data, 1) == 1 {{ var q = (byte(data, 2) as u128) }}\nvar w = x\nreturn w as bytes"
        ),
        &inputs3(),
    );

    // while barrier: header/join places kill; the loop body re-runs the
    // pair store every iteration
    agree4(
        &format!(
            "{U}\nwhile byte(data, 1) == 255 {{ x = x + 1u128 }}\nvar w = x\nreturn w as bytes"
        ),
        &inputs3(),
    );

    // coexistence: the u128 pair entry lives next to a forwarded bytes pair
    // and a forwarded int slot; every shape keeps interpreter agreement
    // (big-output channel, 28 KiB arena)
    agree4_out(
        &format!(
            "{U}\nvar b = buf_left(2, data)\nvar i = (byte(data, 5) as u64)\nvar w = x\nreturn ((w + i) as bytes) ++ b"
        ),
        &inputs3(),
        32,
    );

    // coexistence with the bytes fwd of the SAME statement stream: the
    // bytes PUT between the pair PUT and its read kills only its own shape
    agree4_out(
        &format!(
            "{U}\nvar b = buf_left(2, data)\nvar w = x\nreturn b ++ (w as bytes)"
        ),
        &inputs3(),
        32,
    );
}

/// The may-Nil tag machine on the u128 GET arm (nil_wtag_from_flag on the
/// LOW entry) is structurally unreachable for slots of the accepted subset:
/// a u128-joined slot's every write is width-128 and non-Nil (a nil write
/// into a u128-joined slot falls outside the translatable kernel subset —
/// the pre-existing wide-PUT lowering does not admit a 1-entry nil source),
/// so only the non-tracked face exists and both forwarding paths (hit and
/// reload) share the identical (absent) tag shape. Nothing to pin here —
/// the tag machinery is byte-identical on both sides of the C7 hit branch.
#[test]
fn c7_u128_pair_fwd_differential_bigout() {
    // big-output coexistence shapes ride the bigout envelope (output 32
    // over a 28 KiB arena — the 64 KiB chain mem cap cannot hold the full
    // arena with the larger output channel)
    agree4_out(
        &format!(
            "{U}\nvar b = buf_left(2, data)\nvar i = (byte(data, 5) as u64)\nvar w = x\nreturn ((w + i) as bytes) ++ b"
        ),
        &inputs3(),
        32,
    );
}
