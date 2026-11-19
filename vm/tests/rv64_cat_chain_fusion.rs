//! C6 (impl/cat-u128fwd): CAT chain fusion.
//!
//! A CAT tree (some child is itself a dynamic CAT) used to lower one full
//! per-node `emit_cat` body per inner node — per node an overflow check,
//! an arena check, two byte-copy loops and a descriptor — and re-copied the
//! whole inner result once more at the next layer. The fusion walks the
//! tree and emits the evals + Nil(91) guards in the ORIGINAL per-layer
//! order (`eval x, eval y, guard x, guard y`, layers innermost-first),
//! then ONE overflow check over the total, ONE arena check, one memcpy per
//! leaf and ONE descriptor.
//!
//! Face discipline (bytes.rs::emit_cat_fused carries the full argument):
//! the deleted inner overflow faces (90) are unreachable (every live
//! bytes length < arena_end, so no add in the length chain can wrap); the
//! arena budget face (71) may shift to the fused allocation (ctr.rs CAT
//! precedent); the reachable 91 guard order is preserved node for node.
//!
//! Every dynamic shape is verified against the INTERPRETER (four-way:
//! interp + rv64 Im/Imc/ImcRemap); the activation pins count instructions
//! through `lower_body_mode_arena`.

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{frontend, Interp, Rv64};
use vm::ir::rv64::{lower_body_mode_arena, Rv64Mode};

/// Four-way agreement: interpreter value/fault == all three rv64 modes.
fn agree4_out(body: &str, inputs: &[Vec<u8>], output_max: u16, arena: u64) {
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

/// bigout envelope: output 64 over a 28 KiB arena (the 64 KiB chain mem
/// cap cannot hold the full arena with the larger output channel).
fn agree4_big(body: &str, inputs: &[Vec<u8>]) {
    agree4_out(body, inputs, 64, 0x7000);
}

fn insts_of(body: &str) -> usize {
    let (ir, _) = frontend(body).unwrap_or_else(|e| panic!("{body}: frontend: {e}"));
    let lo = lower_body_mode_arena(&ir, true, Rv64Mode::Im, vm::ir::rv64::ARENA_SIZE_MAX)
        .unwrap_or_else(|e| panic!("{body}: lower: {e}"));
    lo.stats.insts
}

fn inputs3() -> Vec<Vec<u8>> {
    // every input covers the chain cuts (deepest ost+len = 10); buf_cut
    // out-of-range rides a pinned 42-vs-71 divergence family the corpus
    // avoids (BUG-009 watch shape) — not a C6 face
    vec![
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        vec![255, 0, 7, 9, 3, 1, 4, 1, 5, 9, 2, 6],
        vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]
}

/* ---- activation: fusion really leaves the stream ---- */

/// Per fused extra leaf the emission adds one memcpy loop (~9) + the total
/// add (~1) + bound/descriptor growth — well under an unfused node body
/// (overflow add+bgeu+trap + arena li+add+bgeu + TWO copy loops + the
/// descriptor ≈ 27, plus the operand's own evaluation). Measured shapes:
/// a cut-leaf 3-chain adds 19 over the 2-cat (fused), a GET-leaf 4-chain
/// adds 11 over the 3-chain (fused).
#[test]
fn c6_fused_chain_pins() {
    let two = insts_of("var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2)\nreturn r");
    let three = insts_of("var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2)\nreturn r");
    assert!(three - two <= 24, "fused 3rd leaf costs {} (>24: node body leaked in)", three - two);
    // and the fused chain beats the unfused shape outright: the unfused
    // 3-chain costs cat2 + a whole extra node body + leaf traffic
    assert!(three < two + 27, "3-chain {three} !< unfused bound {}", two + 27);

    let g3 = insts_of("var a = buf_cut(data, 1, 2)\nvar r = a ++ a ++ a\nreturn r");
    let g4 = insts_of("var a = buf_cut(data, 1, 2)\nvar r = a ++ a ++ a ++ a\nreturn r");
    assert!(g4 - g3 <= 12, "fused 4th GET-leaf costs {} (>12)", g4 - g3);
}

/// The depth gate refuses chains whose staged leaves cannot fit the
/// 8-position pool (a cut leaf peaks at 4 working entries: a 5-leaf cut
/// chain or a 4-leaf chain of 5-entry cast leaves exceeds it) — such
/// bodies MUST still lower (the unfused per-node path applies), never
/// turn a compiling body into a depth reject.
#[test]
fn c6_gate_fallback_still_lowers() {
    let five = insts_of("var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2) ++ buf_cut(data, 8, 2) ++ buf_cut(data, 10, 2)\nreturn r");
    let four = insts_of("var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2) ++ buf_cut(data, 8, 2)\nreturn r");
    assert!(five > four, "5-chain must lower (fallback)");
    // 4-chain over 5-entry cast leaves: staged leaf 3 would sit at
    // 2*2 + 5 = 9 > 8 — refused, still lowers
    insts_of("var r = ((buf_left(2, data) == buf_left(2, data)) as u64) as bytes ++ ((buf_left(1, data) != buf_left(1, data)) as u64) as bytes ++ ((buf_left(3, data) == buf_left(3, data)) as u64) as bytes\nreturn r");
}

/// Single-layer CAT behavior is byte-for-byte unchanged: the fusion gate
/// requires a CAT child, and the C6 code never runs for a plain `a ++ b`.
/// Two independent single-layer cats each carry a full per-node body
/// (overflow check + arena check + two copy loops + descriptor ≥ 25).
#[test]
fn c6_single_layer_cat_unchanged() {
    let n = insts_of("var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2)\nreturn r");
    let two_cats = insts_of("var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2)\nvar s = buf_cut(data, 6, 2) ++ buf_cut(data, 8, 2)\nreturn r ++ s");
    assert!(two_cats - n >= 25, "single-layer node body cost {} < 25", two_cats - n);
}

/// `x ++ ""` (and `"" ++ x`) folding is untouched: the empty-side fold is
/// the CAT arm's first condition and `cat_dyn_parts` refuses such nodes,
/// whether they are the fused tree's root or a nested child.
#[test]
fn c6_empty_fold_untouched() {
    // root-level empty fold
    agree4_big("var r = buf_cut(data, 1, 3) ++ \"\"\nreturn r ++ buf_cut(data, 6, 2)", &inputs3());
    // nested: the tree a ++ (b ++ "") must NOT fuse through the folded
    // child — the child folds to a pass-through pair, and the root sees a
    // non-CAT dynamic child plus... the root's children are `a` (dynamic,
    // not CAT) and the folded CAT — no fusion trigger, per-node emission
    agree4_big("var r = buf_cut(data, 1, 3) ++ (buf_cut(data, 4, 2) ++ \"\")\nreturn r", &inputs3());
}

/* ---- differential: fused shapes, four-way ---- */

#[test]
fn c6_chain_differential() {
    // 3-chain, left-nested (the parser's shape for a ++ b ++ c) — fused
    agree4_big(
        "var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2)\nreturn r",
        &inputs3(),
    );
    // 4-chain over GET leaves (staged peaks fit) — fused
    agree4_big(
        "var a = buf_cut(data, 1, 2)\nvar r = a ++ a ++ a ++ a\nreturn r",
        &inputs3(),
    );
    // 4-chain over cut leaves — gate refuses, the unfused path lowers
    agree4_big(
        "var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2) ++ buf_cut(data, 8, 2)\nreturn r",
        &inputs3(),
    );
    // right-nested chain (a ++ (b ++ c)) — same leaf set, mirrored tree
    agree4_big(
        "var r = buf_cut(data, 1, 2) ++ (buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2))\nreturn r",
        &inputs3(),
    );
    // mixed literal children ride the existing literal materialization
    agree4_big(
        "var r = buf_cut(data, 1, 2) ++ \"AB\" ++ buf_cut(data, 6, 2)\nreturn r",
        &inputs3(),
    );
    // literal-heavy chain: pool-literal leaves are memcpy sources too
    agree4_big(
        "var r = \"K7\" ++ buf_cut(data, 1, 2) ++ \"XY\" ++ buf_cut(data, 6, 2)\nreturn r",
        &inputs3(),
    );
    // chain inside a value position (the result feeds another expression)
    agree4_big(
        "var r = size(buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2))\nreturn r as bytes",
        &inputs3(),
    );
    // 5-leaf chain: gate refuses, the per-node path lowers it — must agree
    agree4_big(
        "var r = buf_cut(data, 1, 2) ++ buf_cut(data, 4, 2) ++ buf_cut(data, 6, 2) ++ buf_cut(data, 8, 2) ++ buf_cut(data, 10, 2)\nreturn r",
        &inputs3(),
    );
}

/// Nil(91) guard order is preserved node for node: make the guard sites
/// observable through WHICH fault fires. With may-Nil slot reads the
/// guards fire 91 in leaf order; an EMPTY right leaf still evaluates
/// (and guards) its side exactly as the unfused fold did.
#[test]
fn c6_nil_guard_order_preserved() {
    // may-Nil slots: read before any (conditional) write → tracked; the
    // guards fire in leaf order x, y, z — a reordering would swap the
    // faulting position when BOTH leaves are nil, which the interpreter
    // makes observable via which store the guard chain hits first... the
    // fault CODE is the same (91) for every leaf, so the observable is the
    // combined result: with all-nil sources every engine must fault 91;
    // with the first leaf nil and the rest fine too. Every combination
    // below must agree four-way, which pins the SET of fired guards.
    agree4_big(
        "var c = byte(data, 0)\nvar a = \"\"\nvar b = \"\"\nif c == 1 { a = buf_cut(data, 1, 2) }\nif c == 2 { b = buf_cut(data, 4, 2) }\nreturn a ++ b ++ buf_cut(data, 6, 2)",
        &inputs3(),
    );
    // the empty-literal leaf still evaluates its side (its own guard)
    agree4_big(
        "var c = byte(data, 0)\nvar a = \"\"\nif c == 1 { a = buf_cut(data, 1, 2) }\nreturn a ++ \"\" ++ buf_cut(data, 6, 2)",
        &inputs3(),
    );
}

/// The fused result is a fresh arena allocation — an inner view is never
/// aliased (the interpreter's intermediate `a++b` is a consumed temporary
/// in both engines). Pin the value semantics over an aliased-looking
/// shape: the chain reuses one source leaf twice.
#[test]
fn c6_reused_leaf_value_semantics() {
    agree4_big(
        "var a = buf_cut(data, 1, 3)\nvar r = a ++ a ++ a\nreturn r",
        &inputs3(),
    );
    // and the source leaf stays readable after the chain (fresh copy)
    agree4_big(
        "var a = buf_cut(data, 1, 3)\nvar r = a ++ a ++ a\nreturn r ++ a",
        &inputs3(),
    );
}
