#![allow(unused_imports)]

use super::super::*;
use crate::calc::decode::{BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
use crate::calc::{CalcFnObj, CalcRunLimits};
use crate::fitshc::compile_body;
use crate::lang::Tokenizer;

use super::helpers::{HIP5_FITSH, census_stream, compile, compile_with_input, deep_add_body, hip5_wire_narrow_safe, run_e2e, run_e2e_mode, run_input, rust_rvc_code};


    #[test]
    fn hip5_rvc_report() {
        let (ir, hvm) = compile_with_input(HIP5_FITSH);
        let (hvm_bytes, hvm_gas) = hvm_bytecode_stats(&hvm).unwrap();
        println!("HIP5 fitsh: hvm bytecode = {hvm_bytes} B, static gas = {hvm_gas}");

        // task-1 A3: the random wire trips the interpreter's narrow-width
        // fault faces (nibbles ≥ 8, decimal digits 8/9) — both engines trap
        // 90 together there. The report needs a successful full run, so it
        // uses the narrow-safe wire (same shape, constrained nibbles/digits).
        let input = hip5_wire_narrow_safe(200);
        assert_eq!(input.len(), 4800);
        let mut outs: Vec<Vec<u8>> = Vec::new();
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).unwrap();
            // The 40 KiB arena (ARENA_SIZE_MAX, the L1-layout maximum with
            // io 4800+8 under the 64 KiB cap) absorbs the digit-string churn
            // the bump allocator never reclaims — ~199 B per record for
            // u64-range numbers; the deployed rust kernel manages with
            // 14 KiB via a fixed digits buffer. A production backend wants
            // GC or slot regions instead of a bigger box.
            let cap = SpaceCap::new(0);
            let container = match mode {
                Rv64Mode::Im => lo.pack_segments(&cap, 64, 4800, 8),
                _ => lo.pack_segments_profile(
                    &cap,
                    64,
                    4800,
                    8,
                    crate::calc::container::CALC_PROFILE_RV64IMC,
                ),
            }
            .unwrap_or_else(|e| panic!("pack {mode:?}: {e}"));
            let fno = CalcFnObj::build_v1(container).unwrap();
            let exec =
                fno.run(&input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
            let out = match exec.result {
                Ok(o) => o,
                Err(e) => panic!("run {mode:?}: {e:?} exit={}", exec.exit_code),
            };
            println!(
                "HIP5 fitsh {mode:?}: code+pool = {} B (code {}), insts = {}, static cycles = {}, exec cycles = {} ({:.2}/record)",
                lo.code.len() + lo.pool.len(),
                lo.code.len(),
                lo.stats.insts,
                lo.stats.cycles,
                exec.cycles_used,
                exec.cycles_used as f64 / 200.0,
            );
            println!("  sum[{mode:?}] = {}", u64::from_be_bytes(out.as_slice().try_into().unwrap()));
            outs.push(out);
        }
        assert_eq!(outs[0], outs[1], "Imc output must equal Im");
        assert_eq!(outs[0], outs[2], "ImcRemap output must equal Im");
        println!("HIP5 fitsh sum (Im) = {}", u64::from_be_bytes(outs[0].as_slice().try_into().unwrap()));

        // Diff against the deployed rust kernel container (local-only fixture;
        // skipped when the .loc dir is absent so clean checkouts stay green).
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let rel = std::path::Path::new("doc/hacdsdefi4000.loc/out/score_sum.hcf1");
        let hcf1 = [base.join(rel), base.join("../..").join(rel), base.join("..").join(rel)]
            .into_iter()
            .find(|p| p.exists());
        let bytes = match hcf1 {
            Some(p) => std::fs::read(p).ok(),
            None => {
                println!("rust container fixture not present, diff skipped");
                None
            }
        };
        if let Some(bytes) = bytes {
            let (c, _) = crate::calc::container::CalcContainer::decode(&bytes).unwrap();
            let fno = CalcFnObj::build_v1(c).unwrap();
            let exec =
                fno.run(&input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
            let rust_out = match exec.result {
                Ok(o) => o,
                Err(e) => panic!("rust kernel: {e:?} exit={}", exec.exit_code),
            };
            println!(
                "HIP5 rust kernel (deployed, profile 0): out = {}, exec cycles = {} ({:.2}/record)",
                u64::from_be_bytes(rust_out.as_slice().try_into().unwrap()),
                exec.cycles_used,
                exec.cycles_used as f64 / 200.0,
            );
            assert_eq!(outs[0], rust_out, "fitsh scorer must reproduce the rust kernel output");

            // The rust kernel rebuilt at profile 2 (riscv64imac): must
            // decode under RVC and produce the same sum.
            let rel2 = std::path::Path::new("doc/hacdsdefi4000.loc/out_rvc/score_sum.hcf1");
            let rvc_path = [
                base.join(rel2),
                base.join("..").join(rel2),
                base.join("../..").join(rel2),
            ]
            .into_iter()
            .find(|p| p.exists());
            if let Some(p) = rvc_path {
                let (c, _) =
                    crate::calc::container::CalcContainer::decode(&std::fs::read(p).unwrap())
                        .unwrap();
                assert_eq!(c.profile, crate::calc::container::CALC_PROFILE_RV64IMC);
                let fno = CalcFnObj::build_v1(c).unwrap();
                let exec = fno
                    .run(&input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
                let rvc_out = match exec.result {
                    Ok(o) => o,
                    Err(e) => panic!("rust rvc kernel: {e:?} exit={}", exec.exit_code),
                };
                assert_eq!(rvc_out, rust_out, "profile-2 rust kernel must match profile-0");
                println!(
                    "HIP5 rust kernel (profile 2, rvc): out = {}, exec cycles = {} ({:.2}/record)",
                    u64::from_be_bytes(rvc_out.as_slice().try_into().unwrap()),
                    exec.cycles_used,
                    exec.cycles_used as f64 / 200.0,
                );
            } else {
                println!("rust profile-2 fixture not present, rvc diff skipped");
            }
        }
    }


    /// Residue diagnosis: every 4-byte br-zero / j site in the fitsh
    /// ImcRemap stream, with the offending register, plus the slot ld/sd
    /// register census (window vs scratch vs full-width).
    #[test]
    fn fwd_mode_differential_hunt() {
        // bodies exercising PUT→GET forwarding, XOP, banking shapes
        #[allow(clippy::type_complexity)]
        let bodies: Vec<(&str, u64)> = vec![
            ("var s = 0u64\nvar i = 0u64\nwhile i < 10 { s += i\ni += 1 }\nreturn s as bytes", 45),
            ("var s = 0u64\nvar t = 5u64\nvar i = 0u64\nwhile i < 8 { t = t + i\ns += t\ni += 1 }\nreturn (s + t) as bytes", 157),
            ("var a = 3u64\nvar b = 4u64\nvar c = a + b\nvar d = c * 2\nreturn (c + d) as bytes", 21),
            ("var x = 10u64\nvar y = x + 5\nx = y * 2\nreturn (x + y) as bytes", 45),
            ("var s = 0u64\nvar i = 0u64\nwhile i < 6 { var t = i * i\ns += t\ni += 1 }\nreturn s as bytes", 55),
            ("var s = 0u64\nvar i = 0u64\nwhile i < 5 { if i % 2 == 0 { s += i }\ni += 1 }\nreturn s as bytes", 6),
            ("var p = 0x0011223344556677\nvar q = p\nvar r = byte(q, 2)\nreturn ((r + size(p)) as u64) as bytes", 0x22 + 8),
            ("var unused = 42u64\nvar keep = 7u64\nreturn keep as bytes", 7),
            ("var s = 0u64\nvar i = 0u64\nwhile i < 4 { var a = i + 1\nvar b = a * 3\nvar c = a + b\ns += c\ni += 1 }\nreturn s as bytes", 40),
            ("var m = 0u64\nvar i = 0u64\nwhile i < 5 { if i > m { m = i }\ni += 1 }\nreturn m as bytes", 4),
            ("var m = 100u64\nvar i = 0u64\nwhile i < 5 { if 200 > m { m = 200 }\nif m < 150 { m = 150 }\ni += 1 }\nreturn m as bytes", 200),
            ("var m = 0u64\nvar n = 7u64\nif n > m { m = n }\nreturn m as bytes", 7),
            ("var a = 3u64\nvar b = 9u64\nvar m = 0u64\nwhile a < b { if a > m { m = a }\na += 2 }\nreturn m as bytes", 7),
            ("var lo = 1000u64\nvar i = 0u64\nwhile i < 4 { if i < lo { lo = i }\ni += 1 }\nreturn lo as bytes", 0),
        ];
        // the oracle is the source semantics (pinned constants), not a
        // sibling mode — a shared bug would hide from a mode-vs-mode diff
        for (bi, (body, want)) in bodies.iter().enumerate() {
            let out_im = run_e2e_mode(body, &[], Rv64Mode::Im);
            assert_eq!(
                out_im, want.to_be_bytes(),
                "body #{bi} Im wrong: {body}"
            );
            for mode in [Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                let out = run_e2e_mode(body, &[], mode);
                assert_eq!(
                    out, want.to_be_bytes(),
                    "body #{bi} mode {mode:?} wrong: {body}"
                );
            }
            println!("body #{bi} ok = {out_im:x?}");
        }
    }


    /// Disassemble body #9 of the differential hunt (the failing max/min
    /// shape) for manual inspection.
    /// Value-position if/choose executes correctly end to end (the
    /// historically-unimplemented eval path). The `1`/`0`/`2`… arms are
    /// U8 literals and `byte()` operands are U8, so `as bytes` yields the
    /// 1-byte width-exact view (task-1 A2 — same as the interpreter).
    #[test]
    fn value_if_e2e() {
        let body = "var a = byte(data, 0)\nvar r = if a != 0 { 1 } else { 0 }\nreturn r as bytes";
        let out = run_input(body, &[5]);
        assert_eq!(out, 1u8.to_be_bytes());
        let out0 = run_input(body, &[0]);
        assert_eq!(out0, 0u8.to_be_bytes());
        // nested value-if and a bytes-typed arm
        let nested = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = if a > b { if a > 100 { 2 } else { 3 } } else { 4 }\nreturn r as bytes";
        assert_eq!(run_input(nested, &[200, 5]), 2u8.to_be_bytes());
        assert_eq!(run_input(nested, &[50, 5]), 3u8.to_be_bytes());
        assert_eq!(run_input(nested, &[1, 5]), 4u8.to_be_bytes());
        // a Ref-valued arm (bare variable) materializes into the shared
        // join register
        let refarm = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = if a > b { a } else { b }\nreturn r as bytes";
        assert_eq!(run_input(refarm, &[9, 4]), 9u8.to_be_bytes());
        assert_eq!(run_input(refarm, &[4, 9]), 9u8.to_be_bytes());
        // the u64-pinned form keeps the historical 8-byte view
        let pinned = "var a = byte(data, 0)\nvar r = (if a != 0 { 1 } else { 0 }) as u64\nreturn r as bytes";
        assert_eq!(run_input(pinned, &[5]), 1u64.to_be_bytes());
    }


    #[test]
    fn pool_const_repro() {
        for v in [2048u64, 13107, 21845, 1000000007, 1_000_000_000_000] {
            // ≥4 literal sites activate the pool-base register
            let body = format!(
                "var a = 0x11\nvar b = 0x22\nvar c = a ++ b\nvar d = c ++ a\nvar x = {v}u64\nvar y = x % 65537\nreturn (x + y + size(c) + size(d)) as bytes"
            );
            let body = body.replace("\\n", "\n");
            let (ir, _) = compile(&body);
            let lo = lower_body_mode_arena(&ir, false, Rv64Mode::ImcRemap, ARENA_SIZE).unwrap();
            let cap = SpaceCap::new(0);
            let fno = CalcFnObj::build_v1(
                lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC)
                    .unwrap(),
            )
            .unwrap();
            let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
            match exec.result {
                Ok(o) => {
                    let got = u64::from_be_bytes(o.as_slice().try_into().unwrap());
                    println!("pool const {v}: got {got} {}",
                        if got == v + (v % 65537) + 2 + 3 { "OK" } else { "WRONG" });
                    assert_eq!(got, v + (v % 65537) + 2 + 3, "pool const {v} wrong");
                }
                Err(e) => panic!("pool const {v}: {e:?} exit={}", exec.exit_code),
            }
        }
    }


    #[test]
    fn hip5_residue_diagnosis() {
        let (ir, _) = compile_with_input(HIP5_FITSH);
        let lo = lower_body_mode_arena(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
        let profile = crate::calc::container::CALC_PROFILE_RV64IMC;
        let mut off = 0usize;
        let mut brz4 = 0;
        let mut j4 = 0;
        while off < lo.code.len() {
            let (inst, len) = decode_at(&lo.code, off, profile).unwrap();
            let l = len as usize;
            match &inst {
                Inst::Branch { rs2: 0, rs1, .. } if l == 4 => {
                    brz4 += 1;
                    println!("brz4 @{off:04x}: {inst:?}");
                }
                Inst::Jal { rd: 0, .. } if l == 4 => {
                    j4 += 1;
                    println!("j4   @{off:04x}: {inst:?}");
                }
                _ => {}
            }
            off += l;
        }
        println!("total 4B br-zero = {brz4}, 4B j = {j4}");

        // slot ld/sd census by (rs1 base, offset) → approximate slot identity
        let mut mem: std::collections::BTreeMap<(u8, i64), (usize, usize)> = Default::default();
        let mut off = 0usize;
        while off < lo.code.len() {
            let (inst, len) = decode_at(&lo.code, off, profile).unwrap();
            let l = len as usize;
            match &inst {
                Inst::Load { op: LoadOp::Ld, rs1, imm, .. } => {
                    let e = mem.entry((*rs1, *imm)).or_default();
                    e.0 += 1;
                    e.1 += l;
                }
                Inst::Store { op: StoreOp::Sd, rs1, imm, .. } => {
                    let e = mem.entry((*rs1, *imm)).or_default();
                    e.0 += 1;
                    e.1 += l;
                }
                _ => {}
            }
            off += l;
        }
        println!("-- slot ld/sd by (base,off) --");
        for ((b, o), (n, by)) in &mem {
            println!("  x{b} {o:>4}: {n:>3} inst, {by} B");
        }
    }


    /// Static per-slot access counts + peak expression depth for HIP5
    /// (drives local-promotion bank sizing / prioritization).
    #[test]
    fn and_ir_shape_probe() {
        let body = "var a = 1u64\nvar b = 2u64\nvar c = 3u64\nvar x = 0u64\nif a == b && b == c { x = 1 }\nreturn x as bytes";
        let (ir, _) = compile(body);
        fn dump(n: &dyn IRNode, depth: usize) {
            let op = Bytecode::try_from_u8(n.bytecode()).unwrap();
            println!("{:indent$}{op:?}", "", indent = depth * 2);
            for c in children(n) {
                dump(c, depth + 1);
            }
        }
        for n in &ir.subs {
            dump(&**n, 0);
        }
    }


    #[test]
    fn hip5_depth_and_slots() {
        let (ir, _) = compile_with_input(HIP5_FITSH);
        let fp = plan(&ir.subs, true, crate::rt::HEAP_GROW_SEGMENT_DEFAULT, false).unwrap();
        let mut uses = vec![0usize; fp.slot.len()];
        let mut writes = vec![0usize; fp.slot.len()];
        fn walkc(n: &dyn IRNode, uses: &mut Vec<usize>, writes: &mut Vec<usize>) {
            if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
                return walkc(&*w.node, uses, writes);
            }
            let op = Bytecode::try_from_u8(n.bytecode()).unwrap();
            match op {
                Bytecode::GET | Bytecode::GET0 | Bytecode::GET1 | Bytecode::GET2 | Bytecode::GET3 => {
                    let idx = match op {
                        Bytecode::GET => {
                            n.as_any().downcast_ref::<IRNodeParam1>().unwrap().para as usize
                        }
                        Bytecode::GET0 => 0,
                        Bytecode::GET1 => 1,
                        Bytecode::GET2 => 2,
                        _ => 3,
                    };
                    uses[idx] += 1;
                }
                Bytecode::PUT => {
                    let p = n.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    writes[p.para as usize] += 1;
                }
                Bytecode::XOP => {
                    let p = n.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    let (_, idx) = decode_local_operand_mark(p.para);
                    uses[idx as usize] += 1;
                    writes[idx as usize] += 1;
                }
                _ => {}
            }
            for c in children(n) {
                walkc(c, uses, writes);
            }
        }
        for n in &ir.subs {
            walkc(&**n, &mut uses, &mut writes);
        }
        println!("slots = {}", fp.slot.len());
        for (i, ((s, u), w)) in fp.slot.iter().zip(&uses).zip(&writes).enumerate() {
            if *u + *w > 0 {
                println!("  slot {i:>2} {:?}: reads+writes = {}", s, u + w);
            }
        }
        let nilp = NilPlan { tracked: vec![false; fp.slot.len()], occ: HashMap::new(), tag_bytes: false };
        let (mw_sites, hu_sites, he_sites) =
            body_routine_site_counts(&ir.subs, &fp.slot, &fp.slot_w, &[]);
        let mut lw = Lower::new_mode_arena_nil(
            &fp.slot,
            &fp.needs_zero,
            Rv64Mode::ImcRemap,
            ARENA_SIZE_MAX,
            &nilp,
            &ListPlanData::default(),
            &fp.slot_w,
            &[],
            false,
            false,
            0,
            None,
            false,
            (
                mw_sites >= FIN_ROUTINE_MIN_SITES,
                hu_sites >= FIN_ROUTINE_MIN_SITES,
                he_sites >= FIN_ROUTINE_MIN_SITES,
                false,
                false,
                false,
                false,
                false,
            ),
        );
        lw.emit_prologue_body_epilogue(&ir.subs).unwrap();
        println!("HIP5 peak vstack depth = {}", lw.max_depth);
    }




    #[test]
    fn hip5_census_report() {
        let (ir, _) = compile_with_input(HIP5_FITSH);
        let lo = lower_body_mode_arena(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
        let fitsh = census_stream(&lo.code, crate::calc::container::CALC_PROFILE_RV64IMC);
        println!("\n== fitsh ImcRemap code = {} B ==", lo.code.len());
        println!("{:<12} {:>5} {:>7} {:>5}", "cat", "n", "bytes", "2B");
        let mut tn = 0;
        let mut tb = 0;
        for (k, n, b, c) in &fitsh {
            println!("{:<12} {:>5} {:>7} {:>5}", k, n, b, c);
            tn += n;
            tb += b;
        }
        println!("{:<12} {:>5} {:>7}", "TOTAL", tn, tb);
        assert_eq!(tb, lo.code.len());

        if let Some(rcode) = rust_rvc_code() {
            let rust = census_stream(&rcode, crate::calc::container::CALC_PROFILE_RV64IMC);
            println!("\n== rust profile-2 code = {} B ==", rcode.len());
            let mut tn = 0;
            let mut tb = 0;
            for (k, n, b, c) in &rust {
                println!("{:<12} {:>5} {:>7} {:>5}", k, n, b, c);
                tn += n;
                tb += b;
            }
            println!("{:<12} {:>5} {:>7}", "TOTAL", tn, tb);
            assert_eq!(tb, rcode.len());

            // side-by-side delta
            println!("\n== delta (fitsh - rust) by category ==");
            println!("{:<12} {:>7} {:>7} {:>8}", "cat", "fitsh", "rust", "deltaB");
            let mut keys: Vec<&str> = fitsh.iter().map(|r| r.0).collect();
            for (k, _, _, _) in &rust {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            keys.sort_unstable();
            for k in keys {
                let f = fitsh.iter().find(|r| r.0 == k).map(|r| r.2).unwrap_or(0);
                let r = rust.iter().find(|r| r.0 == k).map(|r| r.2).unwrap_or(0);
                println!("{:<12} {:>7} {:>7} {:>8}", k, f, r, f as i64 - r as i64);
            }
        } else {
            println!("rust fixture missing; fitsh-only census");
        }

        // fitsh-side cause refinements (register/target heuristics), v2
        // (P1-G): guard control transfer into the shared fault thunks is
        // peeled out of the br / br-zero / j form buckets into its own
        // semantic bucket (it used to be split by match-arm order: rs2==0
        // guard branches landed in "forward 4B br-zero" or were dropped
        // entirely, and the r==limit c.j fast path landed in "j"). The
        // thunk entries are located by their tail `li a0, <exit code>`
        // immediates inside the final 32 B — the old fixed 7-instruction
        // walkback landed 2 B early because the compressed tail has 8
        // instructions. Pure measurement: nothing here feeds the emitter.
        let profile = crate::calc::container::CALC_PROFILE_RV64IMC;
        let code = &lo.code;
        let mut offs = Vec::new();
        let mut insts = Vec::new();
        let mut lens = Vec::new();
        {
            let mut p = 0usize;
            while p < code.len() {
                let (i, l) = decode_at(code, p, profile).unwrap();
                offs.push(p);
                insts.push(i);
                lens.push(l as usize);
                p += l as usize;
            }
        }
        assert_eq!(
            insts.last(),
            Some(&Inst::Ecall),
            "ecall must close the fitsh stream"
        );
        let tail_lo = code.len().saturating_sub(32);
        let thunk_li = |imm_want: i64| -> usize {
            let mut found = None;
            for i in 0..insts.len() {
                if offs[i] < tail_lo {
                    continue;
                }
                if let Inst::ImmAlu { op: ImmAluOp::Addi, rd: 10, rs1: 0, imm } = &insts[i] {
                    if *imm == imm_want {
                        found = Some(offs[i]);
                    }
                }
            }
            found.unwrap_or_else(|| panic!("thunk li a0, {imm_want} not found in the final 32 B"))
        };
        let thunk_arith = thunk_li(EXIT_ARITH as i64);
        let thunk_cast = thunk_li(EXIT_CAST as i64);
        assert!(thunk_arith < thunk_cast, "thunk tail layout changed");
        // GT-a copy thunks: `li a0,{90,71}` strictly below the shared-thunk
        // region, followed by a jump into a shared thunk (c.j form) or into
        // the trap tail (jal form). No other stream shape emits that pair.
        let mut copy_of: std::collections::BTreeMap<usize, i64> = Default::default();
        for i in 0..insts.len() {
            if offs[i] >= thunk_arith {
                break;
            }
            let code = match &insts[i] {
                Inst::ImmAlu { op: ImmAluOp::Addi, rd: 10, rs1: 0, imm }
                    if *imm == EXIT_ARITH as i64 || *imm == EXIT_CAST as i64 =>
                {
                    *imm
                }
                _ => continue,
            };
            let jump_ok = i + 1 < insts.len()
                && match &insts[i + 1] {
                    Inst::Jal { rd: 0, target, .. } => {
                        let abs = offs[i + 1] as i64 + target;
                        abs == thunk_arith as i64
                            || abs == thunk_cast as i64
                            || (lens[i + 1] == 4 && abs > thunk_cast as i64)
                    }
                    _ => false,
                };
            if jump_ok {
                copy_of.insert(offs[i], code);
            }
        }
        let mut slot_n = 0usize;
        let mut slot_b = 0usize;
        let mut arena_sb_n = 0usize;
        let mut arena_sb_b = 0usize;
        let mut guard_n = 0usize;
        let mut guard_b = 0usize;
        let mut guard_arith = (0usize, 0usize);
        let mut guard_cast = (0usize, 0usize);
        let mut guard_br_rs2nz = (0usize, 0usize);
        let mut guard_br_rs2z = (0usize, 0usize);
        let mut guard_jal = (0usize, 0usize);
        let mut guard_2b = (0usize, 0usize);
        // P3-GUARDTAIL GT-a: copy thunks (`li a0,<code>` + jump) inserted so
        // clustered zero-form guards reach a 2-byte c.bz; the copies ARE
        // guard machinery and the converted sites still guard — both are
        // counted in the guard totals above, this line reports the split.
        let mut guard_copy = (0usize, 0usize); // copy thunks: n, bytes
        let mut guard_copy_cj = 0usize; // copies taking the c.j tail form
        let mut guard_conv = (0usize, 0usize); // converted sites: n, bytes
        let mut brfwd4_z_all = (0usize, 0usize); // forward br-zero, 4B, incl. guards
        let mut brfwd4_z_guard = (0usize, 0usize);
        let mut brfwd4_z_win = 0usize; // forward non-guard 4B br-zero in c.beqz window
        let mut jfwd4_n = 0usize; // forward j, 4B (could be c.j)
        let mut jfwd4_b = 0usize;
        let mut ng_br = (0usize, 0usize); // non-guard br (rs2 != 0, not thunk-bound)
        let mut ng_brz = (0usize, 0usize); // non-guard br-zero
        let mut ng_brz_2b = 0usize;
        let mut li_hist: std::collections::BTreeMap<i64, usize> = Default::default();
        for i in 0..insts.len() {
            let off_i = offs[i];
            let len = lens[i];
            let abs_t =
                |t: i64| (off_i as i64 + t) as usize;
            match &insts[i] {
                Inst::Load { op: LoadOp::Ld, rs1: 8, .. }
                | Inst::Store { op: StoreOp::Sd, rs1: 8, .. } => {
                    slot_n += 1;
                    slot_b += len;
                }
                Inst::Store { op: StoreOp::Sb, rs1: 9, .. } => {
                    arena_sb_n += 1;
                    arena_sb_b += len;
                }
                Inst::Branch { target, rs2, .. } => {
                    let abs = abs_t(*target);
                    let conv_code = copy_of.get(&(abs as usize)).copied();
                    let guard = abs >= thunk_arith || conv_code.is_some();
                    if guard {
                        guard_n += 1;
                        guard_b += len;
                        if conv_code == Some(EXIT_ARITH as i64)
                            || (conv_code.is_none() && abs < thunk_cast)
                        {
                            guard_arith.0 += 1;
                            guard_arith.1 += len;
                        } else {
                            guard_cast.0 += 1;
                            guard_cast.1 += len;
                        }
                        if conv_code.is_some() {
                            guard_conv.0 += 1;
                            guard_conv.1 += len;
                        }
                        if *rs2 == 0 {
                            guard_br_rs2z.0 += 1;
                            guard_br_rs2z.1 += len;
                        } else {
                            guard_br_rs2nz.0 += 1;
                            guard_br_rs2nz.1 += len;
                        }
                        if len == 2 {
                            guard_2b.0 += 1;
                            guard_2b.1 += len;
                        }
                    } else if *rs2 == 0 {
                        ng_brz.0 += 1;
                        ng_brz.1 += len;
                        if len == 2 {
                            ng_brz_2b += 1;
                        }
                        if len == 4 && *target > 4 {
                            brfwd4_z_all.0 += 1;
                            brfwd4_z_all.1 += 4;
                            if (-256..=254).contains(target) {
                                brfwd4_z_win += 1;
                            }
                        }
                    } else {
                        ng_br.0 += 1;
                        ng_br.1 += len;
                    }
                    if len == 4 && *target > 4 && abs >= thunk_arith && *rs2 == 0 {
                        brfwd4_z_guard.0 += 1;
                        brfwd4_z_guard.1 += 4;
                    }
                }
                Inst::Jal { rd: 0, target, .. } => {
                    let abs = abs_t(*target);
                    if copy_of.contains_key(&(off_i.saturating_sub(4))) {
                        // GT-a copy thunk tail: the li counted the copy's
                        // head; count the jump half here
                        let code = copy_of[&(off_i - 4)];
                        guard_n += 1;
                        guard_b += len;
                        guard_copy.0 += 1;
                        guard_copy.1 += len;
                        if code == EXIT_ARITH as i64 {
                            guard_arith.0 += 1;
                            guard_arith.1 += len;
                        } else {
                            guard_cast.0 += 1;
                            guard_cast.1 += len;
                        }
                        if len == 2 {
                            guard_2b.0 += 1;
                            guard_2b.1 += len;
                            guard_copy_cj += 1;
                        }
                    } else if off_i < thunk_arith && abs >= thunk_arith {
                        // external jump into the trap tail: the r == limit
                        // cast fast path (the site preloads a0 itself)
                        guard_n += 1;
                        guard_b += len;
                        guard_cast.0 += 1;
                        guard_cast.1 += len;
                        guard_jal.0 += 1;
                        guard_jal.1 += len;
                        guard_2b.0 += 1;
                        guard_2b.1 += len;
                    } else if *target > 4 && len == 4 {
                        jfwd4_n += 1;
                        jfwd4_b += 4;
                    }
                }
                Inst::ImmAlu { op: ImmAluOp::Addi, rs1: 0, imm, .. }
                | Inst::Lui { val: imm, .. } => {
                    *li_hist.entry(*imm).or_insert(0) += 1;
                    if let Some(&code) = copy_of.get(&off_i) {
                        // GT-a copy head: the jump half is counted in the
                        // Jal arm; the head bytes land in the guard total
                        guard_n += 1;
                        guard_b += len;
                        guard_copy.0 += 1;
                        guard_copy.1 += len;
                        if code == EXIT_ARITH as i64 {
                            guard_arith.0 += 1;
                            guard_arith.1 += len;
                        } else {
                            guard_cast.0 += 1;
                            guard_cast.1 += len;
                        }
                    }
                }
                _ => {}
            }
        }
        // cross-checks against the raw form table (guards must be exactly
        // the difference between the raw and peeled br/br-zero buckets)
        let (raw_br, raw_brz) = {
            let t = census_stream(code, profile);
            (
                t.iter().find(|r| r.0 == "br").map(|r| (r.1, r.2)).unwrap_or((0, 0)),
                t.iter().find(|r| r.0 == "br-zero").map(|r| (r.1, r.2)).unwrap_or((0, 0)),
            )
        };
        assert_eq!(
            ng_br.0 + guard_br_rs2nz.0,
            raw_br.0,
            "guard peel must cover all br"
        );
        assert_eq!(
            ng_brz.0 + guard_br_rs2z.0,
            raw_brz.0,
            "guard peel must cover all br-zero"
        );
        println!("\n== fitsh causes (v2, guard-corrected) ==");
        println!("slot ld/sd (base s0):        {slot_n:>4} inst, {slot_b:>5} B");
        println!("arena sb (base s1):          {arena_sb_n:>4} inst, {arena_sb_b:>5} B");
        println!(
            "guard → thunks:              {guard_n:>4} inst, {guard_b:>5} B \
             (arith→{EXIT_ARITH} {} inst/{} B, cast→{EXIT_CAST} {} inst/{} B)",
            guard_arith.0, guard_arith.1, guard_cast.0, guard_cast.1
        );
        println!(
            "  guard copies (GT-a):       {:>4} inst, {:>5} B \
             ({} copy thunks, {} c.j-form; {} converted 2B c.bz sites {}/{} B)",
            guard_copy.0 + guard_conv.0,
            guard_copy.1 + guard_conv.1,
            guard_copy.0 / 2,
            guard_copy_cj,
            guard_conv.0,
            guard_conv.0,
            guard_conv.1
        );
        println!(
            "  branch-form guards:        {:>4} inst, {:>5} B \
             (rs2≠0 {} inst/{} B, rs2==0 {} inst/{} B)",
            guard_br_rs2nz.0 + guard_br_rs2z.0,
            guard_br_rs2nz.1 + guard_br_rs2z.1,
            guard_br_rs2nz.0,
            guard_br_rs2nz.1,
            guard_br_rs2z.0,
            guard_br_rs2z.1
        );
        println!(
            "  jal fast-path guards:      {:>4} inst, {:>5} B \
             (r==limit cast, site preloads a0)",
            guard_jal.0, guard_jal.1
        );
        println!(
            "  2B-encoded guards:         {:>4} inst, {:>5} B \
             (c.beqz-in-window + c.j fast path)",
            guard_2b.0, guard_2b.1
        );
        println!(
            "forward 4B br-zero:          {:>4} inst, {:>5} B total \
             (guard {}/{} B, non-guard {}/{} B, non-guard in c.beqz window {})",
            brfwd4_z_all.0 + brfwd4_z_guard.0,
            brfwd4_z_all.1 + brfwd4_z_guard.1,
            brfwd4_z_guard.0,
            brfwd4_z_guard.1,
            brfwd4_z_all.0,
            brfwd4_z_all.1,
            brfwd4_z_win
        );
        println!(
            "forward 4B j (non-guard):    {:>4} inst, {:>5} B (c.j → 2B each)",
            jfwd4_n, jfwd4_b
        );
        println!(
            "non-guard branch forms:      br {} inst/{} B; br-zero {} inst/{} B ({} × 2B)",
            ng_br.0, ng_br.1, ng_brz.0, ng_brz.1, ng_brz_2b
        );
        let mut hists: Vec<(i64, usize)> = li_hist.into_iter().collect();
        hists.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("top const/li immediates (remat candidates):");
        for (v, n) in hists.iter().take(12) {
            println!("  li {v:>6} × {n}");
        }

        // ---- rust-side denominators for the P1-G attribution table v2 ----
        // checked-profile / checked-source .text bins from the b1 emit
        // build (gitignored artifacts, regenerated by
        // hacdtestnet-b1:b1-scratch/build_emit.sh; byte-identical to the
        // B1 baselines per 60-b2 §Method). Printed when present so the v2
        // table is reproducible from this census alone; skipped otherwise.
        for name in ["prof", "src"] {
            let dir = std::env::var("RV64_B1_EMIT_DIR").unwrap_or_else(|_| {
                "/media/jji/data/repo/hacashdiamonds/hacdtestnet-b1/b1-scratch/out-emit"
                    .to_string()
            });
            let Ok(rcode) = std::fs::read(format!("{dir}/{name}.text.bin")) else {
                println!(
                    "\n(rust {name} .text bin not found under {dir}; \
                     b1 emit artifacts missing — rust-side v2 columns skipped)"
                );
                continue;
            };
            let rust = census_stream(&rcode, crate::calc::container::CALC_PROFILE_RV64IMC);
            println!("\n== rust {name} .text = {} B ==", rcode.len());
            let mut tn = 0;
            let mut tb = 0;
            for (k, n, b, c) in &rust {
                println!("{:<12} {:>5} {:>7} {:>5}", k, n, b, c);
                tn += n;
                tb += b;
            }
            println!("{:<12} {:>5} {:>7}", "TOTAL", tn, tb);
            assert_eq!(tb, rcode.len());
            // branch audit: far-forward branches (disp >= 100) — the
            // trap-stub-bound guard set clusters here — plus multi-inbound
            // branch targets (stub blocks have many inbound edges).
            let mut p = 0usize;
            let mut rows: Vec<(i64, usize, bool, usize, usize, usize, usize)> = Vec::new();
            let mut targets: std::collections::BTreeMap<usize, (usize, usize)> = Default::default();
            while p < rcode.len() {
                let (inst, len) =
                    decode_at(&rcode, p, crate::calc::container::CALC_PROFILE_RV64IMC).unwrap();
                if let Inst::Branch { rs2, target, .. } = &inst {
                    let abs = (p as i64 + *target) as usize;
                    let e = targets.entry(abs).or_default();
                    e.0 += 1;
                    e.1 += len as usize;
                    if *target >= 100 {
                        rows.push((
                            *target,
                            p,
                            *rs2 == 0,
                            len as usize,
                            abs,
                            e.0,
                            0,
                        ));
                    }
                }
                p += len as usize;
            }
            rows.sort_unstable();
            println!("far-forward branches (disp >= 100):");
            for (disp, off, z, len, abs, _, _) in &rows {
                println!(
                    "  @{off:04x} {} {len}B disp {disp:+>5} -> {abs:#05x}",
                    if *z { "br-zero" } else { "br" }
                );
            }
            println!("branch targets with >= 2 inbound edges:");
            for (t, (n, b)) in &targets {
                if *n >= 2 {
                    println!("  -> {t:#05x}: {n} inbound, {b} B");
                }
            }
        }
    }


    #[test]
    fn pa_repro() {
        // exact HIP5 nc-table shape (16 arms, keys 65..90, holes = default 0)
        const LETTERS: [(u64, u64); 16] = [
            (65, 7), (66, 6), (69, 1), (72, 8), (73, 8), (75, 8), (77, 2), (78, 6),
            (83, 6), (84, 8), (85, 3), (86, 8), (87, 3), (88, 5), (89, 3), (90, 10),
        ];
        let mut chain = String::new();
        for (k, v) in LETTERS {
            chain.push_str(&format!("if x == {k}u64 {{ sel = {v}u64 }}\n"));
        }
        for (x, want) in [(65u64, 7u64), (90, 10), (66, 6), (67, 0), (64, 0), (91, 0), (80, 0)] {
            // x declared FIRST so `var sel = 0` directly precedes the run
            // and the table path (not the chain fallback) is exercised
            let body = format!(
                "var x = {x}u64\nvar sel = 0u64\n{chain}return sel as bytes"
            );
            let out = run_e2e_mode(&body, &[], Rv64Mode::Im);
            println!("x={x} want={want} got={:?}", u64::from_be_bytes(out.as_slice().try_into().unwrap()));
            assert_eq!(out, want.to_be_bytes().to_vec(), "x={x}");
        }
    }


    #[test]
    fn deep_expr_probe() {
        // The register pool is a hard subset limit: depth <= 8 computes
        // correctly; deeper nesting is rejected at compile time (the old
        // spill path silently miscomputed at depth >= 10 — caught here).
        for depth in [7usize, 8] {
            let (body, want) = deep_add_body(depth);
            let out = run_e2e(&body, &[]);
            assert_eq!(out, want.to_be_bytes().to_vec(), "depth {depth}");
        }
        for depth in [9usize, 16] {
            let (body, _) = deep_add_body(depth);
            let (ir, _) = compile(&body);
            let err = lower_body(&ir).unwrap_err().to_string();
            assert!(err.contains("8-value register pool"), "depth {depth}: {err}");
        }
    }
