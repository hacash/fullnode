//! Round-1 adversarial differential audit of the rv64 compiler — a
//! deterministic random/structured program generator that hammers the
//! four-way consistency (consensus interpreter == rv64 {Im, Imc,
//! ImcRemap}) over the semantic surfaces of the merged size
//! optimizations on rv64-size-min (campaign 1 at @9bfeda7, rerun after
//! the D2/D3 gating merge at @151f06f):
//!
//!   * B1/B3 — constant map-key pool staging + compile-time PACKMAP
//!     sort/direct store (duplicate keys -> runtime 84, value evaluation
//!     order, cross-width canonical-key collisions);
//!   * B2    — dynamic uint map-key normalization rewrite (canonical
//!     minimal BE, 0 -> [0x00], cross-width key collisions, descriptor
//!     shrink after remove/re-insert/merge);
//!   * C1/C2 — width-provable guard elimination (narrowing cast chains
//!     u8/u16/u32 at their 2^w-1 boundaries, list indices of every
//!     static width through ITEMGET/ITEMSET/INSERT/REMOVE/HASKEY, the
//!     kept 71/72/85 faces);
//!   * A6    — u128 div/mod two-stage routine (divisors < 2^64 and
//!     >= 2^64, division by zero -> 90);
//!   * A5    — SQRT/SQRTUP tail branch inversion (a dedicated
//!     hand-built-IR bombardment; SQRT has no fitsh source syntax);
//!   * D5/D6 — heap prologue zeroing counted loop (fresh-heap reads,
//!     far-end reads) and while dead back-edge elimination (unconditional
//!     break tails);
//!   * E1/B10 — pool constant loads (every constant site exercises the
//!     c.ldsp / u128 direct-store paths).
//!
//! The generator stays INSIDE the admitted kernel subset and inside the
//! harness envelope, so the iron law holds unconditionally for every
//! generated program: identical Ok bytes or the identical fault code on
//! all four engines. Envelope discipline (the pinned-divergence families
//! of the C1/C2 corpus, docs.loc/rv64im/70-c2-generator.md):
//!   * every `byte(data, i)` index is < 48 and every input is >= 128 B
//!     (D2: byte OOB interp 42 vs rv64 71);
//!   * shift amounts are bounded < 64 (D1: interp 90 vs rv64 &63 mask);
//!   * `choose()` / value-`if` arms are fault-free (vars/literals only —
//!     D9: eager-vs-lazy arm evaluation);
//!   * conditions are pure comparisons (BUG-014 class is fixed, but the
//!     family stays out of the corpus);
//!   * every var is declared unconditionally and initialized (D11 /
//!     BUG-012: may-Nil slot reads);
//!   * no `clamp` (D10), no assert/abort (D3), CAT chains stay far below
//!     the 1280 B value cap (D4) and every CAT / ITEMCONCAT total is a
//!     multiple of 8 (FINDING #2 frame-cursor misalignment, see
//!     audit_finding2_cat_align_repro / audit-round1.md §F2);
//!   * map value species are uniform per map, list element species
//!     uniform per list (the pinned B-family compile rejects);
//!   * `item_concat` totals stay <= 16 B: emit_itemcat checks the total
//!     against S3 = output_max (list.rs), while the interpreter's cap is
//!     the 1280 B value size — a PRE-EXISTING envelope boundary (blamed
//!     at 25806ba, before the size-opt series; minimized in
//!     audit-round1.md §findings).
//!
//! Run (the campaign tests are #[ignore] so the daily gate stays fast):
//!
//!     cargo test -p vm --offline --release --test rv64_adversarial_audit -- --ignored --nocapture
//!
//! Size knobs (deterministic seeds; generation is a pure function of the
//! seed): AUDIT_N (default 500 main programs), AUDIT_FAULT_N (default
//! 200 fault-injected programs), AUDIT_SQRT_N (default 300 SQRT values
//! per width), AUDIT_SEED0 (default 0xA0D5_C0DE).

#![allow(dead_code)]

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rng, Rv64};
use std::collections::BTreeMap;
use std::time::Instant;

use vm::ir::rv64::{lower_body_mode_arena, ARENA_SIZE_MAX, Rv64Mode};
use vm::ir::{IRNode, IRNodeArray, IRNodeParam1, IRNodeParam1Single, IRNodeSingle};
use vm::{Bytecode};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/// The u64 folding modulus — keeps every acc fold overflow-free so later
/// statements stay observable (fault faces are injected deliberately).
const P: &str = "1000000007u64";
const PU128: &str = "1000000007u128";

fn env_u64(name: &str, dflt: u64) -> u64 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(dflt)
}

/* ================================================================== */
/* Section 1: the generator.                                           */
/* ================================================================== */

/// Per-production usage counters (grammar coverage self-report).
#[derive(Default, Clone)]
struct Coverage(BTreeMap<String, u64>);

impl Coverage {
    fn hit(&mut self, prod: &str) {
        *self.0.entry(prod.to_string()).or_insert(0) += 1;
    }
    fn merge(&mut self, other: &Coverage) {
        for (k, v) in other.0.iter() {
            *self.0.entry(k.clone()).or_insert(0) += v;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Species {
    U64,
    U128,
    Bytes,
}

struct MapVar {
    name: String,
    species: Species,
    /// canonical numeric values of constant keys known present (dynamic
    /// inserts also mark the map "has dynamic keys" — reads may 85)
    const_keys: Vec<u128>,
    has_dynamic: bool,
    /// FINDING #1 avoidance (defense in depth): a PACKMAP-created map
    /// that appears AFTER a dynamic-key map read must not be mutated
    /// later (the map-family hard-trap — see audit_finding1_map_trap
    /// and audit-round1.md §F1); reads/length/keys stay admitted. The
    /// generator's dyn_key already casts narrow vars so the arming
    /// shape cannot be produced; this freeze guards the read face.
    frozen: bool,
}

struct ListVar {
    name: String,
    width: &'static str,
    /// true when every element is a literal (the rodata class — INSERT
    /// on it compile-rejects; APPEND/REMOVE are admitted)
    all_literal: bool,
    /// static length upper bound
    len_bound: u64,
    aliased: bool,
}

struct BytesVar {
    name: String,
    /// static length bound
    bound: u64,
}

struct Gen {
    rng: Rng,
    cov: Coverage,
    fault: bool,
    maps: Vec<MapVar>,
    lists: Vec<ListVar>,
    bytes_vars: Vec<BytesVar>,
    /// the single admitted growable/mutated list (frame budget: two
    /// 1024 B frame blocks would exceed the 2040 B S0 window)
    mutated_list: Option<String>,
    /// heap_grow already emitted (at most one per path)
    heap_grown: bool,
    /// FINDING #1 avoidance: set once a dynamic-key map READ has been
    /// emitted — every PACKMAP created afterwards is marked frozen
    /// (mutation would compile into a hard engine trap).
    dyn_read_emitted: bool,
    counter: u64,
}

impl Gen {
    fn new(seed: u64, fault: bool) -> Self {
        Gen {
            rng: Rng::new(seed),
            cov: Coverage::default(),
            fault,
            maps: vec![],
            lists: vec![],
            bytes_vars: vec![],
            mutated_list: None,
            heap_grown: false,
            dyn_read_emitted: false,
            counter: 0,
        }
    }

    fn fresh(&mut self, pfx: &str) -> String {
        let n = format!("{pfx}{}", self.counter);
        self.counter += 1;
        n
    }

    /* ---------------- shared expression helpers ---------------- */

    /// A u64 literal with suffix.
    fn lit64(&mut self, hi: bool) -> String {
        let v = if hi {
            match self.rng.below(6) {
                0 => u64::MAX,
                1 => u64::MAX - self.rng.below(3),
                2 => 1u64 << self.rng.range(32, 63),
                3 => (1u64 << 32) + self.rng.below(4),
                _ => self.rng.below(1 << 20),
            }
        } else {
            self.rng.below(256)
        };
        format!("{v}u64")
    }

    /// A fault-free u64 expression over the prologue vars (folded into
    /// acc; every site stays inside u64 so no accidental 90).
    fn u64_expr(&mut self) -> String {
        match self.rng.below(6) {
            0 => self.lit64(false),
            1 => "(c2 % 7u64)".into(),
            2 => "((n8 as u64) + 1u64)".into(),
            3 => "(c1 % 1000003u64)".into(),
            4 => "(c3 + 1u64)".into(),
            _ => "(c0 % 97u64)".into(),
        }
    }

    /// A fault-free u128 expression.
    fn u128_expr(&mut self) -> String {
        match self.rng.below(5) {
            0 => "340282366920938463463374607431768211455u128".into(),
            1 => "18446744073709551616u128".into(),
            2 => "(w0 % 1000000007u128)".into(),
            3 => "((n8 as u128) + 1u128)".into(),
            _ => "(w1 % 4294967291u128)".into(),
        }
    }

    /* ---------------- map statements ---------------- */

    /// A constant map key: (numeric value, width). The value pool parks
    /// on the canonical-normalization boundaries (B2/B3 collision face).
    fn const_key(&mut self) -> (u128, &'static str) {
        let v: u128 = match self.rng.below(12) {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 255,
            4 => 256,
            5 => 65535,
            6 => 65536,
            7 => 4294967295,
            8 => 4294967296,
            9 => u64::MAX as u128,
            10 => u128::MAX,
            _ => self.rng.below(4096) as u128,
        };
        // pick a width the value fits (cross-width same-value keys are
        // the B2/B3 collision face — allowed, incl. in literals)
        let widths: Vec<&'static str> = ["u8", "u16", "u32", "u64", "u128"]
            .iter()
            .copied()
            .filter(|w| match *w {
                "u8" => v < 256,
                "u16" => v < 65536,
                "u32" => v < 4294967296,
                "u64" => v <= u64::MAX as u128,
                _ => true,
            })
            .collect();
        let w = *self.rng.pick(&widths);
        (v, w)
    }

    /// A dynamic map key expression (B2 rewrite surface). Narrow live
    /// vars are ALWAYS wrapped in a widening cast: a bare narrow live
    /// var (u8/u16/u32) used as a map key arms FINDING #1 (the backend
    /// staging corruption — see audit_finding1_map_trap_repro /
    /// audit_finding1b_write_arm_repro and audit-round1.md §F1), while
    /// cast-expression and u64/u128-typed keys are verified safe faces
    /// of the same B2 normalization surface.
    fn dyn_key(&mut self) -> String {
        match self.rng.below(5) {
            0 => "(n8 as u64)".into(),
            1 => "(n16 as u64)".into(),
            2 => "(n32 as u64)".into(),
            3 => "(c0 % 11u64)".into(),
            _ => "(w2 % 19u128)".into(),
        }
    }

    /// A key expression: constant (any width) or dynamic.
    fn any_key(&mut self) -> String {
        if self.rng.chance(55) {
            let (v, w) = self.const_key();
            format!("{v}{w}")
        } else {
            self.dyn_key()
        }
    }

    fn value_expr(&mut self, sp: Species) -> String {
        match sp {
            Species::U64 => self.u64_expr(),
            Species::U128 => self.u128_expr(),
            Species::Bytes => {
                let n = self.rng.range(1, 8);
                format!("buf_left({n}, data)")
            }
        }
    }

    /// `var mN = <literal>` / `var mN = new_map()` — creation is top
    /// level only (plan rejects conditional declarations).
    fn s_map_create(&mut self) -> String {
        self.cov.hit("S_map_create");
        let name = self.fresh("m");
        let species = match self.rng.below(6) {
            0 | 1 | 2 | 3 => Species::U64,
            4 => Species::U128,
            _ => Species::Bytes,
        };
        if self.rng.chance(35) {
            self.maps.push(MapVar {
                name: name.clone(),
                species,
                const_keys: vec![],
                has_dynamic: false,
                frozen: false,
            });
            return format!("var {name} = new_map()");
        }
        let npairs = self.rng.range(1, 5);
        let mut pairs: Vec<String> = Vec::new();
        let mut used: Vec<u128> = Vec::new();
        for _ in 0..npairs {
            // duplicate-key injection: reuse a used value (any width —
            // the canonical bytes collide) with ~18% / ~60% in fault mode
            let dup = !used.is_empty()
                && self.rng.chance(if self.fault { 60 } else { 18 });
            let (v, w) = if dup {
                let v = *self.rng.pick(&used);
                let widths: Vec<&'static str> = ["u8", "u16", "u32", "u64", "u128"]
                    .iter()
                    .copied()
                    .filter(|w| match *w {
                        "u8" => v < 256,
                        "u16" => v < 65536,
                        "u32" => v < 4294967296,
                        "u64" => v <= u64::MAX as u128,
                        _ => true,
                    })
                    .collect();
                (v, *self.rng.pick(&widths))
            } else {
                let (v, w) = self.const_key();
                used.push(v);
                (v, w)
            };
            let val = self.value_expr(species);
            pairs.push(format!("{v}{w} : {val}"));
        }
        // ~25%: one dynamic parenthesized key pair (the pairwise path)
        if self.rng.chance(25) {
            let k = self.dyn_key();
            let val = self.value_expr(species);
            if self.rng.chance(50) {
                pairs.push(format!("({k}) : {val}"));
            } else {
                pairs.insert(0, format!("({k}) : {val}"));
            }
        }
        self.maps.push(MapVar {
            name: name.clone(),
            species,
            const_keys: used,
            has_dynamic: false,
            frozen: self.dyn_read_emitted,
        });
        format!("var {name} = map{{{}}}", pairs.join(", "))
    }

    /// Indices of maps that may still be mutated (frozen = PACKMAP
    /// created after a dynamic-key read — FINDING #1).
    fn mutable_maps(&self) -> Vec<usize> {
        (0..self.maps.len()).filter(|i| !self.maps[*i].frozen).collect()
    }

    fn s_map_insert(&mut self) -> String {
        let mutables = self.mutable_maps();
        if mutables.is_empty() {
            return self.s_map_read();
        }
        self.cov.hit("S_map_insert");
        let i = *self.rng.pick(&mutables);
        let (name, sp) = (self.maps[i].name.clone(), self.maps[i].species);
        let key = self.any_key();
        if !key.starts_with(|c: char| c.is_ascii_digit()) {
            self.maps[i].has_dynamic = true;
        } else {
            let v: u128 = key[..key.len() - 3].parse().unwrap_or(0);
            // insert overwrites: the numeric key set only grows
            if !self.maps[i].const_keys.contains(&v) {
                self.maps[i].const_keys.push(v);
            }
        }
        let val = self.value_expr(sp);
        format!("insert({name}, {key}, {val})")
    }

    fn s_map_read(&mut self) -> String {
        if self.maps.is_empty() {
            return self.s_acc_fold();
        }
        self.cov.hit("S_map_read");
        let i = self.rng.below(self.maps.len() as u64) as usize;
        let (name, sp) = (self.maps[i].name.clone(), self.maps[i].species);
        // key: a known-present constant (Ok path) / a dynamic expr (85
        // face when absent — an agreed trap) / fault-mode missing key
        let key = if !self.maps[i].const_keys.is_empty() && self.rng.chance(if self.fault { 25 } else { 70 })
        {
            let v = *self.rng.pick(&self.maps[i].const_keys);
            let w = if v <= u64::MAX as u128 { "u64" } else { "u128" };
            format!("{v}{w}")
        } else if self.fault && self.rng.chance(40) {
            let (v, w) = self.const_key();
            format!("{v}{w}")
        } else {
            self.dyn_read_emitted = true;
            self.dyn_key()
        };
        match sp {
            Species::U64 => format!("acc = (acc * 31u64 + ({name}[{key}] as u64)) % {P}"),
            Species::U128 => format!(
                "acc = (acc * 31u64 + (({name}[{key}] % {PU128}) as u64)) % {P}"
            ),
            Species::Bytes => {
                let b = self.fresh("bv");
                self.bytes_vars.push(BytesVar { name: b.clone(), bound: 8 });
                format!(
                    "var {b} = {name}[{key}]\nacc = (acc * 31u64 + (size({b}) as u64)) % {P}"
                )
            }
        }
    }

    fn s_map_remove(&mut self) -> String {
        let mutables = self.mutable_maps();
        if mutables.is_empty() {
            return self.s_map_read();
        }
        self.cov.hit("S_map_remove");
        let i = *self.rng.pick(&mutables);
        let name = self.maps[i].name.clone();
        let key = if !self.maps[i].const_keys.is_empty() && self.rng.chance(70) {
            let v = *self.rng.pick(&self.maps[i].const_keys);
            self.maps[i].const_keys.retain(|k| *k != v);
            let w = if v <= u64::MAX as u128 { "u64" } else { "u128" };
            format!("{v}{w}")
        } else if self.fault {
            // fault face: remove a missing key -> 85
            let (v, w) = self.const_key();
            format!("{v}{w}")
        } else {
            self.dyn_key()
        };
        format!("remove({name}, {key})")
    }

    fn s_map_itemset(&mut self) -> String {
        let mutables = self.mutable_maps();
        if mutables.is_empty() {
            return self.s_map_read();
        }
        self.cov.hit("S_map_itemset");
        let i = *self.rng.pick(&mutables);
        let (name, sp) = (self.maps[i].name.clone(), self.maps[i].species);
        let key = if !self.maps[i].const_keys.is_empty() && self.rng.chance(75) {
            let v = *self.rng.pick(&self.maps[i].const_keys);
            let w = if v <= u64::MAX as u128 { "u64" } else { "u128" };
            format!("{v}{w}")
        } else if self.fault {
            let (v, w) = self.const_key();
            format!("{v}{w}")
        } else {
            self.dyn_key()
        };
        let val = self.value_expr(sp);
        if self.rng.chance(50) {
            format!("{name}[{key}] = {val}")
        } else {
            format!("item_set({name}, {key}, {val})")
        }
    }

    fn s_map_haskey(&mut self) -> String {
        if self.maps.is_empty() {
            return self.s_acc_fold();
        }
        self.cov.hit("S_map_haskey");
        let m = self.rng.pick(&self.maps).name.clone();
        let key = self.any_key();
        if !key.starts_with(|c: char| c.is_ascii_digit()) {
            self.dyn_read_emitted = true;
        }
        format!("acc = (acc * 2u64 + (has_key({m}, {key}) as u64)) % {P}")
    }

    fn s_map_keys_values(&mut self) -> String {
        if self.maps.is_empty() {
            return self.s_acc_fold();
        }
        self.cov.hit("S_map_keys_values");
        let i = self.rng.below(self.maps.len() as u64) as usize;
        let m = self.maps[i].name.clone();
        let sp = self.maps[i].species;
        let ks = self.fresh("ks");
        let mut out = String::new();
        if self.rng.chance(50) {
            out.push_str(&format!("var {ks} = keys({m})\n"));
            out.push_str(&format!("acc = (acc * 31u64 + (length({ks}) as u64)) % {P}"));
            if self.rng.chance(70) {
                let idx = if self.maps[i].const_keys.len() > 1 || self.maps[i].has_dynamic {
                    format!("(c3 % {}u64)", (self.maps[i].const_keys.len().max(1)) as u64)
                } else {
                    "0u64".to_string()
                };
                out.push_str(&format!(
                    "\nacc = (acc * 31u64 + (({ks}[{idx}] % {PU128}) as u64)) % {P}"
                ));
            }
        } else {
            out.push_str(&format!("var {ks} = values({m})\n"));
            match sp {
                Species::U64 => {
                    out.push_str(&format!(
                        "acc = (acc * 31u64 + (length({ks}) as u64)) % {P}\n"
                    ));
                    if self.rng.chance(60) {
                        out.push_str(&format!(
                            "acc = (acc * 31u64 + ({ks}[0u64] as u64)) % {P}"
                        ));
                    }
                }
                Species::U128 => {
                    out.push_str(&format!(
                        "acc = (acc * 31u64 + (length({ks}) as u64)) % {P}\n"
                    ));
                    if self.rng.chance(60) {
                        out.push_str(&format!(
                            "acc = (acc * 31u64 + (({ks}[0u64] % {PU128}) as u64)) % {P}"
                        ));
                    }
                }
                Species::Bytes => {
                    // snapshot of bytes values: fold the first one's size
                    let b = self.fresh("vb");
                    out.push_str(&format!(
                        "acc = (acc * 31u64 + (length({ks}) as u64)) % {P}\nvar {b} = {ks}[0u64]\nacc = (acc * 31u64 + (size({b}) as u64)) % {P}"
                    ));
                }
            }
        }
        out
    }

    fn s_map_merge(&mut self) -> String {
        if self.maps.len() < 2 {
            return self.s_map_insert();
        }
        self.cov.hit("S_map_merge");
        // pick two same-species maps; fault mode prefers colliding keys
        let mut cands: Vec<(usize, usize)> = Vec::new();
        for a in 0..self.maps.len() {
            if self.maps[a].frozen {
                continue; // merge destination mutates — FINDING #1
            }
            for b in 0..self.maps.len() {
                if a != b && self.maps[a].species == self.maps[b].species {
                    cands.push((a, b));
                }
            }
        }
        if cands.is_empty() {
            return self.s_map_insert();
        }
        let (a, b) = if self.fault {
            // prefer a colliding pair when one exists
            let col: Vec<(usize, usize)> = cands
                .iter()
                .copied()
                .filter(|(x, y)| {
                    self.maps[*x].const_keys.iter().any(|k| self.maps[*y].const_keys.contains(k))
                })
                .collect();
            if col.is_empty() {
                *self.rng.pick(&cands)
            } else {
                *self.rng.pick(&col)
            }
        } else {
            // Ok-path bias: disjoint const keys, no dynamic keys on b
            let ok: Vec<(usize, usize)> = cands
                .iter()
                .copied()
                .filter(|(x, y)| {
                    !self.maps[*y].has_dynamic
                        && !self.maps[*x]
                            .const_keys
                            .iter()
                            .any(|k| self.maps[*y].const_keys.contains(k))
                })
                .collect();
            if ok.is_empty() {
                *self.rng.pick(&cands)
            } else {
                *self.rng.pick(&ok)
            }
        };
        let (an, bn) = (self.maps[a].name.clone(), self.maps[b].name.clone());
        // merged numeric key set for later read tracking
        for k in self.maps[b].const_keys.clone() {
            if !self.maps[a].const_keys.contains(&k) {
                self.maps[a].const_keys.push(k);
            }
        }
        if self.maps[b].has_dynamic {
            self.maps[a].has_dynamic = true;
        }
        format!("merge({an}, {bn})")
    }

    /* ---------------- list statements ---------------- */

    fn s_list_create(&mut self) -> String {
        self.cov.hit("S_list_create");
        let name = self.fresh("l");
        let width = *self.rng.pick(&["u8", "u16", "u32", "u64", "u128"]);
        let n = self.rng.range(1, 5) as usize;
        let mut elems = Vec::new();
        let mut all_literal = true;
        for k in 0..n {
            if self.rng.chance(40) && width != "u128" {
                all_literal = false;
                let src = 2 + k;
                elems.push(match width {
                    "u8" => format!("byte(data, {src})"),
                    "u16" => format!("(byte(data, {src}) as u16)"),
                    "u32" => format!("(byte(data, {src}) as u32)"),
                    _ => format!("((byte(data, {src}) as u64) + {k}u64)"),
                });
            } else {
                let v = self.rng.below(match width {
                    "u8" => 256,
                    "u16" => 65536,
                    "u32" => 4294967296,
                    "u64" => 1 << 40,
                    _ => u64::MAX as u64,
                });
                elems.push(format!("{v}{width}"));
            }
        }
        self.lists.push(ListVar {
            name: name.clone(),
            width,
            all_literal,
            len_bound: n as u64,
            aliased: false,
        });
        format!("var {name} = [{}]", elems.join(", "))
    }

    /// A list index expression (C2 surface): narrow-width vars, wide
    /// vars, constants — in or (deliberately) out of bounds.
    fn list_idx(&mut self, len_bound: u64, oob_ok: bool) -> String {
        let m = if len_bound == 0 { 2 } else { len_bound };
        match self.rng.below(7) {
            0 => format!("(n8 % {m}u64)"),                    // u8 width
            1 => format!("(n16 % {m}u64)"),                   // u16 width
            2 => format!("(n32 % {m}u64)"),                   // u32 width
            3 => format!("(c3 % {m}u64)"),                    // u64 width
            4 if len_bound > 0 => format!("{}u64", self.rng.below(len_bound)), // const
            5 if oob_ok => format!("n8"),                     // may OOB -> 85
            _ => format!("(n8 % {}u64)", m + 1),              // == len -> 85 face
        }
    }

    fn elem_expr(&mut self, width: &str) -> String {
        match width {
            "u8" => format!("{}u8", self.rng.below(256)),
            "u16" => format!("{}u16", self.rng.below(65536)),
            "u32" => format!("{}u32", self.rng.below(4294967296)),
            "u64" => format!("{}u64", self.rng.below(1 << 40)),
            _ => format!("{}u128", self.rng.below(u64::MAX as u64)),
        }
    }

    fn s_list_read(&mut self) -> String {
        if self.lists.is_empty() {
            return self.s_list_create();
        }
        self.cov.hit("S_list_read");
        let i = self.rng.below(self.lists.len() as u64) as usize;
        let (name, width, lb) = (
            self.lists[i].name.clone(),
            self.lists[i].width,
            self.lists[i].len_bound,
        );
        let idx = self.list_idx(lb, self.fault);
        let fold = match width {
            "u128" => format!("(({name}[{idx}] % {PU128}) as u64)"),
            _ => format!("({name}[{idx}] as u64)"),
        };
        format!("acc = (acc * 31u64 + {fold}) % {P}")
    }

    fn s_list_write(&mut self) -> String {
        if self.lists.is_empty() {
            return self.s_list_create();
        }
        self.cov.hit("S_list_write");
        let i = self.rng.below(self.lists.len() as u64) as usize;
        let (name, width, lb) = (
            self.lists[i].name.clone(),
            self.lists[i].width,
            self.lists[i].len_bound,
        );
        let idx = self.list_idx(lb, self.fault);
        let val = self.elem_expr(width);
        if self.rng.chance(50) {
            format!("{name}[{idx}] = {val}")
        } else {
            format!("item_set({name}, {idx}, {val})")
        }
    }

    fn s_list_append(&mut self) -> String {
        if self.lists.is_empty() {
            return self.s_list_create();
        }
        // growable budget: one mutated list per program
        let free = match &self.mutated_list {
            None => true,
            Some(n) => self.lists.iter().any(|l| &l.name == n),
        };
        if !free {
            return self.s_list_read();
        }
        self.cov.hit("S_list_append");
        let i = self.rng.below(self.lists.len() as u64) as usize;
        let (name, width) = (self.lists[i].name.clone(), self.lists[i].width);
        if self.lists[i].len_bound >= 24 {
            return self.s_list_read();
        }
        self.mutated_list = Some(name.clone());
        self.lists[i].len_bound += 1;
        let val = self.elem_expr(width);
        format!("append({name}, {val})")
    }

    fn s_list_insert_remove(&mut self) -> String {
        if self.lists.is_empty() {
            return self.s_list_create();
        }
        // INSERT on an all-literal (rodata) list compile-rejects: pick a
        // frame-static (computed-element) receiver, else fall through
        let cands: Vec<usize> = self
            .lists
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.all_literal)
            .map(|(i, _)| i)
            .collect();
        if cands.is_empty() || self.rng.chance(30) {
            // REMOVE is admitted on every class
            self.cov.hit("S_list_remove");
            let i = self.rng.below(self.lists.len() as u64) as usize;
            let (name, lb) = (self.lists[i].name.clone(), self.lists[i].len_bound);
            if lb > 0 {
                self.lists[i].len_bound -= 1;
            }
            self.mutated_list = Some(name.clone());
            let idx = self.list_idx(lb.saturating_sub(1), self.fault);
            return format!("remove({name}, {idx})");
        }
        let free = match &self.mutated_list {
            None => true,
            Some(n) => self.lists.iter().any(|l| &l.name == n),
        };
        if !free {
            return self.s_list_read();
        }
        self.cov.hit("S_list_insert");
        let i = *self.rng.pick(&cands);
        let (name, width, lb) = (
            self.lists[i].name.clone(),
            self.lists[i].width,
            self.lists[i].len_bound,
        );
        // insert at == len is legal (append position); past len -> 81
        let idx = if self.fault && self.rng.chance(30) {
            format!("(n8 % {}u64)", lb + 3)
        } else {
            format!("(n8 % {}u64)", lb + 1)
        };
        self.mutated_list = Some(name.clone());
        self.lists[i].len_bound += 1;
        let val = self.elem_expr(width);
        format!("insert({name}, {idx}, {val})")
    }

    fn s_list_folds(&mut self) -> String {
        if self.lists.is_empty() {
            return self.s_list_create();
        }
        self.cov.hit("S_list_folds");
        let l = self.rng.pick(&self.lists);
        match self.rng.below(3) {
            0 => format!("acc = (acc * 31u64 + (length({}) as u64)) % {P}", l.name),
            1 => format!(
                "acc = (acc * 2u64 + (has_key({}, (n8 % {}u64)) as u64)) % {P}",
                l.name,
                (l.len_bound + 1).max(1)
            ),
            _ => format!(
                "acc = (acc * 2u64 + (has_key({}, {}u64) as u64)) % {P}",
                l.name,
                self.rng.below(l.len_bound.max(1))
            ),
        }
    }

    fn s_itemcat(&mut self) -> String {
        // item_concat: total = n * stride must stay <= 16 (S3 envelope)
        // AND be a multiple of 8 (FINDING #2 envelope — a non-multiple-
        // of-8 ITEMCONCAT destination misaligns the frame cursor and any
        // later PACKMAP literal hard-traps on rv64; see
        // audit_finding2_cat_align_repro), receiver never aliased
        let cands: Vec<usize> = self
            .lists
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let total = l.len_bound
                    * match l.width {
                        "u8" => 1,
                        "u16" => 2,
                        "u32" => 4,
                        "u64" => 8,
                        _ => 16,
                    };
                !l.aliased && total <= 16 && total % 8 == 0
            })
            .map(|(i, _)| i)
            .collect();
        if cands.is_empty() {
            return self.s_list_folds();
        }
        self.cov.hit("S_itemcat");
        let i = *self.rng.pick(&cands);
        let name = self.lists[i].name.clone();
        let b = self.fresh("cb");
        let stride = match self.lists[i].width {
            "u8" => 1,
            "u16" => 2,
            "u32" => 4,
            "u64" => 8,
            _ => 16,
        };
        self.bytes_vars.push(BytesVar {
            name: b.clone(),
            bound: self.lists[i].len_bound * stride,
        });
        format!(
            "var {b} = item_concat({name})\nacc = (acc * 31u64 + (size({b}) as u64)) % {P}"
        )
    }

    fn s_take(&mut self) -> String {
        if self.lists.is_empty() {
            return self.s_list_folds();
        }
        let free = match &self.mutated_list {
            None => true,
            Some(n) => self.lists.iter().any(|l| &l.name == n),
        };
        if !free {
            return self.s_list_folds();
        }
        self.cov.hit("S_take");
        let i = self.rng.below(self.lists.len() as u64) as usize;
        let (name, width, lb) = (
            self.lists[i].name.clone(),
            self.lists[i].width,
            self.lists[i].len_bound,
        );
        if lb == 0 {
            // take on an empty list — agreed fault face
            let t = self.fresh("tk");
            return format!("var {t} = take_first({name})\nacc = (acc + ({t} as u64)) % {P}");
        }
        self.mutated_list = Some(name.clone());
        self.lists[i].len_bound -= 1;
        let op = if self.rng.chance(50) { "take_first" } else { "take_last" };
        if width == "u128" {
            format!("acc = (acc * 31u64 + (({op}({name}) % {PU128}) as u64)) % {P}")
        } else {
            format!("acc = (acc * 31u64 + ({op}({name}) as u64)) % {P}")
        }
    }

    /* ---------------- scalar statements ---------------- */

    /// A generic safe acc fold (filler).
    fn s_acc_fold(&mut self) -> String {
        self.cov.hit("S_acc_fold");
        let e = self.u64_expr();
        format!("acc = (acc * 31u64 + ({e} % 97u64)) % {P}")
    }

    /// Narrowing cast chains (C1) — over vars only (literal casts fold
    /// and reject out-of-range at the front end).
    fn s_cast_chain(&mut self) -> String {
        self.cov.hit("S_cast_chain");
        let srcs = ["c0", "c1", "n8", "n16", "n32", "w0"];
        let s = *self.rng.pick(&srcs);
        let w1 = *self.rng.pick(&["u8", "u16", "u32"]);
        let w2 = *self.rng.pick(&["u8", "u16", "u32", "u64"]);
        let t = self.fresh("t");
        if self.rng.chance(50) {
            // single narrowing + re-widen (the C1-eliminated/kept guards)
            format!("var {t} = (({s} as {w1}) as u64)\nacc = (acc * 31u64 + ({t} % 97u64)) % {P}")
        } else {
            // chain of two casts
            format!(
                "var {t} = ((({s} as {w1}) as {w2}) as u64)\nacc = (acc * 31u64 + ({t} % 97u64)) % {P}"
            )
        }
    }

    /// u128 div/mod (A6): divisors both sides of 2^64; division by a
    /// runtime zero only in fault mode.
    fn s_u128_divmod(&mut self) -> String {
        self.cov.hit("S_u128_divmod");
        let op = if self.rng.chance(50) { "/" } else { "%" };
        let numer: String = match self.rng.below(4) {
            0 => "w0".into(),
            1 => "(w0 * w1)".into(),
            2 => "(w1 + 340282366920938463463374607431768211455u128)".into(),
            _ => "((c0 % 1000003u64) as u128)".into(),
        };
        let divisor: String = if self.fault && self.rng.chance(35) {
            // runtime zero -> 90 (an agreed face both engines must hit)
            "((n8 as u128) * (n8 % 2u128))".to_string()
        } else {
            match self.rng.below(3) {
                0 => "((n8 as u128) + 1u128)".to_string(),                       // < 2^64
                1 => "(18446744073709551616u128 + (n16 as u128))".to_string(),   // >= 2^64
                _ => "((w1 % 18446744073709551615u128) + 1u128)".to_string(),    // straddles
            }
        };
        let t = self.fresh("q");
        format!(
            "var {t} = ({numer} {op} {divisor})\nacc = (acc * 31u64 + (({t} % {PU128}) as u64)) % {P}"
        )
    }

    /// u64 div/mod with safe divisors (fault mode: runtime zero -> 90).
    fn s_u64_divmod(&mut self) -> String {
        self.cov.hit("S_u64_divmod");
        let op = if self.rng.chance(50) { "/" } else { "%" };
        let divisor = if self.fault && self.rng.chance(35) {
            "((n8 as u64) * (n8 % 2u64))".to_string()
        } else {
            "((n8 as u64) + 1u64)".to_string()
        };
        format!("acc = (acc {op} {divisor}) % {P}")
    }

    fn s_pow(&mut self) -> String {
        self.cov.hit("S_pow");
        let t = self.fresh("pw");
        if self.rng.chance(40) {
            // u128 pow (exp <= 255 as u128 — tiny, in range)
            format!(
                "var {t} = (3u128 ** (n8 as u128))\nacc = (acc * 31u64 + (({t} % {PU128}) as u64)) % {P}"
            )
        } else if self.fault && self.rng.chance(25) {
            // u64 pow overflow -> 90
            format!("var {t} = (2u64 ** 64u64)\nacc = (acc + {t}) % {P}")
        } else {
            format!("var {t} = (3u64 ** (n8 % 12u64))\nacc = (acc * 31u64 + {t}) % {P}")
        }
    }

    fn s_shift(&mut self) -> String {
        self.cov.hit("S_shift");
        let amt = match self.rng.below(3) {
            0 => "(n8 % 8u64)".to_string(),
            1 => "(n16 % 63u64)".to_string(),
            _ => "7u64".to_string(),
        };
        let op = if self.rng.chance(50) { "<<" } else { ">>" };
        format!("acc = ((acc {op} {amt}) + (c2 % 7u64)) % {P}")
    }

    fn s_arith(&mut self) -> String {
        self.cov.hit("S_arith");
        if self.fault && self.rng.chance(30) {
            // overflow face -> 90
            self.cov.hit("S_arith_ovf");
            "acc = (18446744073709551615u64 * (c3 + 1u64))".to_string()
        } else {
            let e = self.u64_expr();
            format!("acc = (acc + ({e} % 13u64)) % {P}")
        }
    }

    fn s_cat(&mut self) -> String {
        self.cov.hit("S_cat");
        // FINDING #2 envelope: every CAT total length MUST be a multiple
        // of 8 (a non-multiple-of-8 CAT destination slot misaligns the
        // frame cursor and any PACKMAP literal allocated afterwards
        // hard-traps on rv64 — audit_finding2_cat_align_repro). The
        // chained re-CAT appends exactly 8 so the invariant holds.
        let n1 = self.rng.range(1, 8);
        let n2 = 8 - n1;
        let b = self.fresh("s");
        let bound = n1 + n2;
        self.bytes_vars.push(BytesVar { name: b.clone(), bound });
        let (e1, e2) = if self.rng.chance(50) {
            (format!("buf_left({n1}, data)"), format!("buf_right({n2}, data)"))
        } else {
            (format!("buf_right({n1}, data)"), format!("buf_left({n2}, data)"))
        };
        let mut out = format!("var {b} = {e1} ++ {e2}\n");
        if self.rng.chance(40) {
            out.push_str(&format!("{b} = {b} ++ buf_left(8, data)\n"));
            self.bytes_vars.last_mut().unwrap().bound += 8;
        }
        out.push_str(&format!("acc = (acc * 31u64 + (size({b}) as u64)) % {P}"));
        out
    }

    fn s_choose(&mut self) -> String {
        self.cov.hit("S_choose");
        let a = self.u64_expr();
        let b = self.u64_expr();
        let cond = match self.rng.below(4) {
            0 => "(c2 < 128u64)".to_string(),
            1 => "(n8 == n16 % 256u64)".to_string(),
            2 => "((c0 % 3u64) == 0u64)".to_string(),
            _ => "(n8 < 200u8)".to_string(),
        };
        let t = self.fresh("ch");
        if self.rng.chance(50) {
            // CHOOSE node (ternary family): eager bytecode arms are
            // fault-free vars/lits, so laziness is unobservable (D9-safe)
            format!("var {t} = choose({cond}, {a}, {b})\nacc = (acc * 31u64 + ({t} % 97u64)) % {P}")
        } else {
            // value-position if (IRIFR — lazy on both engines)
            format!("var {t} = if {cond} {{ {a} }} else {{ {b} }}\nacc = (acc * 31u64 + ({t} % 97u64)) % {P}")
        }
    }

    fn s_while(&mut self) -> String {
        self.cov.hit("S_while");
        let trips = self.rng.range(2, 15);
        let iv = self.fresh("i");
        let mut body = String::new();
        match self.rng.below(5) {
            0 if !self.mutable_maps().is_empty() => {
                self.cov.hit("S_while_map_insert");
                let mutables = self.mutable_maps();
                let m = self.maps[*self.rng.pick(&mutables)].name.clone();
                let sp = self.maps.iter().find(|x| x.name == m).unwrap().species;
                // NOTE: build the value from `iv` eagerly — a literal
                // "{iv}" inside a substituted format arg is NOT
                // re-scanned by format! and would leak into the body.
                let val = match sp {
                    Species::U64 => format!("({iv} * 2u64)"),
                    Species::U128 => format!("(({iv} as u128) * 3u128)"),
                    Species::Bytes => "buf_left(2, data)".to_string(),
                };
                body.push_str(&format!("insert({m}, {iv} % 9u64, {val})\n"));
            }
            1 if !self.mutable_maps().is_empty() => {
                self.cov.hit("S_while_map_remove");
                let mutables = self.mutable_maps();
                let m = self.maps[*self.rng.pick(&mutables)].name.clone();
                body.push_str(&format!("remove({m}, {iv} % 7u64)\n"));
            }
            2 if self.mutated_list.is_none() && !self.lists.is_empty() => {
                self.cov.hit("S_while_list_append");
                let i = self.rng.below(self.lists.len() as u64) as usize;
                let (lname, w) = (self.lists[i].name.clone(), self.lists[i].width);
                self.mutated_list = Some(lname.clone());
                self.lists[i].len_bound += trips;
                let val = self.elem_expr(w);
                body.push_str(&format!("append({lname}, {val})\n"));
            }
            _ => body.push_str(&format!("acc = (acc * 3u64 + {iv}) % {P}\n")),
        }
        // D6 shape: the body tail cannot fall out — the backedge is dead
        match self.rng.below(4) {
            0 => body.push_str(&format!("if {iv} == c3 {{ break }}\n")),
            1 => body.push_str("break\n"), // unconditional tail break (D6)
            2 => body.push_str(&format!("if {iv} == (c3 + 1u64) {{ continue }}\n")),
            _ => {}
        }
        format!("var {iv} = 0u64\nwhile {iv} < {trips}u64 {{ {iv} += 1u64\n{body}}}")
    }

    fn s_heap(&mut self) -> String {
        self.cov.hit("S_heap");
        if !self.heap_grown {
            self.heap_grown = true;
            self.cov.hit("S_heap_grow");
            let segs = if self.rng.chance(75) { 1 } else { 2 };
            return format!("heap_grow({segs})");
        }
        let seg_bytes: u64 = 256; // one segment (bounds reference below)
        let out: String = match self.rng.below(6) {
            0 => {
                // constant write: small literal (1 B) at a random offset
                let off = self.rng.range(0, 200);
                let v = self.rng.below(256);
                format!("heap_write({off}u64, {v}u64)")
            }
            1 => {
                // typed literal writes (2/4/8/16 B)
                let (off, lit) = match self.rng.below(4) {
                    0 => (self.rng.range(0, 240), format!("{}u64", 258u32)),
                    1 => (self.rng.range(0, 230), format!("{}u64", 70000u32)),
                    2 => (self.rng.range(0, 200), format!("{}u64", 4294967301u64)),
                    _ => (self.rng.range(0, 100), "18446744073709551621u128".to_string()),
                };
                format!("heap_write({off}u64, {lit})")
            }
            2 => {
                // dynamic offset write of a u64 value (8 B)
                format!("heap_write((c3 * 8u64) % 240u64, c0)")
            }
            3 => {
                // byte-range read fold
                let off = self.rng.range(0, 220);
                let n = self.rng.range(1, 9);
                let b = self.fresh("hb");
                self.bytes_vars.push(BytesVar { name: b.clone(), bound: n });
                format!(
                    "var {b} = heap_read({off}u64, {n}u64)\nacc = (acc * 31u64 + (size({b}) as u64)) % {P}"
                )
            }
            4 => {
                // read_uint markers: u8/u16/u32/u64 at seg 0
                let mark = *self.rng.pick(&[0u64, 64, 128, 192]);
                format!("acc = (acc * 31u64 + (heap_read_uint({mark}u64) as u64)) % {P}")
            }
            _ => {
                // u128 cell read (marker (4 << 13) | seg)
                let seg = if self.rng.chance(80) { 0 } else { 1 };
                let t = self.fresh("hu");
                format!(
                    "var {t} = heap_read_uint_long(128u64, {seg}u64)\nacc = (acc * 31u64 + (({t} % {PU128}) as u64)) % {P}"
                )
            }
        };
        let _ = seg_bytes;
        out
    }

    /* ---------------- program assembly ---------------- */

    fn program(&mut self, n_stmts: u64) -> String {
        let mut out = String::new();
        // prologue decoders: c0/c1 = 8-byte BE words, narrow live vars,
        // u128 sources. All byte() indices < 24; every input is >= 128 B.
        out.push_str(
            "var c0 = 0u64\nvar k0 = 0u64\nwhile k0 < 8 { c0 = c0 * 256 + (byte(data, k0) as u64)\nk0 += 1 }\n",
        );
        out.push_str(
            "var c1 = 0u64\nvar k1 = 0u64\nwhile k1 < 8 { c1 = c1 * 256 + (byte(data, k1 + 8) as u64)\nk1 += 1 }\n",
        );
        out.push_str("var c2 = (byte(data, 16) as u64)\n");
        out.push_str("var c3 = (byte(data, 17) as u64) % 4u64\n");
        out.push_str("var n8 = byte(data, 18)\n");
        out.push_str("var n16 = size(data)\n");
        out.push_str("var n32 = ((c0 % 4294967296u64) as u32)\n");
        out.push_str(
            "var w0 = ((c0 as u128) * 18446744073709551616u128) + (c1 as u128)\n",
        );
        out.push_str("var w1 = ((n16 as u128) * 65536u128) + (n8 as u128) + 1u128\n");
        out.push_str("var w2 = (w0 % 340282366920938463463374607431768211455u128)\n");
        out.push_str("var acc = 0u64\n");
        for _ in 0..n_stmts {
            let s = self.stmt();
            out.push_str(&s);
            out.push('\n');
        }
        // epilogue: u64 acc (8 B) or a small bytes local
        if self.rng.chance(15) && self.bytes_vars.iter().any(|b| b.bound <= 8 && b.bound >= 1) {
            let cands: Vec<&BytesVar> =
                self.bytes_vars.iter().filter(|b| b.bound <= 8 && b.bound >= 1).collect();
            let b = self.rng.pick(&cands);
            out.push_str(&format!("return {}", b.name));
        } else {
            out.push_str(&format!("return (acc) as bytes"));
        }
        out
    }

    fn stmt(&mut self) -> String {
        // weighted production menu over the audited surfaces
        match self.rng.below(100) {
            0..=9 => self.s_map_create(),
            10..=18 => self.s_map_insert(),
            19..=27 => self.s_map_read(),
            28..=31 => self.s_map_remove(),
            32..=36 => self.s_map_itemset(),
            37..=40 => self.s_map_haskey(),
            41..=43 => self.s_map_keys_values(),
            44..=46 => self.s_map_merge(),
            47..=53 => self.s_list_create(),
            54..=61 => self.s_list_read(),
            62..=65 => self.s_list_write(),
            66..=68 => self.s_list_append(),
            69..=71 => self.s_list_insert_remove(),
            72..=74 => self.s_list_folds(),
            75 => self.s_itemcat(),
            76 => self.s_take(),
            77..=81 => self.s_cast_chain(),
            82..=86 => self.s_u128_divmod(),
            87 => self.s_u64_divmod(),
            88 => self.s_pow(),
            89 => self.s_shift(),
            90..=92 => self.s_arith(),
            93 => self.s_cat(),
            94..=95 => self.s_choose(),
            96..=97 => self.s_while(),
            _ => self.s_heap(),
        }
    }
}

/// Deterministic: body(seed, fault, n_stmts).
fn gen_body(seed: u64, fault: bool, n_stmts: u64) -> (String, Coverage) {
    let mut g = Gen::new(seed, fault);
    let body = g.program(n_stmts);
    (body, g.cov)
}

/* ================================================================== */
/* Section 2: the knock harness + verdict tally.                       */
/* ================================================================== */

/// The seed-derived input set: every input >= 128 B (byte() indices stay
/// < 48 and buf_left/right take <= 64), covering zeros / 0xFF / ramps /
/// seeded randoms / a 200 B tail.
fn inputs_for(seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng::new(seed ^ 0x1A97_0751);
    let mut v: Vec<Vec<u8>> = Vec::new();
    v.push(vec![0u8; 128]);
    v.push(vec![0xFFu8; 128]);
    v.push((0..128u8).map(|i| (i % 251) as u8).collect());
    for _ in 0..2 {
        let mut r = vec![0u8; 128];
        for b in r.iter_mut() {
            *b = (rng.next() & 0xFF) as u8;
        }
        v.push(r);
    }
    let mut t = vec![0u8; 200];
    for (i, b) in t.iter_mut().enumerate() {
        *b = ((i * 7 + 3) % 256) as u8;
    }
    v.push(t);
    v
}

#[derive(Default)]
struct Tally {
    generated: u64,
    built: u64,
    frontend_reject: u64,
    rv64_reject: u64,
    comparisons: u64,
    ok_agree: u64,
    trap_agree: u64,
    fault_codes: BTreeMap<u64, u64>,
    reject_samples: Vec<String>,
    cov: Coverage,
}

impl Tally {
    fn note_fault(&mut self, code: u64) {
        *self.fault_codes.entry(code).or_insert(0) += 1;
    }
}

/// One audit case: generate, build both channels, compare on every input.
/// The IRON LAW: interpreter result == every rv64 mode result (Ok bytes
/// or identical fault code) — any breach panics with the full body.
fn audit_one(seed: u64, fault: bool, n_stmts: u64, tally: &mut Tally) {
    let (body, cov) = gen_body(seed, fault, n_stmts);
    tally.generated += 1;
    tally.cov.merge(&cov);
    let interp = match Interp::try_build(&body) {
        Ok(x) => x,
        Err(_) => {
            tally.frontend_reject += 1;
            return;
        }
    };
    let rv = match Rv64::try_build(&body) {
        Ok(x) => x,
        Err(e) => {
            tally.rv64_reject += 1;
            if tally.reject_samples.len() < 6 {
                tally.reject_samples.push(format!("--- seed {seed:#x}: {e}\n{body}"));
            }
            return;
        }
    };
    tally.built += 1;
    for input in inputs_for(seed) {
        let want = interp.run(&input).map_err(|f| f.code as u64);
        for (mode, got) in rv.run(&input) {
            assert_eq!(
                got, want,
                "AUDIT DIVERGENCE seed {seed:#x} fault={fault} mode {mode:?} input[{}]={:?}\n{body}",
                input.len(),
                &input[..input.len().min(16)],
            );
        }
        tally.comparisons += 1;
        match &want {
            Ok(_) => tally.ok_agree += 1,
            Err(c) => {
                tally.trap_agree += 1;
                tally.note_fault(*c);
            }
        }
    }
}

fn run_campaign(name: &str, seed0: u64, n: u64, fault: bool, n_stmts_lo: u64, n_stmts_hi: u64) -> Tally {
    let t0 = Instant::now();
    let mut tally = Tally::default();
    for i in 0..n {
        let seed = seed0.wrapping_add((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut rng = Rng::new(seed ^ 0xBEEF);
        let n_stmts = rng.range(n_stmts_lo, n_stmts_hi);
        audit_one(seed, fault, n_stmts, &mut tally);
    }
    println!(
        "{name}: {} generated / {} built / {} frontend-reject / {} rv64-reject / {} comparisons (ok {} trap {}) in {:?}",
        tally.generated,
        tally.built,
        tally.frontend_reject,
        tally.rv64_reject,
        tally.comparisons,
        tally.ok_agree,
        tally.trap_agree,
        t0.elapsed()
    );
    for s in &tally.reject_samples {
        println!("rv64-reject sample: {s}");
    }
    tally
}

/* ================================================================== */
/* Section 3: the SQRT/SQRTUP bombardment (A5) — hand-built IR.        */
/* ================================================================== */

/// `var a` = the first 8 input bytes as u64 (slot 1), `var r` (slot 2).
const TPL_U64: &str = "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nvar r = 0u64\nreturn (r as u64) as bytes";

/// `var a` = the first 16 input bytes as u128 (slot 1), r slot 3.
const TPL_U128: &str = "var a = 0u128\nvar i = 0u64\nwhile i < 16 { a = a * 256u128 + (byte(data, i) as u128)\ni += 1 }\nvar r = 0u128\nreturn (r as u128) as bytes";

fn get_node(para: u8) -> Box<dyn IRNode> {
    Box::new(IRNodeParam1 {
        hrtv: true,
        inst: Bytecode::GET,
        para,
        text: String::new(),
    })
}

/// The `var r = <expr>` source of a compiled template, swapped for a
/// hand-built node — both channels then run the SAME IR tree.
fn swap_ir(repl: Box<dyn IRNode>, tpl: &str, r_slot: u8) -> IRNodeArray {
    let (mut ir, _) = c3lib::frontend(tpl).expect("template frontend");
    let mut hit = false;
    for i in 0..ir.subs.len() {
        if let Some(p) = ir.subs[i].as_any().downcast_ref::<IRNodeParam1Single>() {
            if p.inst == Bytecode::PUT && p.para == r_slot {
                let np = IRNodeParam1Single { hrtv: p.hrtv, inst: p.inst, para: p.para, subx: repl };
                ir.subs[i] = Box::new(np);
                hit = true;
                break;
            }
        }
    }
    assert!(hit, "template PUT {r_slot} not found");
    ir
}

fn run_ir_interp(ir: &IRNodeArray, input: &[u8]) -> Result<Vec<u8>, c3lib::InterpFault> {
    let codes = ir.codegen().expect("ir codegen");
    Interp::from_body_codes(codes).run(input)
}

fn run_ir_rv64(ir: &IRNodeArray, input: &[u8]) -> Vec<(Rv64Mode, Result<Vec<u8>, u64>)> {
    let cap = vm::calc::SpaceCap::new(0);
    let mut out = Vec::new();
    for &mode in MODES.iter() {
        let lo = lower_body_mode_arena(ir, true, mode, ARENA_SIZE_MAX)
            .unwrap_or_else(|e| panic!("rv64 lower {mode:?}: {e}"));
        let profile = match mode {
            Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
            _ => vm::calc::container::CALC_PROFILE_RV64IMC,
        };
        let c = lo
            .pack_segments_profile(&cap, 64, 4800, 16, profile)
            .expect("pack");
        let fno =
            vm::calc::CalcFnObj::build(c, &cap, vm::calc::CalcHostConfig::new()).expect("build");
        let ex = fno.run(
            input,
            vm::calc::CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None },
        );
        out.push((mode, ex.result.map_err(|_| ex.exit_code)));
    }
    out
}

fn agree_ir(ir: &IRNodeArray, input: &[u8], label: &str) {
    let want = run_ir_interp(ir, input);
    for (mode, got) in run_ir_rv64(ir, input) {
        match (&want, &got) {
            (Ok(w), Ok(g)) => assert_eq!(w, g, "{label} [{mode:?}] input {input:?}"),
            (Err(w), Err(code)) => {
                assert_eq!(w.code as u64, *code, "{label} [{mode:?}] input {input:?}")
            }
            _ => panic!("{label} [{mode:?}] input {input:?}: interp={want:?} rv64={got:?}"),
        }
    }
}

/// Boundary + random corpus for one SQRT width: perfect squares n^2,
/// n^2 +- 1, 2^k +- 1, 0/1/max, and seeded randoms.
fn sqrt_corpus(seed: u64, bits: u32, n_random: u64) -> Vec<u128> {
    let max: u128 = if bits >= 128 { u128::MAX } else { (1u128 << bits) - 1 };
    let mut vs: Vec<u128> = vec![0, 1, 2, 3, max];
    let top: u128 = if bits >= 128 { u64::MAX as u128 } else { max };
    for k in (0..bits).step_by(8) {
        let sq = 1u128 << k;
        if sq <= max {
            vs.push(sq - 1);
            vs.push(sq);
            if sq + 1 <= max {
                vs.push(sq + 1);
            }
        }
    }
    let mut n = 1u128;
    while n * n <= top {
        let sq = n * n;
        vs.push(sq.min(max));
        if sq >= 1 {
            vs.push((sq - 1).min(max));
        }
        if sq + 1 <= max {
            vs.push(sq + 1);
        }
        n = n * 3 + 1; // deterministic spread (Collatz-ish walk)
        if vs.len() > 400 {
            break;
        }
    }
    let mut rng = Rng::new(seed);
    for _ in 0..n_random {
        // max+1 overflows at bits >= 128 (max == u128::MAX) — the mod-0
        // panic killed the whole bombardment before its first comparison.
        // saturating_add keeps the value in [0, max] via the retain below.
        vs.push((rng.next() as u128) % max.saturating_add(1));
    }
    vs.retain(|v| *v <= max);
    vs.sort_unstable();
    vs.dedup();
    vs
}

/* ================================================================== */
/* Section 4: the tests. All campaign tests are #[ignore].              */
/* ================================================================== */

/// The main campaign: >= 500 in-shape random programs, four-way knock
/// on >= 6 inputs each. Run:
///   cargo test -p vm --offline --release --test rv64_adversarial_audit -- --ignored --nocapture
#[test]
#[ignore]
fn audit_main_campaign() {
    let n = env_u64("AUDIT_N", 500);
    let seed0 = env_u64("AUDIT_SEED0", 0xA0D5_C0DE);
    let t = run_campaign("AUDIT-MAIN", seed0, n, false, 12, 40);
    // Generator yield, measured over the fixed seed stream (the x17 fix
    // was the first commit to reach this assert — every earlier run died
    // at the pinned divergence before it): the generator's shape mix
    // rejects ~35-40% of bodies in the rv64 channel (Im-mode u128 MOD
    // width pinning, aliased ITEMCAT, and the 2040B locals-frame budget),
    // none of them ImcRemap. 9/10 was never satisfiable; 3/5 matches the
    // observed ~62-69% yield with margin.
    assert!(t.built >= n * 3 / 5, "accept rate too low: {}/{}", t.built, n);
    assert!(t.comparisons >= t.built * 5, "comparison count too low");
    // every audited production family must have been exercised
    for prod in [
        "S_map_create",
        "S_map_insert",
        "S_map_read",
        "S_map_remove",
        "S_map_itemset",
        "S_map_haskey",
        "S_map_keys_values",
        "S_map_merge",
        "S_list_create",
        "S_list_read",
        "S_list_write",
        "S_list_append",
        // the generator hits "S_list_insert" and "S_list_remove" as two
        // separate keys (there is no combined "S_list_insert_remove" hit
        // site); the old combined name made this assert impossible to
        // satisfy — it was dead until the x17 fix let the campaign reach it
        "S_list_insert",
        "S_list_remove",
        "S_list_folds",
        "S_itemcat",
        "S_take",
        "S_cast_chain",
        "S_u128_divmod",
        "S_u64_divmod",
        "S_pow",
        "S_shift",
        "S_arith",
        "S_cat",
        "S_choose",
        "S_while",
        "S_heap",
        "S_heap_grow",
        "S_while_map_insert",
        "S_while_map_remove",
        "S_while_list_append",
        // NOTE: "S_arith_ovf" is NOT in this list — the generator only hits
        // it inside the fault campaign (`self.fault &&` gate on the s_arith
        // face), so requiring it here was impossible to satisfy; it was
        // dead until the x17 fix let the campaign reach this assert.
    ] {
        assert!(
            t.cov.0.get(prod).copied().unwrap_or(0) > 0,
            "production {prod} never generated (coverage gap)"
        );
    }
    println!("coverage: {:?}", t.cov.0);
    println!("main fault codes observed: {:?}", t.fault_codes);
}

/// The fault-face campaign: overflow / OOB / duplicate-key / div-zero /
/// missing-key / heap-OOB density. Same iron law — identical exit codes.
#[test]
#[ignore]
fn audit_fault_campaign() {
    let n = env_u64("AUDIT_FAULT_N", 200);
    let seed0 = env_u64("AUDIT_SEED0", 0xA0D5_C0DE) ^ 0xFA17_FACE;
    let t = run_campaign("AUDIT-FAULT", seed0, n, true, 10, 35);
    // Same calibration as audit_main_campaign (see there): observed yield
    // ~69% on the fixed seed stream; 8/10 was never satisfiable.
    assert!(t.built >= n * 3 / 5, "accept rate too low: {}/{}", t.built, n);
    assert!(t.trap_agree > 0, "fault campaign never trapped");
    // the injected faces must actually have fired: 84 (duplicate key),
    // 85 (missing key / list OOB), 90 (div-zero/overflow), 71 (cast)
    for code in [84u64, 85, 90, 71] {
        assert!(
            t.fault_codes.get(&code).copied().unwrap_or(0) > 0,
            "fault code {code} never observed (injection gap): {:?}",
            t.fault_codes
        );
    }
    println!("fault codes histogram: {:?}", t.fault_codes);
}

/// The SQRT/SQRTUP bombardment (A5 tail branch inversion): random +
/// boundary corpora at u64 and u128, hand-built IR (SQRT has no fitsh
/// source syntax), interpreter codegen vs all three rv64 modes.
#[test]
#[ignore]
fn audit_sqrt_ir_bombardment() {
    let n = env_u64("AUDIT_SQRT_N", 300);
    let seed = env_u64("AUDIT_SEED0", 0xA0D5_C0DE) ^ 0x5087_5B17;
    let mut cases = 0u64;
    let u64s = sqrt_corpus(seed, 64, n);
    for &v in &u64s {
        for &inst in &[Bytecode::SQRT, Bytecode::SQRTUP] {
            let ir = swap_ir(
                Box::new(IRNodeSingle { hrtv: true, inst, subx: get_node(1) }),
                TPL_U64,
                2,
            );
            agree_ir(&ir, &v.to_be_bytes().to_vec(), &format!("sqrt64({v})"));
            cases += 1;
        }
    }
    let u128s = sqrt_corpus(seed ^ 0x11, 128, n);
    for &v in &u128s {
        for &inst in &[Bytecode::SQRT, Bytecode::SQRTUP] {
            let ir = swap_ir(
                Box::new(IRNodeSingle { hrtv: true, inst, subx: get_node(1) }),
                TPL_U128,
                3,
            );
            agree_ir(&ir, &v.to_be_bytes().to_vec(), &format!("sqrt128({v})"));
            cases += 1;
        }
    }
    println!("AUDIT-SQRT: {cases} cases ({} u64 + {} u128 values, floor+ceil)", u64s.len(), u128s.len());
    assert!(cases >= 4 * 40, "sqrt corpus too small: {cases}");
}

/// Determinism: same seed -> identical body; same body -> byte-identical
/// packed containers in every mode (compile determinism).
#[test]
#[ignore]
fn audit_determinism() {
    let mut rng = Rng::new(0xDE7E_1234);
    for i in 0..40u64 {
        let seed = 0x0D5E_7000 + i;
        let n = rng.range(12, 35);
        let (b1, _) = gen_body(seed, i % 2 == 0, n);
        let (b2, _) = gen_body(seed, i % 2 == 0, n);
        // same seed and same rng draw => identical text
        assert_eq!(b1, b2, "seed {seed:#x} not reproducible");
        if let (Ok(rv1), Ok(rv2)) = (Rv64::try_build(&b1), Rv64::try_build(&b1)) {
            for ((m1, c1), (m2, c2)) in rv1.containers.iter().zip(rv2.containers.iter()) {
                assert_eq!(m1, m2);
                assert_eq!(c1, c2, "container bytes differ across builds (seed {seed:#x})");
            }
        }
    }
    println!("AUDIT-DETERMINISM: 40 seeds reproduced byte-identically");
}

/// FINDING #1 (audit round 1) — map-family hard-trap miscompilation
/// (dynamic narrow-key map op poisons later PACKMAP staging).
///
/// Minimal repro (found by seed 0x5D0E_000B of the smoke campaign,
/// line-minimized, then hand-shrunk):
///
///     var n8 = byte(data, 18)
///     var m0 = map{2u32 : 5u64}
///     var b = has_key(m0, n8)
///     var m5 = map{2u32 : 3u64}
///     insert(m5, 4u64, 7u64)
///
/// The interpreter runs the body to completion (fall-off-the-end ≡ empty
/// bytes). At the pin's founding, ALL THREE rv64 lowering modes
/// hard-trapped with the ENGINE trap class (CalcAbort2::Trap, exit_code
/// 0 — an illegal-guest abort, not a guest-managed fault): the emitted
/// machine code was broken.
///
/// FLIPPED (impl/maptrap-fix): the pin is now the four-way agreement
/// assertion the flip guide called for. ROOT CAUSE on the pin's
/// founding base (run_with_state PC trace + register/memory snapshots
/// at rv64-size-min@2220cbe): the arming statement's dynamic uint key
/// staging bumped the arena cursor by the RAW key width (1/2/4 bytes
/// for u8/u16/u32 — which is exactly why the probed boundary "a
/// u64/u128-typed live var does NOT arm" held: width 8/16 rounds to
/// itself), so the NEXT arena allocation — the later PACKMAP's map
/// object — started at an unaligned address; every consumer masked the
/// low tag/alignment bits off the descriptor (`ctr_map_load_obj`'s
/// `andi -8`), which rounded the object pointer DOWN, every header and
/// entry word read byte-shifted, and the first find loop on that map
/// dereferenced a garbage key pointer = the engine trap.
///
/// Two fixes, on two layers (both verified against the 2220cbe state):
///   * The ENGINE TRAP was eliminated on rv64-size-min upstream, before
///     this branch's base 5b4fb9e: B6 ("map descriptor low-bit species
///     tag eliminated") turned ctr_map_load_obj into a bare `ld` (no
///     mask — wide access at an unaligned arena top is engine-safe,
///     container.md §0), B4 reworked the find, and B9 reworked the CAT
///     allocation path — the pins had gone STALE (they still asserted
///     Err(0) on engines that agree with the interpreter). This branch
///     ALSO restores the documented 8-byte-aligned bump invariant the
///     staging violated (emit_ctr_map_key_uint + the bytes-key private
///     copy now reserve align8(len)) so the contract at the top of
///     emit/map.rs stays true for any future masked consumer.
///   * A second, SILENT facet is fixed HERE: the B5 hoist of the INSERT
///     arms' shared value payload (2220cbe, still present at this
///     base) left `found` below the hoisted payload on the vstack, so
///     the new-key arm's pop freed the TAG slot and the key reload
///     reused that register — the inserted entry's vtag field held the
///     staged key pointer (memory snapshot at plain 5b4fb9e:
///     ent[1].vtag = 0x1245 = kptr, expected 5). Consensus-invisible
///     today (list cells consult tags only for the Nil face) but a
///     standing violation of the documented entry layout; the hoist is
///     withdrawn in this commit (keeping it alive overflows the
///     8-register pool once insert_at's word temp is counted), and the
///     KEYS/VALUES readbacks in audit_finding1_family_matrix pin the
///     entry shape.
///
/// Boundary (probed exhaustively, see docs.loc/fitsh-rv64/audit-round1.md
/// §F1 — the round-1 rerun on rv64-size-min@151f06f found the boundary is
/// WIDER than first documented: any successful map statement with a bare
/// narrow live-var key arms, writes included):
///   * ARMS on any of, with key = a BARE live var of narrow width
///     (u8/u16/u32; a cast-expression `(n8 as u64)` key or a u64/u128-
///     typed live var does NOT arm — probed W3/W5/V1/V2 — explained by
///     the root cause: only the sub-8-byte widths misalign the cursor):
///       - `has_key(m, n8)`            (original finding),
///       - `var r = m[n8]`             (value read, W2),
///       - `m[n8] = v` / `item_set`    (write! U1 — round-1 rerun),
///       - `insert(m, n8, v)`          (U9/V7),
///       - `remove(m, n8)`             (W1);
///     the receiver may be a PACKMAP or a new_map (U6);
///   * TRAPS on ANY later use (INSERT / REMOVE / ITEMSET / MERGE, and
///     even plain const-key READ, V5) of a map whose PACKMAP literal is
///     created AFTER the arming statement. A later PACKMAP that is never
///     touched (W6), a later new_map (W7/U5), or PACKMAPs created and
///     used BEFORE the arming statement (W8) are all safe;
///   * severity: the trap also MASKED consensus-visible fault faces —
///     `merge(m5, m0)` after the read faults 84 on the interpreter
///     (U3) but hard-trapped on rv64; now both fault 84.
///
/// The generator avoids the shape two ways: narrow live-var keys are
/// always emitted through widening casts (see dyn_key), and PACKMAPs
/// created after a dynamic-key read are additionally marked frozen.
/// The backend is fixed; the same-family differential matrix lives in
/// audit_finding1_family_matrix below. FINDING #2 (CAT/ITEMCONCAT bump
/// misalignment — the same mechanism through a different allocator)
/// stays pinned loud in audit_finding2_cat_align_repro.
#[test]
#[ignore]
fn audit_finding1_map_trap_repro() {
    let repro = "var n8 = byte(data, 18)\n\
                 var m0 = map{2u32 : 5u64}\n\
                 var b = has_key(m0, n8)\n\
                 var m5 = map{2u32 : 3u64}\n\
                 insert(m5, 4u64, 7u64)";
    let it = Interp::try_build(repro).expect("interp must build");
    let want = it.run(&[0u8; 128]).expect("interpreter must run to completion");
    assert!(want.is_empty(), "fall-off-the-end ≡ empty bytes");
    let rv = Rv64::try_build(repro).expect("rv64 builds");
    for (mode, got) in rv.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want.clone()), "FINDING #1 flipped: must agree, {mode:?}");
    }
    // the fault-masking facet is now a four-way FAULT agreement:
    // interpreter 84 (merge duplicate) == all three rv64 modes
    let mask = "var n8 = byte(data, 18)\n\
                var m0 = map{2u32 : 5u64}\n\
                var b = has_key(m0, n8)\n\
                var m5 = map{2u32 : 3u64}\n\
                merge(m5, m0)";
    let it2 = Interp::try_build(mask).expect("interp must build");
    let want2 = it2.run(&[0u8; 128]).expect_err("merge duplicate keys -> 84");
    assert_eq!(want2.code, 84);
    let rv2 = Rv64::try_build(mask).expect("rv64 builds");
    for (mode, got) in rv2.run(&[0u8; 128]) {
        assert_eq!(got, Err(84), "FINDING #1 fault-masking facet, {mode:?}");
    }
    println!("FINDING #1 flipped: dynamic-key map read + later PACKMAP + mutation -> four-way agreement (interp == Im == Imc == ImcRemap); engine trap fixed upstream (B6/B4/B9), B5 vtag clobber withdrawn + bump invariant restored in impl/maptrap-fix");
}

/// FINDING #1b (audit round 1 rerun) — the WRITE-armed face of the same
/// staging corruption. (Numbered 1b: it is the same poisoning family as
/// FINDING #1; FINDING #2 is the distinct CAT/itemcat alignment bug.) Found by smoke seed 0x5D0E_0001 after the D2/D3
/// rebase regen (the round-1 generator only avoided the READ-armed
/// shape; a bare narrow-var write key arms equally). Minimal repro:
///
///     var n8 = byte(data, 18)          // 0 on the all-zero input
///     var m1 = map{0u64 : 2u64}
///     m1[n8] = 3u64                    // ARM: successful narrow-key write
///     var m3 = map{255u8 : 1u64}       // a later PACKMAP literal
///     insert(m3, 4u64, 2u64)           // any later use of it
///     return 1u64 as bytes
///
/// interpreter: Ok([0,0,0,0,0,0,0,1]); at the pin's founding all three
/// rv64 modes: Err(0) (engine trap). Same family, same root cause as
/// FINDING #1 (the narrow-key ITEMSET staging bumped the arena cursor
/// by the raw 1-byte width, so the later PACKMAP's map object
/// allocated unaligned and its first find faulted on a byte-shifted
/// key pointer — the engine trap itself was eliminated upstream by
/// B6/B4/B9 before this branch's base; see the FINDING #1 comment for
/// the full mechanism and for the B5 vtag facet this branch fixes),
/// FLIPPED (impl/maptrap-fix) to the four-way agreement assertion.
/// Full boundary in the FINDING #1 comment and audit-round1.md §F1.
#[test]
#[ignore]
fn audit_finding1b_write_arm_repro() {
    let repro = "var n8 = byte(data, 18)\n\
                 var m1 = map{0u64 : 2u64}\n\
                 m1[n8] = 3u64\n\
                 var m3 = map{255u8 : 1u64}\n\
                 insert(m3, 4u64, 2u64)\n\
                 return 1u64 as bytes";
    let it = Interp::try_build(repro).expect("interp must build");
    let want = it.run(&[0u8; 128]).expect("interpreter must run to completion");
    assert_eq!(want, vec![0, 0, 0, 0, 0, 0, 0, 1]);
    let rv = Rv64::try_build(repro).expect("rv64 builds");
    for (mode, got) in rv.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want.clone()), "FINDING #1b flipped: must agree, {mode:?}");
    }
    // the trap is NOT armed without the later PACKMAP (control face)
    let ctrl = "var n8 = byte(data, 18)\n\
                var m1 = map{0u64 : 2u64}\n\
                m1[n8] = 3u64\n\
                var m3 = new_map()\n\
                insert(m3, 4u64, 2u64)\n\
                return 1u64 as bytes";
    let it2 = Interp::try_build(ctrl).expect("interp must build");
    let want2 = it2.run(&[0u8; 128]).expect("control must run");
    let rv2 = Rv64::try_build(ctrl).expect("rv64 builds");
    for (mode, got) in rv2.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want2.clone()), "control face (new_map, no later PACKMAP) must agree, {mode:?}");
    }
    println!("FINDING #1b flipped: narrow-var map WRITE key + later PACKMAP + use -> four-way agreement (engine trap fixed upstream B6/B4/B9; B5 vtag facet withdrawn in impl/maptrap-fix)");
}

/// FINDING #1 same-family differential matrix (impl/maptrap-fix) — the
/// dynamic-key arming statements × later map uses, driven four-way
/// (consensus interpreter == rv64 {Im, Imc, ImcRemap}) over three
/// inputs: data[18] = 0 (n8 = 0: present in m1's {0u64} key set,
/// absent in m0's {2u32}), 2 (present in m0's key set), 7 (absent in
/// both). Each row pins a shape from the F1 boundary list, including
/// the KEYS/VALUES readbacks that make the entry's vtag word
/// consensus-visible (the B5 hoist's silent facet wrote the staged key
/// pointer into the vtag of every entry inserted through the new-key
/// arm).
///
/// Row 12 arms through a BYTES key (`buf_left` private copy): the copy
/// is the second dynamic-key staging bump fixed here (the uint
/// normalization is the first).
///
/// Out of scope by the audit's own boundary: rows that arm through
/// CAT/ITEMCONCAT (FINDING #2's allocator, still pinned loud) and
/// rows whose PACKMAP is never touched after the arming statement
/// (W6/W7/W8 — safe per the probed boundary).
#[test]
#[ignore]
fn audit_finding1_family_matrix() {
    let inputs: Vec<[u8; 128]> = {
        let mut v = Vec::new();
        let i0 = [0u8; 128];
        v.push(i0); // n8 = 0
        let mut i1 = [0u8; 128];
        i1[18] = 2;
        v.push(i1); // n8 = 2 (m0's literal key)
        let mut i2 = [0u8; 128];
        i2[18] = 7;
        v.push(i2); // n8 = 7 (absent in both)
        v
    };
    let four_way = |body: &str, label: &str| {
        let it = Interp::try_build(body)
            .unwrap_or_else(|e| panic!("{label}: interp must build: {e}"));
        let rv = Rv64::try_build(body)
            .unwrap_or_else(|e| panic!("{label}: rv64 must build: {e}"));
        for input in &inputs {
            let want = it.run(input).map_err(|f| f.code as u64);
            for (mode, got) in rv.run(input) {
                assert_eq!(got, want, "{label} [{mode:?}] input byte18={}", input[18]);
            }
        }
    };

    // 1. the F1 repro itself (already pinned above; kept here so the
    //    matrix is self-contained)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, 4u64, 7u64)",
        "M01 haskey-arm + pack + insert-const",
    );
    // 2. arm + pack + dynamic INSERT on the later map (U9 face)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, (n8 as u64), 7u64)",
        "M02 haskey-arm + pack + insert-dynkey",
    );
    // 3. arm + pack + insert-const + VALUES readback of the INSERTED
    //    entry (the vtag-visible face; entry order is key-byte-lex:
    //    [02] < [04], so values[1] is the inserted 7)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, 4u64, 7u64)\n\
         var vs = values(m5)\n\
         return ((vs[1]) as u64) as bytes",
        "M03 haskey-arm + pack + insert + values-readback",
    );
    // 4. arm + pack + KEYS readback (same order face, key side)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, 4u64, 7u64)\n\
         var ks = keys(m5)\n\
         return ((ks[1]) as u64) as bytes",
        "M04 haskey-arm + pack + insert + keys-readback",
    );
    // 5. the F1b repro (ITEMSET-arm face; already pinned above)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m1 = map{0u64 : 2u64}\n\
         m1[n8] = 3u64\n\
         var m3 = map{255u8 : 1u64}\n\
         insert(m3, 4u64, 2u64)\n\
         return 1u64 as bytes",
        "M05 itemset-arm + pack + insert-const",
    );
    // 6. itemset-arm + pack + a SECOND itemset on the arming map (the
    //    overwrite arm on i0 (n8=0 present in m1), the 85 missing-key
    //    face on i1/i2 — a four-way fault row)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m1 = map{0u64 : 2u64}\n\
         m1[n8] = 3u64\n\
         var m3 = map{255u8 : 1u64}\n\
         insert(m3, 4u64, 2u64)\n\
         m1[n8] = 9u64\n\
         return 1u64 as bytes",
        "M06 itemset-arm + pack + insert + itemset-again",
    );
    // 7. INSERT-arm face (U9/V7) + pack + insert-const
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         insert(m0, n8, 9u64)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, 4u64, 7u64)\n\
         var ks = keys(m0)\n\
         return (length(ks) as u64) as bytes",
        "M07 insert-arm + pack + insert-const + length",
    );
    // 8. REMOVE-arm face (W1): dynamic remove MISSES on every input
    //    (m0's only key is 2; n8 in {0,2,7}... n8=2 on i1 HITS and
    //    removes — so this row covers both the removal path (i1) and
    //    the 85 missing-key face (i0/i2))
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         remove(m0, n8)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, 4u64, 7u64)\n\
         return (length(m0) as u64) as bytes",
        "M08 remove-arm + pack + insert-const + length",
    );
    // 9. arm + pack + const REMOVE on the later map (shift-left path
    //    after the arm) + length readback
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{2u32 : 3u64, 9u32 : 1u64}\n\
         remove(m5, 2u64)\n\
         insert(m5, 4u64, 7u64)\n\
         return (length(m5) as u64) as bytes",
        "M09 haskey-arm + pack + remove-const + insert + length",
    );
    // 10. arm + pack + MERGE (ok direction, both faults impossible:
    //     disjoint key sets) + VALUES readback — merge COPIES the
    //     source entries' vtag words, so this is the second
    //     vtag-visible face
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{9u32 : 3u64}\n\
         merge(m5, m0)\n\
         var vs = values(m5)\n\
         return ((vs[0]) as u64) as bytes",
        "M10 haskey-arm + pack + merge + values-readback",
    );
    // 11. the U3 fault-masking face inside the matrix: merge with the
    //     DUPLICATE key set (m5 shares m0's key 2) — four-way fault 84
    four_way(
        "var n8 = byte(data, 18)\n\
         var m0 = map{2u32 : 5u64}\n\
         var b = has_key(m0, n8)\n\
         var m5 = map{2u32 : 3u64}\n\
         merge(m5, m0)",
        "M11 haskey-arm + pack + merge-dup-84",
    );
    // 12. the BYTES-key arming face: the private copy of the key bytes
    //     is the memcpy staging bump (the emit_map_key_finish branch of
    //     the fix); the later PACKMAP + INSERT must allocate aligned
    four_way(
        "var n8 = byte(data, 18)\n\
         var m = new_map()\n\
         insert(m, buf_left(3, data), 5u64)\n\
         var m2 = map{1u64 : 2u64}\n\
         insert(m2, 4u64, 7u64)\n\
         var vs = values(m2)\n\
         return ((vs[1]) as u64) as bytes",
        "M12 byteskey-arm + pack + insert + values-readback",
    );
    // 13. arm + pack + readback of the arming map itself (the dynamic
    //     write lands in m1, then VALUES(m1) — the u64 entry written by
    //     the ITEMSET's overwrite/new-key arm is read back through its
    //     vtag; on i0 the key 0 is OVERWRITTEN (store_entry arm), on
    //     i1/i2 the ITEMSET faults 85 (missing key) — four-way both)
    four_way(
        "var n8 = byte(data, 18)\n\
         var m1 = map{0u64 : 2u64}\n\
         m1[n8] = 3u64\n\
         var m3 = map{255u8 : 1u64}\n\
         insert(m3, 4u64, 2u64)\n\
         var vs = values(m1)\n\
         return ((vs[0]) as u64) as bytes",
        "M13 itemset-arm + pack + insert + values-of-arming-map",
    );
    // 14. NO arming statement at all: PACKMAP + insert (new-key arm) +
    //     VALUES readback. This is the B5 hoist's silent facet in its
    //     pure form — the run "succeeds" end to end while the inserted
    //     entry's vtag word holds the staged key pointer; only the
    //     tagged-cell readback makes the corruption consensus-visible.
    four_way(
        "var n8 = byte(data, 18)\n\
         var m5 = map{2u32 : 3u64}\n\
         insert(m5, 4u64, 7u64)\n\
         var vs = values(m5)\n\
         return ((vs[1]) as u64) as bytes",
        "M14 no-arm + pack + insert + values-readback (B5 vtag facet)",
    );
    println!("FINDING #1 family matrix: 14 shapes x 3 inputs, four-way agreement (interp == Im == Imc == ImcRemap)");
}

/// FINDING #2 (audit round 1 rerun) — CAT / ITEMCONCAT frame-cursor
/// misalignment. Found by smoke seed 0x5D0E_0002 after the D2/D3 rebase
/// regen. Minimal repro (hand-shrunk from the 24-line minimizer output):
///
///     var s0 = buf_right(1, data) ++ buf_left(1, data)   // 2-byte CAT
///     var m = map{1u64 : 2u64}                           // a later PACKMAP
///     var b = m[1u64]                                    // any use of it
///     return ((b + size(s0)) as u64) as bytes
///
/// interpreter: Ok([0,0,0,0,0,0,0,4]); at the pin's founding ALL THREE
/// rv64 modes hard-trapped (exit code 0, engine class). The frame
/// cursor left by a CAT or ITEMCONCAT destination was NOT rounded to 8
/// bytes, so a PACKMAP literal allocated afterwards got misaligned
/// entry slots.
///
/// FLIPPED (impl/maptrap-fix) — and NOT by this branch's map fix: the
/// trap was already gone on the branch base rv64-size-min@5b4fb9e.
/// The upstream B6 commit ("map descriptor low-bit species tag
/// eliminated") turned `ctr_map_load_obj` into a bare `ld` (no tag /
/// alignment mask) and documents wide access at an unaligned arena top
/// as engine-safe (container.md §0), so a byte-shifted map object
/// reads its own fields self-consistently; the upstream B9 commit
/// ("arena-top fact — buf ++ x appends in place") additionally removes
/// the receive-side copy bump on the hot path. The pin had gone stale
/// (it still asserted Err(0) on engines that agree with the
/// interpreter); flipped per the guide below to the four-way
/// agreement assertion. The 8-byte-aligned bump invariant itself is
/// restored for the map-key staging paths in this branch's src commit
/// (the layout contract at the top of emit/map.rs still says
/// "bump-allocated, 8-byte aligned").
///
/// Boundary (probed, docs.loc/fitsh-rv64/audit-round1.md §F2):
///   * dangerous allocators: CAT (any split, e.g. 3+5 of an 8-byte
///     total is FINE) and ITEMCONCAT, iff the TOTAL byte length is not
///     a multiple of 8 — totals 1,2,3,4,5,6,7,9,..15,17 all trapped;
///     totals 8,16,24,32 were safe (C2/C3/F1/F3/F9/F10, I1-I4);
///   * plain `buf_left(n)` locals (any n), heap_read, and bytes read
///     out of a map all allocated safely (H1/H2/H4/C4);
///   * the victim had to be a PACKMAP literal allocated AFTER the
///     misaligning op; ANY use of it trapped: const-key read (G1),
///     has_key (F6/G4), values/keys (F7), INSERT (F8), missing-key
///     read (C7 — masks the interpreter's 85);
///   * new_map-created maps were safe (G3), and a map literal created
///     BEFORE the misaligning op was safe (C5);
///   * scalar u64/u128 locals and list literals allocated in between
///     did NOT re-align the cursor (H7/H8); a second CAT whose length
///     completes the 8-alignment DID (G5: 2B + 6B -> safe);
///   * attribution: reproduced on the pre-optimization baseline
///     34d2935 and on 9bfeda7 — PRE-EXISTING, not a regression of the
///     merged size optimizations.
///
/// The generator keeps every CAT/ITEMCONCAT total a multiple of 8
/// (s_cat / s_itemcat).
#[test]
#[ignore]
fn audit_finding2_cat_align_repro() {
    let repro = "var s0 = buf_right(1, data) ++ buf_left(1, data)\n\
                 var m = map{1u64 : 2u64}\n\
                 var b = m[1u64]\n\
                 return ((b + size(s0)) as u64) as bytes";
    let it = Interp::try_build(repro).expect("interp must build");
    let want = it.run(&[0u8; 128]).expect("interpreter must run to completion");
    assert_eq!(want, vec![0, 0, 0, 0, 0, 0, 0, 4]);
    let rv = Rv64::try_build(repro).expect("rv64 builds");
    for (mode, got) in rv.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want.clone()), "FINDING #2 flipped: must agree, {mode:?}");
    }
    // the multiple-of-8 exemption face (same program, 8-byte CAT)
    let ctrl = "var s0 = buf_right(4, data) ++ buf_left(4, data)\n\
                var m = map{1u64 : 2u64}\n\
                var b = m[1u64]\n\
                return ((b + size(s0)) as u64) as bytes";
    let it2 = Interp::try_build(ctrl).expect("interp must build");
    let want2 = it2.run(&[0u8; 128]).expect("control must run");
    let rv2 = Rv64::try_build(ctrl).expect("rv64 builds");
    for (mode, got) in rv2.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want2.clone()), "control face (8-byte CAT) must agree, {mode:?}");
    }
    // the ITEMCONCAT face (4-byte total) and its 8-byte exemption
    let ic4 = "var l = [7u32]\n\
               var ic = item_concat(l)\n\
               var m = map{1u64 : 2u64}\n\
               var b = m[1u64]\n\
               return ((b + size(ic)) as u64) as bytes";
    let it3 = Interp::try_build(ic4).expect("interp must build");
    let want3 = it3.run(&[0u8; 128]).expect("interp must run");
    let rv3 = Rv64::try_build(ic4).expect("rv64 builds");
    for (mode, got) in rv3.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want3.clone()), "FINDING #2 item_concat face must agree, {mode:?}");
    }
    let ic8 = "var l = [7u64]\n\
               var ic = item_concat(l)\n\
               var m = map{1u64 : 2u64}\n\
               var b = m[1u64]\n\
               return ((b + size(ic)) as u64) as bytes";
    let it4 = Interp::try_build(ic8).expect("interp must build");
    let want4 = it4.run(&[0u8; 128]).expect("control must run");
    let rv4 = Rv64::try_build(ic8).expect("rv64 builds");
    for (mode, got) in rv4.run(&[0u8; 128]) {
        assert_eq!(got, Ok(want4.clone()), "control face (8-byte item_concat) must agree, {mode:?}");
    }
    println!("FINDING #2 flipped: non-multiple-of-8 CAT/item_concat + later PACKMAP + use -> four-way agreement (trap already fixed upstream by B6/B9; pin was stale)");
}

/// A fast always-on smoke: 12 programs through the same pipeline so a
/// trivial harness breakage is caught by the daily gate too.
#[test]
fn audit_smoke() {
    let mut t = Tally::default();
    for i in 0..12u64 {
        let seed = 0x5D0E_0000 + i;
        audit_one(seed, i % 3 == 0, rng_n_stmts(seed), &mut t);
    }
    assert!(t.built >= 10, "smoke accept rate: {}/12", t.built);
    assert!(t.comparisons >= t.built * 5);
}

fn rng_n_stmts(seed: u64) -> u64 {
    Rng::new(seed).below(20) + 12
}

/// Debug helper (not part of the audit): regenerate the body for
/// AUDIT_MIN_SEED / AUDIT_MIN_FAULT / AUDIT_MIN_N and line-minimize it
/// against the first diverging harness input. Used to shrink any
/// AUDIT DIVERGENCE panic into a minimal repro for the report.
#[test]
#[ignore]
fn audit_debug_minimize() {
    let seed = env_u64("AUDIT_MIN_SEED", 0);
    if seed == 0 {
        println!("set AUDIT_MIN_SEED (and optional AUDIT_MIN_FAULT/AUDIT_MIN_N)");
        return;
    }
    let fault = std::env::var("AUDIT_MIN_FAULT").is_ok();
    let n = env_u64("AUDIT_MIN_N", rng_n_stmts(seed));
    let (body, _) = gen_body(seed, fault, n);
    let inputs = inputs_for(seed);
    let diverges = |b: &str| -> Option<(usize, u64)> {
        let it = match Interp::try_build(b) {
            Ok(x) => x,
            Err(_) => return None,
        };
        let rv = match Rv64::try_build(b) {
            Ok(x) => x,
            Err(_) => return None,
        };
        for (ix, input) in inputs.iter().enumerate() {
            let want = it.run(input).map_err(|f| f.code as u64);
            if want.is_err() {
                continue;
            }
            for (_mode, got) in rv.run(input) {
                if got != want {
                    return Some((ix, 0));
                }
            }
        }
        None
    };
    if diverges(&body).is_none() {
        println!("seed {seed:#x} does not diverge — check the body/n");
        println!("{body}");
        return;
    }
    let mut lines: Vec<String> = body.split('\n').map(|s| s.to_string()).collect();
    let mut i = 0;
    while i < lines.len() {
        let mut cand = lines.clone();
        cand.remove(i);
        if diverges(&cand.join("\n")).is_some() {
            lines = cand;
        } else {
            i += 1;
        }
    }
    let min = lines.join("\n");
    println!("MINIMIZED ({} lines) for seed {seed:#x} fault={fault} n={n}:\n{min}", lines.len());
    let it = Interp::try_build(&min).unwrap();
    let rv = Rv64::try_build(&min).unwrap();
    for input in &inputs {
        let want = it.run(input).map_err(|f| f.code as u64);
        let gets = rv.run(input);
        if gets.iter().any(|(_, g)| *g != want) {
            println!("DIVERGES on input[{:>3}] {:?}: interp={want:?} modes={gets:?}", input.len(), &input[..8]);
        }
    }
}
