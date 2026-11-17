//! Regression pins for the three-way (interpreter / rv64 / reference)
//! alignment campaign bugs. Every assertion is written to the INTERPRETER
//! (HVM) semantics — never the pre-fix rv64 behavior.
//!
//! Each test names its root cause and the fixing commit site:
//! * pow_exp_over_u32_max_traps_90        → emit/eval.rs POW loop path
//! * (bug 2) address stain slot           → (file noted at the test)
//! * (bug 3) runtime x % 2^k mask         → (file noted at the test)

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

/// Four-way agreement: interpreter value/fault == all three rv64 modes.
/// (Trap codes compared as u64; interpreter fault code as u64.)
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("{body}: rv64 build: {e}"));
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

fn no_input() -> Vec<Vec<u8>> {
    vec![vec![]]
}

/* ==================================================================== */
/* BUG (3way #1): POW exponent > u32::MAX must trap Arithmetic(90).     */
/*                                                                      */
/* Interpreter ground truth (instruction.rs pow_checked): the u64/u128  */
/* arms convert the exponent through `u32::try_from` FIRST, and the     */
/* conversion failure faults Arithmetic 90 — BEFORE any multiplication, */
/* so even base 0/1 (whose mathematical power never overflows) traps.   */
/* The rv64 loop path squared/multiplied the raw u64 exponent with no   */
/* exponent range check: base 0/1 with exp = 2^32 ran the multiply loop */
/* 2^32 times and returned a clean value (worse: a 4-billion-iteration  */
/* hang under a cycle budget) instead of trapping 90. The u128 path     */
/* (emit_u128_pow) already had the pre-check; the 64-bit loop path in   */
/* emit/eval.rs now emits the same `exp >= 2^32 → fault_arith` guard.   */
/* ==================================================================== */

#[test]
fn pow_exp_over_u32_max_traps_90() {
    // NOTE: bare `1u64 ** 4294967296u64` hits a frontend literal-parse
    // quirk ("integer literal expression exceeds u128"), so every shape
    // goes through `var` declarations (the g1 repro form).
    let mut agree = |base: &str, exp: &str| {
        let body = format!(
            "var a = {base}\nvar b = {exp}u64\nreturn ((a ** b) as bytes)"
        );
        agree4(&body, &[vec![]]);
    };
    // the g1 three-way minimal repro: exp = 2^32. Interp: Trap(90) for
    // EVERY base (0, 1, 2) — the exp→u32 conversion faults before the pow
    // itself runs.
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = 4294967296u64\nreturn ((a ** b) as bytes)",
        &[vec![0], vec![1], vec![2], vec![255]],
    );
    agree("1u64", "4294967296");
    agree("0u64", "4294967296");
    agree("2u64", "4294967296");

    // base 1 with a huge exponent is the discriminating shape (no mul
    // overflow to hide behind): interp 90, pre-fix rv64 looped to a clean
    // stop / budget exit.
    agree("1u64", "4294967297");
    agree("0u64", "4294967297");

    // just below the boundary: exp == u32::MAX still fits u32 — base 2
    // then overflows u64 → 90 on both engines (mulhu face == checked_pow).
    agree("2u64", "4294967295");
    // exp == 2^32+1, base 2: interp faults on the conversion (90) before
    // any overflow consideration.
    agree("2u64", "4294967297");

    // sanity: normal exponents still agree (unrolled + loop paths).
    agree("3u64", "5");
    agree("2u64", "64");
}

/* ==================================================================== */
/* BUG (3way #2): an Address-stained slot must answer the Address face. */
/*                                                                      */
/* Interpreter ground truth (g6_types_bug_type_id_address_slot_stain):  */
/* a local holds the runtime Value, so after `var ad = (data as         */
/* address)` the re-read `ad` IS an Address value: `(ad is address)` →  */
/* 1, `(ad is bytes)` → 0, `type_id(ad)` → 9. The slot model mapped the */
/* PUT of an Address value to SlotTy::Bytes, so the type tests on the   */
/* re-read answered the Bytes face (0/1/8) — compiled-but-divergent.    */
/* Fix: a dedicated SlotTy::Address stain (identical runtime cell),     */
/* carried through PUT/PUTX unification, GET/GETX res-ty, the frame     */
/* cell size and the type-test lowering. A slot mixing Address and      */
/* Bytes sources is now a compile reject (the interpreter's stain is    */
/* last-write-wins and no static face can answer it).                   */
/* ==================================================================== */

/// a version-1 address: version byte 1 + 20 content bytes
fn addr_v1() -> Vec<u8> {
    let mut v = vec![1u8];
    v.extend((0u8..20).collect::<Vec<u8>>());
    v
}

#[test]
fn address_slot_keeps_stain() {
    let input = addr_v1();

    // type_id on the slot re-read: interp 9 (Address), pre-fix rv64 said 8
    agree4(
        "var ad = (data as address)\nreturn ((type_id(ad)) as u64) as bytes",
        &[input.clone()],
    );
    // `is address` on the slot re-read: interp 1, pre-fix rv64 said 0
    agree4(
        "var ad = (data as address)\nvar r = ((ad is address) as u64)\nreturn (r) as bytes",
        &[input.clone()],
    );
    // `is bytes`: interp 0, pre-fix rv64 said 1
    agree4(
        "var ad = (data as address)\nvar r = ((ad is bytes) as u64)\nreturn (r) as bytes",
        &[input.clone()],
    );
    // unaffected faces pinned by the g6 report: is nil / is u128 / as u128
    agree4(
        "var ad = (data as address)\nvar e = ((ad is nil) as u64)\nvar u = ((ad is u128) as u64)\nreturn ((e + u * 2u64)) as bytes",
        &[input.clone()],
    );

    // stain survives a slot-to-slot copy and a second Address write
    agree4(
        "var ad = (data as address)\nvar ad2 = ad\nad = (data as address)\nreturn ((type_id(ad2) + type_id(ad) * 16u64) as u64) as bytes",
        &[input.clone()],
    );

    // a plain bytes slot keeps the bytes face (1/8)
    agree4(
        "var s = buf_left(1, data)\nvar e = ((s is bytes) as u64)\nvar i = ((type_id(s)) as u64)\nreturn ((e + i * 2u64)) as bytes",
        &[input.clone()],
    );

    // inline (no-slot) forms: this tree admits `type_id(address)` (the
    // sid table maps Ty::Address → 9 and the type-test operand lowers as
    // the bytes descriptor) — both engines must answer the Address face
    agree4(
        "return ((type_id(data as address)) as u64) as bytes",
        &[input.clone()],
    );
    agree4(
        "var r = (((data as address) is address) as u64)\nreturn (r) as bytes",
        &[input.clone()],
    );
    agree4(
        "var r = (((data as address) is bytes) as u64)\nreturn (r) as bytes",
        &[input.clone()],
    );
}

#[test]
fn address_bytes_slot_mix_is_compile_reject() {
    // the interpreter's stain is last-write-wins (runtime-flow): after
    // `ad = bytes`, `type_id(ad)` answers 8 again. No static slot face can
    // reproduce that — the kernel must refuse to compile the mix (allowed:
    // fail-fast, never compiled-but-divergent).
    let body = "var ad = (data as address)\nad = buf_left(1, data)\nreturn ((type_id(ad)) as u64) as bytes";
    let rv = Rv64::try_build(body);
    assert!(rv.is_err(), "address/bytes slot mix must be a compile reject");
    let it = Interp::try_build(body);
    assert!(it.is_ok(), "the interpreter accepts the mix: {:?}", rv.err());
}

/* ==================================================================== */
/* BUG (3way #3): runtime `x % 2^k` must MASK, not pass through.        */
/*                                                                      */
/* Interpreter ground truth (g10_fullnode_mod_pow2_bug_candidate):      */
/* 10000000000u64 % 4294967296u64 == 1410065408 (unsigned rem < 2^32).  */
/* The MOD-by-constant pow2 fast path emitted `andi d, d, (k-1) as      */
/* i32` — for k = 2^32 the mask 0xFFFFFFFF truncated to -1, which       */
/* sign-extends to the identity: the ORIGINAL value came back. Every    */
/* k with k-1 above the 12-bit signed immediate window was affected     */
/* (k in (2048, 2^31] even tripped the assembler's imm-range assert at  */
/* compile time). The DIVEXACT pow2 arm had the same mask shape (x =    */
/* 2^32 `/= 2^32` would fault 90 instead of dividing) — both now load   */
/* the mask into a pool temp when it does not fit i12. The general      */
/* remu arm and the u128 pair path were already correct.                */
/* ==================================================================== */

#[test]
fn mod_pow2_runtime_masks() {
    // the g10 repro: m % 2^32 with m >= 2^32 (a runtime value, not folded)
    let mut agree = |m: &str, k: &str| {
        let body =
            format!("var m = {m}u64\nvar n = (m % {k}u64)\nreturn (n) as bytes");
        agree4(&body, &[vec![]]);
    };
    agree("10000000000", "4294967296"); // → 1410065408
    agree("18446744073709551615", "4294967296"); // 2^64-1 % 2^32 = 2^32-1
    agree("12345678901234567890", "4294967296");
    agree("4294967296", "4294967296"); // exactly 2^32 → 0

    // sweep the immediate-window boundary: k-1 <= 2047 rode andi already
    // (k = 2048); k in (2048, 2^31] tripped the compile assert; k >= 2^32
    // silently returned the original value
    for k in [2048u64, 4096, 65536, 16777216, 2147483648, 4294967296, 1099511627776] {
        agree("18446744073709551615", &k.to_string()); // → k-1
        agree("999999999", &k.to_string());
    }

    // mod by a non-pow2 constant and by a runtime value stay on remu
    agree("10000000000", "4294967295");
    agree("18446744073709551615", "1000000007");

    // the DIVEXACT pow2 arm shares the mask shape: x /= 2^j must divide
    // when exact and fault 90 otherwise — including masks above i12
    let mut agree_div = |x: &str, k: &str, expect_ok: bool| {
        let body =
            format!("var x = {x}u64\nvar n = (x / {k}u64)\nreturn (n) as bytes");
        let it = Interp::try_build(&body).unwrap();
        let rv = Rv64::try_build(&body).unwrap();
        let want = it.run(&[]);
        for (mode, got) in rv.run(&[]) {
            match (&want, &got) {
                (Ok(w), Ok(g)) => assert_eq!(w, g, "{body} [{mode:?}]"),
                (Err(wf), Err(code)) => {
                    assert_eq!(wf.code as u64, *code, "{body} [{mode:?}]");
                    assert!(!expect_ok, "{body} unexpectedly trapped: {code}");
                }
                _ => panic!("{body} [{mode:?}]: interp={want:?} rv64={got:?}"),
            }
        }
    };
    agree_div("8589934592", "4294967296", true); // 2^33 / 2^32 = 2
    agree_div("4294967295", "4294967296", false); // not exact → 90
    agree_div("4096", "4096", true); // boundary: mask 4095 rides a temp
    agree_div("4095", "4096", false);
}

/* ==================================================================== */
/* BUG (3way #4): the `as bytes` view of a slot assigned PLAIN values   */
/* of mixed static widths.                                              */
/*                                                                      */
/* Root cause. The interpreter stores each assignment's VALUE with its  */
/* own width tag (`execute.rs` PUT → `locals.save(idx, v)` keeps the    */
/* popped value untouched) and `cast_bytes` emits the CURRENT tag's     */
/* width BE (`value/cast.rs` → scalar_bytes). A slot assigned `1001`    */
/* (a bare literal → CU16) and later `0` (→ CU8) therefore views as     */
/* ONE byte — and the SAME `as bytes` site emits different lengths for  */
/* different inputs when the assignments sit in branches (the g17       */
/* tier2 HACD no-suffix errno face: `oe = [0]` on success inputs vs     */
/* `oe = [3, 233]` on failure inputs). The kernel slot cell has ONE     */
/* static width — the join (max) over every write site                  */
/* (`widths.rs join_slot_widths`) — so its `as bytes` view was fixed    */
/* at the join width: `errno as bytes` compiled to `[0, 0]` where the   */
/* interpreter produces `[0]`. "Compiles but answers differently".      */
/*                                                                      */
/* Fix (no reject: hacd200's `if total < 1 { total = 1 }` +             */
/* `total as bytes` shape must keep compiling — the u128/bool mixings   */
/* keep their own established rejects). `join_slot_widths` now also     */
/* flags the face: an `as bytes` VIEW of a slot whose write sites       */
/* carry mixed real widths (plain PUT/PUTX store their rhs's own        */
/* width; XOP/max-min stores grow by the rhs). A flagged slot gets a    */
/* 1-byte runtime width shadow in the frame: every assignment stores    */
/* its own width, compound writes grow it, and the slot's `as bytes`    */
/* view (`emit_cbytes_dyn`) emits exactly the shadow's byte count —     */
/* the interpreter's cast_bytes face, replayed. Unflagged bodies stay   */
/* byte identical.                                                      */
/* ==================================================================== */

#[test]
fn narrow_literal_put_bytes_width_tiers() {
    // no-suffix literals infer their minimal width (g10); a plain PUT
    // stores it and the `as bytes` view must follow — per width tier,
    // including the exactly-at-boundary values
    for n in [
        0u64, 1, 200, 255, // u8 tier + boundary
        256, 257, 1001, 65535, // u16 tier + boundaries
        65536, 65537, 4294967295, // u32 tier + boundaries
        4294967296, // u64 tier boundary
    ] {
        agree4(&format!("var errno = {n}\nreturn (errno as bytes)"), &[vec![]]);
    }
    // the g10 narrow fold face at the exactly-full boundary
    agree4(
        "var w = byte(data, 0) * 256 + byte(data, 1)\nreturn (w as bytes)",
        &[vec![0xFF, 0xFF], vec![0xFF, 0x00], vec![0x12, 0x34]],
    );
}

#[test]
fn narrow_put_view_follows_last_assignment() {
    // THE bug face: `errno` narrows 1001 (u16) → 0 (u8); the interpreter
    // views [0]; the pre-fix kernel kept the u16 join and emitted [0, 0]
    agree4(
        "var errno = 1001\nerrno = 0\nreturn (errno as bytes)",
        &[vec![]],
    );
    // widening re-assignment
    agree4(
        "var errno = 0\nerrno = 1001\nreturn (errno as bytes)",
        &[vec![]],
    );
    // narrow → widen → narrow → widen, all plain PUTs
    agree4(
        "var errno = 1001\nerrno = 0\nerrno = 5\nerrno = 70000\nreturn (errno as bytes)",
        &[vec![]],
    );
    // the HACD oe++ident wire shape (concat after the view)
    agree4(
        "var errno = 1001\nerrno = 0\nvar oe = errno as bytes\nreturn oe++\"ab\"",
        &[vec![]],
    );
}

#[test]
fn narrow_put_view_is_path_dependent() {
    // the assignments sit in branches: the view width depends on the
    // input — the exact g17 tier2 face shape (errno decl 0, branch PUTs
    // of 100x). input [7] takes the branch (u16 view), [8] does not
    // (u8 view), and both must match the interpreter byte for byte.
    agree4(
        "var errno = 0\nif byte(data, 0) == 7 { errno = 1001 }\nreturn (errno as bytes)",
        // (no empty input: byte(data, 0) out of bounds is the pre-pinned
        // 42-vs-71 fault-code face, not this bug)
        &[vec![7], vec![8], vec![9]],
    );
    // else-arms assign different widths per path
    agree4(
        "var errno = 0\nif byte(data, 0) == 7 { errno = 1001 } else { errno = 3 }\nreturn (errno as bytes)",
        &[vec![7], vec![8]],
    );
    // nested if (the HACD body's shape: errno PUTs several levels deep)
    agree4(
        "var errno = 0\nif byte(data, 0) == 7 { if byte(data, 1) == 9 { errno = 1005 } else { errno = 1001 } }\nreturn (errno as bytes)",
        &[vec![7, 9], vec![7, 8], vec![8, 9]],
    );
    // conditional PUT inside a loop (backedge must not stale the shadow)
    agree4(
        "var e = 0\nvar k = 0\nwhile k < 3 { k += 1\nif byte(data, 0) == 7 { e = 1001 } }\nreturn (e as bytes)",
        &[vec![7], vec![8]],
    );
}

#[test]
fn narrow_put_compound_growth_matches_xop_join() {
    // `+=` keeps the slot/rhs COMMON width (g9): the interpreter's XOP
    // result is max(current, rhs) — the shadow grows exactly so
    // a u8 counter still faults at 256 (pinned in rv64_width_semantics);
    // here: growth above the narrow start
    agree4(
        "var e = 0\ne += 1000\nreturn (e as bytes)",
        &[vec![]],
    );
    // conditional growth: taken → u16 view, untaken → stays u8
    agree4(
        "var e = 0\nif byte(data, 0) == 7 { e += 1000 }\nreturn (e as bytes)",
        &[vec![7], vec![8]],
    );
    // a narrow `+=` on a narrowed slot: 1001 → 0 (u8) then += 5 stays u8
    agree4(
        "var e = 1001\ne = 0\ne += 5\nreturn (e as bytes)",
        &[vec![]],
    );
    // max/min fused update (the hacd200 `if 50 > nu { nu = 50 }` shape)
    // on a viewed mixed-width slot
    agree4(
        "var e = 1001\ne = 0\nif 50 > e { e = 50 }\nreturn (e as bytes)",
        &[vec![]],
    );
    // constant-select fusion (match_sel_arm runs, default PUT right before
    // the run, every arm ≤ 255) on a viewed mixed-width slot: the fused
    // table store must narrow the shadow to the u8 face
    agree4(
        "var s = 1001\nvar i = 0\nwhile i < 5 { i += 1\ns = 9\nif i == 1 { s = 1 }\nif i == 2 { s = 2 }\nif i == 3 { s = 3 }\nif i == 4 { s = 4 } }\nreturn (s as bytes)",
        &[vec![]],
    );
    // an over-255 arm disqualifies the whole run — the arms fall back to
    // plain PUTs, each replaying its own width into the shadow
    agree4(
        "var s = 0\nvar i = 0\nwhile i < 5 { i += 1\ns = 9\nif i == 1 { s = 1 }\nif i == 2 { s = 2 }\nif i == 3 { s = 3 }\nif i == 4 { s = 1001 } }\nreturn (s as bytes)",
        &[vec![]],
    );
}

#[test]
fn narrow_put_view_stays_exact_after_arith_reads() {
    // arithmetic READS stay at the static join width (the value plane is
    // unchanged by the fix); the view follows the shadow — both faces in
    // one body
    agree4(
        "var e = 1001\ne = 0\nvar d = e * 256 + byte(data, 0)\nif d == 1799 { e = 65535 }\nreturn (e as bytes)",
        &[vec![7], vec![8]],
    );
    // u32-tier slot narrowed then VIEWED (the value plane of the GET that
    // feeds `as bytes` is the shadow's, not the join's)
    agree4(
        "var e = 70000\ne = 257\nreturn (e as bytes)",
        &[vec![]],
    );
    // NOTE the boundary: an ARITHMETIC RESULT's own width (`(e * 2) as
    // bytes`) still follows the static join on the kernel (the interp's
    // dynamic-plane result width is a separate, still-open face of the
    // same root cause — the shadow replays assignment widths, not
    // expression planes; unobserved in the corpus, unpinned here).
}
