//! T17 — general list values compile: nested elements, container values
//! flowing into every admitted consumer, TAKEFIRST/TAKELAST, MERGE, the
//! true shifting INSERT/REMOVE, KEYS/VALUES(list), list EQ/NEQ, the list
//! cast/ordered-compare/XOP fault faces and the CHOOSE descriptor select.
//!
//! Every case is a differential against the interpreter on all three rv64
//! modes (Im/Imc/ImcRemap): identical output bytes, or the identical
//! failure code (the iron rule: never another value, never another code —
//! billing is the only allowed difference).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};
use vm::calc::{CalcFnObj, CalcRunLimits};
use vm::ir::rv64::{lower_body_mode_arena, Rv64Mode, ARENA_SIZE_MAX};

fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

struct Rv64Ctr {
    engines: Vec<(Rv64Mode, CalcFnObj)>,
}

impl Rv64Ctr {
    fn try_build(body: &str) -> Result<Self, String> {
        let (ir, _) = c3lib::frontend(body)?;
        let cap = vm::calc::SpaceCap::new(0);
        let mut engines = Vec::new();
        for &mode in c3lib::MODES.iter() {
            let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX)
                .map_err(|e| format!("rv64(lower,{mode:?}): {e}"))?;
            let profile = match mode {
                Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
                _ => vm::calc::container::CALC_PROFILE_RV64IMC,
            };
            let container = lo
                .pack_segments_profile(&cap, 64, 4800, 16, profile)
                .map_err(|e| format!("rv64(pack,{mode:?}): {e}"))?;
            let bytes = container.encode().map_err(|e| format!("rv64(encode,{mode:?}): {e}"))?;
            let (rebuilt, _) = vm::calc::container::CalcContainer::decode(&bytes)
                .map_err(|e| format!("rv64(roundtrip,{mode:?}): {e}"))?;
            let fno = CalcFnObj::build(rebuilt, &cap, vm::calc::CalcHostConfig::new())
                .map_err(|e| format!("rv64(build,{mode:?}): {e}"))?;
            engines.push((mode, fno));
        }
        Ok(Self { engines })
    }

    fn run(&self, input: &[u8]) -> Vec<(Rv64Mode, Result<Vec<u8>, u64>)> {
        self.engines
            .iter()
            .map(|&(mode, ref fno)| {
                let ex = fno.run(
                    input,
                    CalcRunLimits {
                        cycles: c3lib::RV64_CYCLES,
                        resource: -1,
                        deadline: None,
                    },
                );
                (mode, ex.result.map_err(|_| ex.exit_code))
            })
            .collect()
    }
}

/// Four-way agreement on EVERY input: interpreter == all three rv64 modes
/// (Ok with identical bytes, or Err with the identical exit code).
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64Ctr::try_build(body)
        .unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    for input in inputs {
        let want = interp_obs(&it, input);
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got, want,
                "mode {mode:?} diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

fn one() -> Vec<Vec<u8>> {
    vec![vec![1, 2, 3, 4]]
}

fn bytes_all() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8, 7]).collect()
}

fn must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
}

/* ================================================================== */
/* TAKEFIRST / TAKELAST                                               */
/* ================================================================== */

#[test]
fn t17_take_first_last_values_and_alias_visibility() {
    agree4(
        "var l = [10u64, 20u64, 30u64]\nvar a = (take_first(l) as u64)\nvar b = (take_last(l) as u64)\nreturn ((a * 100u64 + b) * 100u64 + length(l)) as bytes",
        &one(),
    );
    // the mutation is visible through every alias (shared object)
    agree4(
        "var l = [1u64, 2u64, 3u64]\nvar m = l\nvar a = (take_first(m) as u64)\nreturn ((length(l) as u64) * 10u64 + l[0]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64, 3u64]\nvar m = l\nvar a = (take_last(l) as u64)\nreturn ((length(m) as u64) * 10u64 + m[1]) as bytes",
        &one(),
    );
    // empty list → CompoOpOverflow(81)
    agree4(
        "var l = []\nvar a = (take_first(l) as u64)\nreturn a as bytes",
        &one(),
    );
    agree4(
        "var l = []\nvar a = (take_last(l) as u64)\nreturn a as bytes",
        &one(),
    );
    // narrow element width flows through the take
    agree4(
        "var l = [7u8, 200u8]\nvar a = (take_first(l) as u64)\nreturn (a as bytes)",
        &one(),
    );
    // u128 element pair
    agree4(
        "var l = [340282366920938463463374607431768211455u128, 1u128]\nvar a = take_first(l)\nreturn (a as bytes)",
        &one(),
    );
    // the popped element keeps ITS width through `as bytes`
    agree4(
        "var l = [200u8, 1u8]\nvar a = take_first(l)\nreturn (a as bytes)",
        &one(),
    );
    agree4(
        "var l = [4660u16, 1u16]\nvar a = take_last(l)\nreturn (a as bytes)",
        &one(),
    );
}

/* ================================================================== */
/* INSERT / REMOVE — the true shifts                                  */
/* ================================================================== */

#[test]
fn t17_insert_remove_shift_and_faces() {
    // insert in the middle / at the end, then read back the shifted list
    agree4(
        "var l = [1u64, 2u64, 3u64]\ninsert(l, 1u64, 99u64)\nreturn ((length(l) as u64) * 1000u64 + l[1]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\ninsert(l, 2u64, 9u64)\nreturn ((length(l) as u64) * 100u64 + l[2]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64, 3u64]\nremove(l, 1u64)\nreturn ((length(l) as u64) * 100u64 + l[1]) as bytes",
        &one(),
    );
    // alias visibility of both shifts
    agree4(
        "var l = [1u64, 2u64, 3u64]\nvar m = l\ninsert(m, 0u64, 5u64)\nreturn ((length(l) as u64) * 100u64 + l[0]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64, 3u64]\nvar m = l\nremove(m, 0u64)\nreturn ((length(l) as u64) * 100u64 + l[0]) as bytes",
        &one(),
    );
    // insert idx > len → 81
    agree4(
        "var l = [1u64, 2u64]\ninsert(l, 3u64, 9u64)\nreturn 0u64 as bytes",
        &one(),
    );
    // remove idx >= len → 85
    agree4(
        "var l = [1u64, 2u64]\nremove(l, 2u64)\nreturn 0u64 as bytes",
        &one(),
    );
    // insert at the 128-element cap → 40; remove at 128 ok
    agree4(
        "var l = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(l, i)\ni += 1u64\n}\ninsert(l, 0u64, 7u64)\nreturn length(l) as bytes",
        &one(),
    );
    // variable index
    agree4(
        "var l = [1u64, 2u64, 3u64, 4u64]\nvar i = (byte(data, 0) as u64) % 4u64\nremove(l, i)\nreturn ((length(l) as u64) * 100u64 + l[0]) as bytes",
        &bytes_all(),
    );
    agree4(
        "var l = [1u64, 2u64, 3u64, 4u64]\nvar i = (byte(data, 0) as u64) % 5u64\ninsert(l, i, 9u64)\nreturn ((length(l) as u64) * 100u64 + l[0]) as bytes",
        &bytes_all(),
    );
    // u128 element insert/remove
    agree4(
        "var l = [1u128, 2u128]\ninsert(l, 1u64, 340282366920938463463374607431768211455u128)\nreturn (l[1] as bytes)",
        &one(),
    );
    agree4(
        "var l = [1u128, 2u128, 3u128]\nremove(l, 1u64)\nreturn (l[1] as bytes)",
        &one(),
    );
}

/* ================================================================== */
/* MERGE                                                              */
/* ================================================================== */

#[test]
fn t17_merge_list_semantics_and_faces() {
    agree4(
        "var a = [1u64, 2u64]\nvar b = [3u64, 4u64]\nmerge(a, b)\nreturn ((length(a) as u64) * 10000u64 + a[2] * 100u64 + a[3]) as bytes",
        &one(),
    );
    // alias of the source observes nothing; the receiver is the target
    agree4(
        "var a = [1u64]\nvar b = [2u64]\nvar c = b\nmerge(a, b)\nreturn ((length(a) as u64) * 100u64 + length(c) as u64) as bytes",
        &one(),
    );
    // same Rc → CompoOpInvalid(80)
    agree4(
        "var a = [1u64, 2u64]\nmerge(a, a)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var a = [1u64]\nvar b = a\nmerge(a, b)\nreturn 0u64 as bytes",
        &one(),
    );
    // new length over the 128 cap → 40
    agree4(
        "var a = []\nvar i = 0u64\nwhile i < 100u64 {\nappend(a, i)\ni += 1u64\n}\nvar b = []\nvar j = 0u64\nwhile j < 29u64 {\nappend(b, j)\nj += 1u64\n}\nmerge(a, b)\nreturn length(a) as bytes",
        &one(),
    );
    // exactly 128 is admitted
    agree4(
        "var a = []\nvar i = 0u64\nwhile i < 100u64 {\nappend(a, i)\ni += 1u64\n}\nvar b = []\nvar j = 0u64\nwhile j < 28u64 {\nappend(b, j)\nj += 1u64\n}\nmerge(a, b)\nreturn length(a) as bytes",
        &one(),
    );
    // the merge result bound to a fresh slot is the receiver alias
    agree4(
        "var a = [1u64]\nvar b = [2u64]\nvar m = merge(a, b)\nreturn ((length(m) as u64) * 100u64 + length(a) as u64) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* KEYS / VALUES(list) — the 83 face                                  */
/* ================================================================== */

#[test]
fn t17_keys_values_on_a_list_fault_83() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = keys(l)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = values(l)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var l = []\nkeys(l)\nreturn 0u64 as bytes",
        &one(),
    );
}

/* ================================================================== */
/* container values flowing into consumers (the runtime 76 face)       */
/* ================================================================== */

#[test]
fn t17_nested_container_elements_fault_76() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = [l]\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = [7u64, l, 9u64]\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var a = [1u64]\nvar b = [2u64]\nappend(a, b)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var a = [1u64]\nvar b = [2u64]\ninsert(a, 0u64, b)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var a = [1u64]\nvar b = [2u64]\na[0] = b\nreturn 0u64 as bytes",
        &one(),
    );
    // a scalar element before the nested one still evaluates/faults first
    agree4(
        "var a = [1u64]\nvar b = [2u64]\nvar i = (byte(data, 0) as u64) + 7u64\nvar m = [a[i], b]\nreturn 0u64 as bytes",
        &one(),
    );
}

/* ================================================================== */
/* list EQ / NEQ — value_content_eq                                    */
/* ================================================================== */

#[test]
fn t17_list_eq_neq_content() {
    agree4(
        "var l = [1u64, 2u64]\nvar r = ((l == l) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = [1u64, 2u64]\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = [1u64, 3u64]\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64]\nvar m = [1u64, 2u64]\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar r = ((l != l) as u64)\nreturn r as bytes",
        &one(),
    );
    // numeric equality across element widths
    agree4(
        "var l = [1u64, 2u64]\nvar m = [1u8, 2u8]\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u8, 2u8]\nvar m = [1u16, 2u16]\nvar r = ((l == m) as u64)\nreturn r as bytes",
        &one(),
    );
}

/* ================================================================== */
/* fault-only faces: ordered compare / casts / XOP                     */
/* ================================================================== */

#[test]
fn t17_list_fault_faces_match() {
    agree4(
        "var l = [1u64, 2u64]\nvar r = ((l < l) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar r = ((l >= l) as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar r = (l as bytes)\nreturn r",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar r = (l as u64)\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nl += 1u64\nreturn 0u64 as bytes",
        &one(),
    );
}

/* ================================================================== */
/* CHOOSE / binding alias forms                                        */
/* ================================================================== */

#[test]
fn t17_choose_arm_selects_a_descriptor() {
    agree4(
        "var c = (byte(data, 0) as u64)\nvar l = [1u64, 2u64]\nvar r = choose(c == 0u64, l, l)\nreturn length(r) as bytes",
        &bytes_all(),
    );
    agree4(
        "var c = (byte(data, 0) as u64)\nvar a = [1u64]\nvar b = [10u64, 20u64]\nvar r = choose(c == 0u64, a, b)\nreturn ((length(r) as u64) * 100u64 + r[0]) as bytes",
        &bytes_all(),
    );
    // the selected descriptor is an alias: mutation is visible both ways
    agree4(
        "var c = (byte(data, 0) as u64)\nvar a = [1u64]\nvar b = [10u64, 20u64]\nvar r = choose(c == 0u64, a, b)\nappend(r, 5u64)\nreturn ((length(a) as u64) * 100u64 + length(b) as u64) as bytes",
        &bytes_all(),
    );
    // a dropped choose only evaluates the condition
    agree4(
        "var c = (byte(data, 0) as u64)\nvar a = [1u64]\nvar b = [10u64, 20u64]\nchoose(c == 0u64, a, b)\nreturn length(a) as bytes",
        &bytes_all(),
    );
}

#[test]
fn t17_append_result_binds_an_alias() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = append(l, 3u64)\nreturn ((length(l) as u64) * 100u64 + length(m) as u64) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = append(l, 3u64)\nappend(m, 4u64)\nreturn length(l) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clear(l)\nreturn ((length(l) as u64) * 100u64 + length(m) as u64) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* reject faces that stay (B-family / ABI)                             */
/* ================================================================== */

#[test]
fn t17_rejects_that_stay() {
    // returning a list is an ABI limitation (machine always pushes Bytes)
    must_reject("var l = [1u64, 2u64]\nreturn l", "RET");
    // a mixed list/int compare is not lowered (the interpreter faults 90)
    must_reject(
        "var l = [1u64, 2u64]\nvar r = ((l == 1u64) as u64)\nreturn r as bytes",
        "EQ",
    );
}

/* ================================================================== */
/* Extra edge coverage                                                */
/* ================================================================== */

#[test]
fn t17_take_through_clone_does_not_touch_the_original() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nvar a = (take_first(m) as u64)\nreturn ((length(l) as u64) * 100u64 + (length(m) as u64)) as bytes",
        &one(),
    );
}

#[test]
fn t17_insert_remove_result_binds() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = insert(l, 1u64, 9u64)\nreturn ((length(m) as u64) * 100u64 + m[1]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = remove(l, 0u64)\nreturn ((length(m) as u64) * 100u64 + m[0]) as bytes",
        &one(),
    );
    // the bound alias sees a later mutation of the original
    agree4(
        "var l = [1u64]\nvar m = insert(l, 1u64, 5u64)\nappend(l, 7u64)\nreturn ((length(m) as u64) * 100u64 + m[1]) as bytes",
        &one(),
    );
}

#[test]
fn t17_empty_and_single_element_shifts() {
    // insert into an empty list (idx == len == 0) grows it
    agree4(
        "var l = []\ninsert(l, 0u64, 5u64)\nreturn ((length(l) as u64) * 100u64 + l[0]) as bytes",
        &one(),
    );
    // remove the only element, then take → 81
    agree4(
        "var l = [4u64]\nremove(l, 0u64)\nvar a = (take_first(l) as u64)\nreturn a as bytes",
        &one(),
    );
    // take_last down to empty
    agree4(
        "var l = [4u64]\nvar a = (take_last(l) as u64)\nvar b = (take_last(l) as u64)\nreturn a as bytes",
        &one(),
    );
    // merge into an empty list
    agree4(
        "var a = []\nvar b = [1u64, 2u64]\nmerge(a, b)\nreturn ((length(a) as u64) * 100u64 + a[1]) as bytes",
        &one(),
    );
    // merge two empty lists
    agree4(
        "var a = []\nvar b = []\nmerge(a, b)\nreturn length(a) as bytes",
        &one(),
    );
    // an empty source imposes no element width on a typed receiver
    agree4(
        "var a = [1u8, 2u8]\nvar b = []\nmerge(a, b)\nreturn ((length(a) as u64) * 100u64 + (a[1] as u64)) as bytes",
        &one(),
    );
}

#[test]
fn t17_merge_u128_elements() {
    agree4(
        "var a = [1u128]\nvar b = [2u128]\nmerge(a, b)\nreturn (a[1] as bytes)",
        &one(),
    );
}

#[test]
fn t17_keys_values_on_aliased_list_fault_83() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nkeys(m)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nvalues(m)\nreturn 0u64 as bytes",
        &one(),
    );
}

#[test]
fn t17_choose_mixed_width_arms_reject() {
    // the descriptor select stores into one slot: the two arm lists must
    // share one element width (the interpreter would accept the mix —
    // B-family compile reject, never a wrong value)
    must_reject(
        "var c = (byte(data, 0) as u64)\nvar a = [1u64]\nvar b = [1u8]\nvar r = choose(c == 0u64, a, b)\nreturn length(r) as bytes",
        "mix element widths",
    );
}

#[test]
fn t17_rebound_nested_pack_faults_76() {
    agree4(
        "var l = [1u64, 2u64]\nl = [l]\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64]\nvar m = [1u64, 2u64]\nl = [3u64, m]\nreturn 0u64 as bytes",
        &one(),
    );
}

/* ================================================================== */
/* The production fitshc entry wired the same model                    */
/* ================================================================== */

#[test]
fn t17_production_entry_accepts_general_list_values() {
    // kernel::build lowers through lower_body_mode_arena_heap — T17 made
    // that entry the container model, so a contract body using the new
    // list faces compiles end to end (the OP_CONSTRUCT boundary no longer
    // rewrites these to a reject).
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        var l = [1u64, 2u64, 3u64]\n        var m = l\n        var a = take_first(m)\n        insert(l, 0u64, a)\n        remove(l, 2u64)\n        var b = [7u64]\n        merge(l, b)\n        return length(l) as bytes\n    }\n}\n";
    vm::fitshc::compile_with_rv64im(src, &[])
        .unwrap_or_else(|e| panic!("production entry must compile the body: {e}"));
}

#[test]
fn t17_shifts_in_loops_with_aliases() {
    agree4(
        "var l = []\nvar i = 0u64\nwhile i < 5u64 {\nappend(l, i)\ni += 1u64\n}\nvar m = l\nvar j = 0u64\nwhile j < 3u64 {\nremove(m, 0u64)\nj += 1u64\n}\nreturn ((length(l) as u64) * 100u64 + l[0]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nvar k = 0u64\nwhile k < 3u64 {\ninsert(m, 0u64, k * 10u64)\nk += 1u64\n}\nreturn ((length(l) as u64) * 1000u64 + l[0] * 100u64 + l[3]) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64, 3u64]\nvar m = l\nvar s = 0u64\nvar i = 0u64\nwhile i < 2u64 {\ns += (take_first(m) as u64)\ni += 1u64\n}\nreturn (s * 100u64 + length(l)) as bytes",
        &one(),
    );
    agree4(
        "var a = [1u64]\nvar b = [2u64]\nvar i = 0u64\nwhile i < 3u64 {\nmerge(a, b)\ni += 1u64\n}\nreturn ((length(a) as u64) * 100u64 + length(b) as u64) as bytes",
        &one(),
    );
}

#[test]
fn t17_clone_rebind_replaces_only_the_target() {
    agree4(
        "var l = [1u64, 2u64]\nl = clone(l)\nappend(l, 3u64)\nreturn length(l) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nl = clone(l)\nappend(l, 3u64)\nreturn ((length(l) as u64) * 100u64 + length(m) as u64) as bytes",
        &one(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nl = m\nappend(m, 7u64)\nreturn ((length(l) as u64) * 100u64 + l[2]) as bytes",
        &one(),
    );
}

#[test]
fn t17_merge_into_untyped_receiver_adopts_the_source_width() {
    // the receiver never stored an element before the merge, so the
    // source's element width must become the receiver's (the interpreter
    // keeps the u8 elements' own width through every later read)
    agree4(
        "var a = []\nvar b = [1u8, 2u8]\nmerge(a, b)\nreturn (a[0] as bytes)",
        &one(),
    );
    agree4(
        "var a = []\nvar b = [1u16, 2u16]\nmerge(a, b)\nreturn (a[1] as bytes)",
        &one(),
    );
    agree4(
        "var a = []\nvar b = [1u128, 2u128]\nmerge(a, b)\nreturn (a[1] as bytes)",
        &one(),
    );
}
