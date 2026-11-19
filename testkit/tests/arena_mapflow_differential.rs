//! B14/B9 (impl/arena-mapflow) targeted differential corpus.
//!
//! Every case runs the REAL pipeline — fitsh source →
//! `compile_rv64im_body_kernel` (per mode) → container → consensus verify
//! (`verify_container_with_cap`, so every instruction passes the consensus
//! decoder) → calc-interpreter execution — in all three lowering modes,
//! against native expected outputs (byte-exact / exit code).
//!
//! Candidate coverage:
//!   * B14 — "fresh empty map + first INSERT/ITEMSET" fact: the first
//!     INSERT after `PUT m := new_map()` lowers without the 46-instruction
//!     find, the 84 duplicate face, the 40 cap face and the shift loop
//!     (pos = 0 / found = 0 / len = 1 are compile-time constants); the
//!     first ITEMSET lowers to the certain missing-key 85. The fact dies
//!     at if-else joins, at any other map operation (LENGTH / HASKEY /
//!     ITEMGET / KEYS / REMOVE / MERGE) and at an alias PUT — every case
//!     here pins the interpreter's observable behavior around those kills.
//!   * B9 — `buf = buf ++ x` in-place append under the arena-top fact
//!     (the receive side's bytes are the last arena allocation): the
//!     receive-side copy is dropped. The OLD VIEW of the receive operand
//!     must stay byte-identical for every alias holder — the key hazard
//!     class — plus the non-receive shapes (`y ++ buf`), chained appends,
//!     buf_set → CAT fact handoff, C6-fused chains with the head leaf
//!     in place, and the fact killed by an intervening arena allocation.

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::SpaceCap;
use vm::fitshc::{compile_rv64im_body_kernel, Rv64imBodyMeta};
use vm::ir::rv64::Rv64Mode;
use vm::lang::Tokenizer;

const BUDGET: u64 = 50_000_000;

/* ---------------- case model ------------------------------------------ */

#[derive(Debug)]
enum Want {
    Ok(Vec<u8>),
    Exit(u64),
}

struct Case {
    id: &'static str,
    body: &'static str,
    input: Vec<u8>,
    want: Want,
}

fn be64(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/* ---------------- engine mounting ------------------------------------- */

fn compile_container(body: &str, m: Rv64Mode) -> CalcContainer {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let bytes = compile_rv64im_body_kernel(
        "arena_mapflow_kernel",
        tokens,
        Some("data"),
        &[],
        &[],
        &[],
        &Rv64imBodyMeta { mode: m, ..Rv64imBodyMeta::default() },
        &SpaceCap::new(0),
    )
    .unwrap_or_else(|e| panic!("body must compile ({m:?}): {e}\n{body}"));
    let (c, _) = CalcContainer::decode(&bytes).expect("container decodes");
    c
}

#[test]
fn arena_mapflow_differential_three_modes() {
    let cases = build_cases();
    assert!(cases.len() >= 25, "corpus shrank: {}", cases.len());
    let mut ran = 0usize;
    for &m in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        for c in &cases {
            run_case(c, m);
            ran += 1;
        }
    }
    println!("arena-mapflow differential: {} case-mode executions over {} cases", ran, cases.len());
}

fn run_case(c: &Case, m: Rv64Mode) {
    let container = compile_container(c.body, m);
    let module = verify_container_with_cap(&container, &SpaceCap::new(0))
        .unwrap_or_else(|e| panic!("[{} {m:?}] consensus verify rejected: {e}", c.id));
    let (res, _regs, _mem) = interp::run_lockstep(&module, &container, &c.input, BUDGET, None);
    let got = match res {
        Ok(r) => Ok(r.output),
        Err((CalcAbort::GuestExit(code), _)) => Err(code),
        Err((a, _)) => panic!("[{} {m:?}] unexpected abort {a:?}", c.id),
    };
    match (&c.want, got) {
        (Want::Ok(w), Ok(o)) => assert_eq!(
            o, *w,
            "[{} {m:?}] output mismatch (input 0x{}): got 0x{} want 0x{}",
            c.id,
            hex::encode(&c.input),
            hex::encode(&o),
            hex::encode(w)
        ),
        (Want::Exit(e), Err(g)) => assert_eq!(
            g, *e,
            "[{} {m:?}] exit mismatch (input 0x{}): got {g} want {e}",
            c.id,
            hex::encode(&c.input)
        ),
        (want, got) => panic!(
            "[{} {m:?}] face mismatch (input 0x{}): got {got:?} want {want:?}",
            c.id,
            hex::encode(&c.input)
        ),
    }

    // AOT parity (feature-gated backend): same lowered code, both engines.
    #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
    {
        let aot = vm::calc::aot::aot_compile(&module)
            .unwrap_or_else(|| panic!("[{} {m:?}] aot compile refused", c.id));
        let (out, _ar, _am) = aot.run_with_state(&module, &container, &c.input, BUDGET);
        match (&c.want, out.abort.is_none(), out.exit_code) {
            (Want::Ok(w), true, 0) => assert_eq!(
                out.output.as_deref(),
                Some(w.as_slice()),
                "[{} {m:?}] aot output mismatch",
                c.id
            ),
            (Want::Exit(e), false, g) => assert_eq!(g, *e, "[{} {m:?}] aot exit mismatch", c.id),
            _ => panic!("[{} {m:?}] aot face mismatch: {:?}", c.id, out.abort),
        }
    }
}

/* ---------------- corpus ----------------------------------------------- */

fn build_cases() -> Vec<Case> {
    let mut v: Vec<Case> = Vec::new();

    /* ---- B14: the len-zero map fact ------------------------------------ */

    // Constant key first INSERT consumes the fact (find/84/40/shift all
    // gone); the read-back goes through the regular itemget.
    v.push(Case {
        id: "b14.constkey.insert",
        body: "var m = new_map()\n\
               insert(m, 7u64, 40u64)\n\
               return ((m[7u64] as u64) * 10u64 + length(m)) as bytes",
        input: vec![],
        want: Want::Ok(be64(401)),
    });

    // Dynamic key: the find's collapse is key-independent (zero scan
    // iterations) — the fact consumes for dynamic keys too.
    v.push(Case {
        id: "b14.dynkey.insert",
        body: "var m = new_map()\n\
               var k = (byte(data, 0) as u64) % 5u64\n\
               insert(m, k, 40u64)\n\
               return ((m[k] as u64) * 10u64 + length(m)) as bytes",
        input: vec![3],
        want: Want::Ok(be64(401)),
    });

    // The SECOND INSERT is a regular-site overwrite (found = 1 path): the
    // fact was consumed by the first insert and the shift/cap/find faces
    // must behave exactly like the unfused lowering.
    v.push(Case {
        id: "b14.second.insert.overwrite",
        body: "var m = new_map()\n\
               insert(m, 7u64, 40u64)\n\
               insert(m, 7u64, 41u64)\n\
               return ((m[7u64] as u64) * 10u64 + length(m)) as bytes",
        input: vec![],
        want: Want::Ok(be64(411)),
    });

    // Sorted-order discipline across one fast insert + two regular ones.
    v.push(Case {
        id: "b14.three.sorted.keys",
        body: "var m = new_map()\n\
               insert(m, 3u64, 30u64)\n\
               insert(m, 1u64, 10u64)\n\
               insert(m, 2u64, 20u64)\n\
               var ks = keys(m)\n\
               return ((ks[0] as u64) * 100u64 + (ks[1] as u64) * 10u64 + (ks[2] as u64)) as bytes",
        input: vec![],
        want: Want::Ok(be64(123)),
    });

    // An if-else join between the create and the insert kills the fact at
    // place(): both arms must land on the regular path with identical
    // observable behavior.
    let ifjoin = "var m = new_map()\n\
                  var t = 0u64\n\
                  if byte(data, 0) == 1u8 { t = 1u64 } else { t = 2u64 }\n\
                  insert(m, 7u64, 40u64)\n\
                  return ((m[7u64] as u64) + t) as bytes";
    v.push(Case { id: "b14.ifjoin.then", body: ifjoin, input: vec![1], want: Want::Ok(be64(41)) });
    v.push(Case { id: "b14.ifjoin.else", body: ifjoin, input: vec![2], want: Want::Ok(be64(42)) });

    // Alias PUT: the object gets a second name — the fact dies on BOTH
    // slots; inserts through either name share the object.
    v.push(Case {
        id: "b14.alias.put",
        body: "var m = new_map()\n\
               var b = m\n\
               insert(b, 1u64, 2u64)\n\
               insert(m, 2u64, 3u64)\n\
               return (((m[1u64] as u64) * 10u64 + m[2u64]) * 10u64 + length(m)) as bytes",
        input: vec![],
        want: Want::Ok(be64(232)),
    });

    // ITEMSET on a len-zero map: EVERY key is missing → the certain 85.
    v.push(Case {
        id: "b14.itemset.empty.85",
        body: "var m = new_map()\n\
               item_set(m, 1u64, 2u64)\n\
               return 0u64 as bytes",
        input: vec![],
        want: Want::Exit(85),
    });

    // ITEMSET on a len-one map (the fast insert ran first): existing key
    // overwrites, missing key still faults 85.
    v.push(Case {
        id: "b14.itemset.hit.after.fastinsert",
        body: "var m = new_map()\n\
               insert(m, 1u64, 2u64)\n\
               item_set(m, 1u64, 9u64)\n\
               return ((m[1u64] as u64) * 10u64 + length(m)) as bytes",
        input: vec![],
        want: Want::Ok(be64(91)),
    });
    v.push(Case {
        id: "b14.itemset.miss.after.fastinsert",
        body: "var m = new_map()\n\
               insert(m, 1u64, 2u64)\n\
               item_set(m, 9u64, 9u64)\n\
               return 0u64 as bytes",
        input: vec![],
        want: Want::Exit(85),
    });

    // Bytes keys: the first insert (dynamic bytes key) takes the fact; the
    // second is a regular insert; KEYS sorts canonically.
    v.push(Case {
        id: "b14.byteskey.first",
        body: "var m = new_map()\n\
               insert(m, buf_left(2, data), 1u64)\n\
               insert(m, buf_right(1, data), 2u64)\n\
               var ks = keys(m)\n\
               return ks[0]",
        input: vec![0x02, 0x01],
        want: Want::Ok(vec![0x01]),
    });

    // Bytes values through the fast insert.
    v.push(Case {
        id: "b14.bytesvalue",
        body: "var m = new_map()\n\
               insert(m, 1u64, buf_left(3, data))\n\
               return m[1u64]",
        input: vec![0xaa, 0xbb, 0xcc],
        want: Want::Ok(vec![0xaa, 0xbb, 0xcc]),
    });

    // Zero key (the minimal canonical key bytes).
    v.push(Case {
        id: "b14.zero.key",
        body: "var m = new_map()\n\
               insert(m, 0u64, 5u64)\n\
               return ((m[0u64] as u64) * 10u64 + length(m)) as bytes",
        input: vec![],
        want: Want::Ok(be64(51)),
    });

    // Read-only map ops between create and insert kill the fact
    // conservatively — behavior identical either way.
    v.push(Case {
        id: "b14.haskey.between",
        body: "var m = new_map()\n\
               var h = has_key(m, 1u64)\n\
               insert(m, 1u64, 2u64)\n\
               return ((h as u64) * 10u64 + (m[1u64] as u64)) as bytes",
        input: vec![],
        want: Want::Ok(be64(2)),
    });
    v.push(Case {
        id: "b14.len.between",
        body: "var m = new_map()\n\
               var l = length(m)\n\
               insert(m, 1u64, 2u64)\n\
               return ((l as u64) * 10u64 + length(m)) as bytes",
        input: vec![],
        want: Want::Ok(be64(1)),
    });

    // MERGE after a fast insert mutates the receiver.
    v.push(Case {
        id: "b14.merge.after",
        body: "var m = new_map()\n\
               insert(m, 1u64, 2u64)\n\
               var b = map{3u64 : 4u64}\n\
               merge(m, b)\n\
               return ((m[1u64] as u64) * 10u64 + m[3u64]) as bytes",
        input: vec![],
        want: Want::Ok(be64(24)),
    });

    // Missing-key ITEMGET after the fast insert: the regular find on a
    // len-one map faults 85.
    v.push(Case {
        id: "b14.itemget.missing.85",
        body: "var m = new_map()\n\
               insert(m, 1u64, 2u64)\n\
               return (m[9u64] as u64) as bytes",
        input: vec![],
        want: Want::Exit(85),
    });

    // The key's 73 faces keep their position BEFORE the (deleted) find:
    // a Bool key still faults 73, a may-Nil key still faults 73.
    v.push(Case {
        id: "b14.boolkey.73",
        body: "var m = new_map()\n\
               insert(m, (1u64 == 1u64), 5u64)\n\
               return 0u64 as bytes",
        input: vec![],
        want: Want::Exit(73),
    });
    v.push(Case {
        id: "b14.nilkey.73",
        body: "var m = new_map()\n\
               var q = 0u64\n\
               if q == 1u64 { var t = \"ab\" }\n\
               insert(m, t, 5u64)\n\
               return 0u64 as bytes",
        input: vec![],
        want: Want::Exit(73),
    });

    /* ---- B9: the arena-top fact / in-place append ---------------------- */

    // Single-layer in-place: the first CAT allocates (fact established on
    // the result), the second consumes it — the receive side is NOT
    // re-copied and the output is the interpreter's concat.
    v.push(Case {
        id: "b9.inplace.single",
        body: "var b = data ++ \"abc\"\n\
               b = b ++ \"def\"\n\
               return b",
        input: vec![0x11, 0x22],
        want: Want::Ok(b"\x11\x22abcdef".to_vec()),
    });

    // The arena-top value on the RIGHT is not a receive side: a literal
    // receiver copies in front (no fact on the literal pair).
    v.push(Case {
        id: "b9.right.not.recv",
        body: "var b = data ++ \"abc\"\n\
               var c = \"pre\" ++ b\n\
               return c",
        input: vec![0x11],
        want: Want::Ok(b"pre\x11abc".to_vec()),
    });

    // THE alias hazard: a holder of the old descriptor must keep reading
    // exactly its old bytes after the in-place append (alias holders index
    // by their own len; the append writes only above the old tail).
    v.push(Case {
        id: "b9.alias.oldview",
        body: "var b = data ++ \"abc\"\n\
               var c = b\n\
               b = b ++ \"def\"\n\
               return c ++ \"|\" ++ b",
        input: vec![0x11],
        want: Want::Ok(b"\x11abc|\x11abcdef".to_vec()),
    });

    // Chained appends across statements: each statement consumes the
    // previous result's fact.
    v.push(Case {
        id: "b9.chain.two",
        body: "var b = data ++ \"ab\"\n\
               b = b ++ \"cd\"\n\
               var b2 = b ++ \"ef\"\n\
               return b2",
        input: vec![0x05],
        want: Want::Ok(b"\x05abcdef".to_vec()),
    });

    // C6 fused chain whose HEAD leaf carries the fact: leaf 0's memcpy is
    // skipped inside the fused emission, leaves 1.. append over S1.
    v.push(Case {
        id: "b9.fused.head",
        body: "var b = data ++ \"x\"\n\
               b = (b ++ \"y\") ++ \"z\"\n\
               return b",
        input: vec![0x01],
        want: Want::Ok(b"\x01xyz".to_vec()),
    });

    // buf_set's result is a fresh arena-top copy — the fact hands off to
    // the following CAT; the patched byte and the appended byte coexist.
    v.push(Case {
        id: "b9.bufset.then.cat",
        body: "var b = \"abcdefgh\"\n\
               b = buf_set(b, 2u64, 85u8)\n\
               b = b ++ \"!\"\n\
               return ((byte(b, 2) as u64) * 1000u64 + (size(b) as u64)) as bytes",
        input: vec![],
        want: Want::Ok(be64(85_009)),
    });

    // Self-concat: the receive side and the appended side are the SAME
    // descriptor — the forward byte copy must not corrupt (src == dst
    // region boundary is exact).
    v.push(Case {
        id: "b9.self.concat",
        body: "var b = data ++ \"abc\"\n\
               b = b ++ b\n\
               b = b ++ b\n\
               return (size(b) as u64) as bytes",
        input: vec![0x11, 0x22],
        want: Want::Ok(be64(20)),
    });

    // cbytes result hands its fact to a CAT chain.
    v.push(Case {
        id: "b9.cbytes.chain",
        body: "var n = 300u64\n\
               var h = n as bytes\n\
               h = h ++ h\n\
               h = h ++ h\n\
               return (size(h) as u64) as bytes",
        input: vec![],
        want: Want::Ok(be64(32)),
    });

    // An intervening arena allocation (the map object) kills the fact —
    // the append falls back to the regular copy; output identical.
    v.push(Case {
        id: "b9.mapalloc.between",
        body: "var b = data ++ \"abc\"\n\
               var m = new_map()\n\
               insert(m, 1u64, 2u64)\n\
               b = b ++ \"def\"\n\
               return ((size(b) as u64) * 10u64 + (m[1u64] as u64)) as bytes",
        input: vec![0x11],
        want: Want::Ok(be64(72)),
    });

    // Loop accumulator (the interpreter's quadratic; the lowering stays
    // correct across the backedge-kill discipline).
    v.push(Case {
        id: "b9.loop.accumulate",
        body: "var out = \"\"\n\
               var i = 0u64\n\
               while i < 5u64 { out = out ++ \"ab\"\n\
               i += 1u64 }\n\
               return out",
        input: vec![],
        want: Want::Ok(b"ababababab".to_vec()),
    });

    // Empty input: the input-local copy establishes a (bp, bl == 0) fact —
    // appending over a zero-length view must stay exact.
    v.push(Case {
        id: "b9.empty.left",
        body: "var b = data ++ \"x\"\n\
               b = b ++ \"y\"\n\
               return b",
        input: vec![],
        want: Want::Ok(b"xy".to_vec()),
    });

    // A slice (buf_cut) as the appended source: the copy reads from the
    // middle of the arena while writing at the top.
    v.push(Case {
        id: "b9.cut.source",
        body: "var b = data ++ \"ab\"\n\
               b = b ++ buf_cut(data, 1, 1)\n\
               return b",
        input: vec![0x11, 0x22],
        want: Want::Ok(b"\x11\x22ab\x22".to_vec()),
    });

    // B14 + B9 in one body: the map's fast insert does not disturb the
    // bytes chain's fact handoff (the alloc between them kills it).
    v.push(Case {
        id: "mix.map.and.cat",
        body: "var m = new_map()\n\
               var b = data ++ \"k\"\n\
               insert(m, 1u64, 2u64)\n\
               b = b ++ \"z\"\n\
               return ((size(b) as u64) * 10u64 + (m[1u64] as u64)) as bytes",
        input: vec![0x11],
        want: Want::Ok(be64(32)),
    });

    v
}
