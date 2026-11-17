//! P2 (§3 shared prework) — the arena container descriptor value model's
//! differential tests.
//!
//! The model (vm/src/ir/rv64.rs, "P2 arena container object model"):
//! a container-valued list slot whose value ESCAPES (alias `var m = l`,
//! re-bind `l = <list>`, clone-bind `var m = clone(l)`) takes the
//! `ListKind::Arena` storage class — its 8-byte frame cell holds the
//! descriptor (pointer) of an RW-arena object `{len, cap, tagged cells}`;
//! alias PUT copies the descriptor (one shared object, the interpreter's
//! `Rc<RefCell<Compo>>`), CLONE is the only deep copy, and in-place
//! mutation (append / clear / item_set) lands on the shared header or
//! cells so every alias observes it.
//!
//! T17 wired the model into every lowering entry: the default channel IS
//! the container model (`lower_body_mode_arena_containers` remains an
//! alias for the harnesses that name it). The last section pins that
//! parity — the historical escape shapes build on the default entry too.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::Interp;
use vm::calc::{CalcFnObj, CalcRunLimits};
use vm::ir::rv64::{lower_body_mode_arena_containers, Rv64Mode, ARENA_SIZE_MAX};

fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

/// The opt-in container-model channel: same packing envelope as
/// c3lib's `Rv64` (all three modes, round-tripped through the consensus
/// decoder), built on the containers entry instead.
struct Rv64Ctr {
    engines: Vec<(Rv64Mode, CalcFnObj)>,
}

impl Rv64Ctr {
    fn try_build(body: &str) -> Result<Self, String> {
        let (ir, _) = c3lib::frontend(body)?;
        let cap = vm::calc::SpaceCap::new(0);
        let mut engines = Vec::new();
        for &mode in c3lib::MODES.iter() {
            let lo = lower_body_mode_arena_containers(
                &ir,
                true,
                mode,
                ARENA_SIZE_MAX,
                16, // SpaceCap::HEAP_GROW_SEGMENT_DEFAULT (vm::rt is private)
            )
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

/// Four-way agreement on EVERY input: interpreter == all three rv64
/// container-model modes (Ok with identical bytes, or Err with the
/// identical exit code).
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64Ctr::try_build(body)
        .unwrap_or_else(|e| panic!("ctr rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body)
        .unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
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

fn ctr_must_reject(body: &str, frag: &str) {
    let err = Rv64Ctr::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("ctr rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
}

fn one_input() -> Vec<Vec<u8>> {
    vec![vec![1, 2, 3, 4]]
}

/* ================================================================== */
/* Section 1: alias / CLONE / re-bind — the shared-mutable semantics.  */
/* ================================================================== */

#[test]
fn ctr1_alias_append_visible_through_every_alias() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nappend(m, 3u64)\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nappend(l, 3u64)\nreturn length(m) as bytes",
        &one_input(),
    );
    agree4(
        "var a = [5u64]\nvar b = a\nvar c = b\nappend(c, 2u64)\nreturn length(a) as bytes",
        &one_input(),
    );
}

#[test]
fn ctr2_alias_itemset_visible() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nm[0] = 9u64\nreturn l[0] as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nl[1] = 44u64\nreturn (m[0] * 100u64 + m[1]) as bytes",
        &one_input(),
    );
}

#[test]
fn ctr3_alias_clear_and_variable_index_reads() {
    agree4(
        "var l = [1u64, 7u64]\nvar m = l\nclear(m)\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [7u64, 8u64, 9u64]\nvar m = l\nvar i = (byte(data, 0) as u64) % 3u64\nreturn m[i] as bytes",
        &(0u16..=255).map(|b| vec![b as u8, 5]).collect::<Vec<_>>(),
    );
    agree4(
        "var l = [7u64, 8u64]\nvar m = l\nvar i = (byte(data, 0) as u64) % 4u64\nreturn (has_key(m, i) as u64) as bytes",
        &(0u16..=255).map(|b| vec![b as u8]).collect::<Vec<_>>(),
    );
}

#[test]
fn ctr4_clone_is_the_only_deep_copy() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nappend(m, 3u64)\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nappend(m, 3u64)\nreturn length(m) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nm[0] = 7u64\nreturn l[0] as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nm[0] = 7u64\nreturn m[0] as bytes",
        &one_input(),
    );
    agree4(
        "var a = [1u64]\nvar b = a\nvar c = clone(b)\nappend(a, 9u64)\nreturn length(c) as bytes",
        &one_input(),
    );
    agree4(
        "var l = []\nvar m = clone(l)\nappend(m, 4u64)\nreturn length(l) as bytes",
        &one_input(),
    );
}

#[test]
fn ctr5_rebind_binds_a_fresh_object() {
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nl = [9u64]\nreturn length(m) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nl = [9u64]\nreturn (m[0] * 100u64 + m[1]) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nl = [9u64]\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nl = [3u64, 4u64, 5u64]\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64]\nl = []\nappend(l, 6u64)\nreturn l[0] as bytes",
        &one_input(),
    );
}

#[test]
fn ctr6_alias_mutation_in_loops() {
    agree4(
        "var l = []\nvar i = 0u64\nwhile i < 5u64 { append(l, i * i)\ni += 1 }\nvar m = l\nappend(m, 99u64)\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = []\nvar i = 0u64\nwhile i < 5u64 { append(l, i * i)\ni += 1 }\nvar m = l\nvar s = 0u64\nvar j = 0u64\nwhile j < 6u64 { s += m[j]\nj += 1 }\nreturn s as bytes",
        &one_input(),
    );
}

/* ================================================================== */
/* Section 2: element widths (the tagged cell carries lo/hi pairs).    */
/* ================================================================== */

#[test]
fn ctr7_u128_elements_alias_and_clone() {
    agree4(
        "var l = [1u128, 2u128]\nvar m = l\nappend(m, 340282366920938463463374607431768211455u128)\nreturn m[2] as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u128, 2u128]\nvar m = l\nappend(m, 3u128)\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [170141183460469231731687303715884105727u128]\nvar m = clone(l)\nappend(m, 1u128)\nreturn l[0] as bytes",
        &one_input(),
    );
}

#[test]
fn ctr8_narrow_elements_alias() {
    agree4(
        "var l = [1u32, 2u32]\nvar m = l\nappend(m, 3u32)\nreturn m[2] as bytes",
        &one_input(),
    );
    agree4(
        "var l = [7u8, 8u8]\nvar m = l\nappend(m, 9u8)\nreturn (m[2] as u64) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u16, 2u16]\nvar m = l\nm[1] = 999u16\nreturn (l[1] as u64) as bytes",
        &one_input(),
    );
}

/* ================================================================== */
/* Section 3: fault faces (iron rule: identical codes, not billing).   */
/* ================================================================== */

#[test]
fn ctr9_fault_faces_match_the_interpreter() {
    // index >= len through an alias -> 85
    agree4(
        "var l = [1u64]\nvar m = l\nappend(m, 2u64)\nvar r = m[2]\nreturn r as bytes",
        &one_input(),
    );
    // index >= 2^32 -> 72 (extract_u32 face, before the length check)
    agree4(
        "var l = [1u64]\nvar m = l\nvar i = 4294967296u64\nvar r = m[i]\nreturn r as bytes",
        &one_input(),
    );
    // item_set index >= len -> 85
    agree4(
        "var l = [1u64]\nvar m = l\nm[1] = 2u64\nreturn length(m) as bytes",
        &one_input(),
    );
    // the 128-element compo cap: append at len == 128 -> 40 on BOTH
    // engines (the arena object's capacity IS the interpreter's
    // compo_length default)
    agree4(
        "var l = []\nvar i = 0u64\nwhile i < 128u64 { append(l, i)\ni += 1 }\nvar m = l\nappend(m, 5u64)\nreturn length(m) as bytes",
        &one_input(),
    );
    // clear then read -> 85 (the shared header is the live state)
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nclear(m)\nvar r = l[0]\nreturn r as bytes",
        &one_input(),
    );
}

/* ================================================================== */
/* Section 4: the escape rejects that STAY (T17/T16 lower them later). */
/* ================================================================== */

#[test]
fn ctr10_container_model_keeps_the_t17_t16_rejects() {
    // T17 finished the list escape family — alias / re-bind / clone /
    // nested / take / merge / insert / remove / eq all LOWER on the
    // container model (rv64_t17_list_values.rs). The rejects that stay:
    // the ABI-level list return, maps (T16), an aliased item_concat, and
    // the compound item assignment on an arena list.
    ctr_must_reject(
        "var l = [1u64, 2u64]\nvar m = l\nreturn m",
        "RET",
    );
    ctr_must_reject(
        "var l = [1u64, 2u64]\nvar m = l\nvar r = item_concat(m)\nreturn r",
        "ITEMCAT",
    );
    ctr_must_reject(
        "var l = [1u64, 2u64]\nvar m = l\nm[0] += 5u64\nreturn 0u64 as bytes",
        "ITEMSET",
    );
    // a dropped CLONE statement has no admitted consumer
    ctr_must_reject(
        "var l = [1u64]\nclone(l)\nreturn 0u64 as bytes",
        "CLONE",
    );
    // T16 lowered the map FAMILY (vm/tests/rv64_map_family.rs); a map
    // VALUE still cannot escape as a returned/copied/comparable value.
    ctr_must_reject(
        "var m = new_map()\nreturn m",
        "RET",
    );
}

#[test]
fn ctr11_default_channel_is_the_container_model() {
    // T17 wired the arena container model into every lowering entry: the
    // historical escape shapes now BUILD on the default channel and agree
    // with the interpreter (the differentials below run all three modes).
    agree4(
        "var l = [1u64, 2u64]\nvar m = l\nreturn length(m) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nl = [3u64]\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = clone(l)\nappend(m, 3u64)\nreturn length(l) as bytes",
        &one_input(),
    );
    agree4(
        "var l = [1u64, 2u64]\nvar m = [l]\nreturn 0u64 as bytes",
        &one_input(),
    );
}

#[test]
fn ctr12_in_shape_lists_still_work_on_the_container_channel() {
    // the flat fast path is untouched: non-escaping lists lower
    // byte-identically even on the opt-in channel (no alias -> no Arena)
    agree4(
        "var l = [1u64, 2u64, 3u64]\nvar i = (byte(data, 0) as u64) % 3u64\nreturn l[i] as bytes",
        &(0u16..=255).map(|b| vec![b as u8]).collect::<Vec<_>>(),
    );
    agree4(
        "var l = []\nappend(l, 4u64)\nappend(l, 5u64)\nreturn l[1] as bytes",
        &one_input(),
    );
}
