//! SCRATCH (P2-D04 reproducible builds and determinism audit): concurrent-determinism meta-test.
//!
//! The G3 promise = same input must yield the same output bytes. This test
//! upgrades the spot-check "two runs agree" into an in-process meta property
//! of "8 concurrent threads mixed with serial runs": the same fitsh body is
//! compiled by many threads at once (sharing in-process global state such as the
//! env switches FWD_DISABLED etc.); artifacts (ImcRemap = consensus profile 2)
//! must be byte-identical to the serial baseline.
//!
//! Covers three body classes: small arithmetic bodies, >4 KiB branch-relaxed
//! bodies (triggering the relaxation-ladder retry), and bank-pressured bodies
//! (exercising the WebWalk/allocate_bank ordering paths).

use std::sync::Arc;

use vm::calc::container::CalcContainer;
use vm::fitshc::{CompiledCode, compile_body};
use vm::ir::rv64::{ARENA_SIZE_MAX, Rv64Mode, lower_body_mode_arena};
use vm::lang::Tokenizer;
use vm::ValueTy;

fn pack(body: &str, mode: Rv64Mode) -> CalcContainer {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let args: Vec<(String, ValueTy)> = vec![("data".to_string(), ValueTy::Bytes)];
    let (ir, compiled, _) = compile_body(tokens, args, &[], &[], false).expect("compile body");
    assert!(matches!(compiled, CompiledCode::Bytecode(_)));
    let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).expect("lower");
    let cap = vm::calc::SpaceCap::new(0);
    let profile = match mode {
        Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
        _ => vm::calc::container::CALC_PROFILE_RV64IMC,
    };
    lo.pack_segments_profile(&cap, 64, 4800, 16, profile).expect("pack")
}

/// Consensus-profile (ImcRemap) container-byte serial baseline.
fn pack_remap_bytes(body: &str) -> Vec<u8> {
    pack(body, Rv64Mode::ImcRemap).encode().expect("encode")
}

fn big_relax_body() -> String {
    let mut body = String::from("var s = 0u64\nvar i = 0u64\nwhile i < 3 {\n");
    for k in 0..260 {
        body.push_str(&format!("if byte(data, 0) != {k} {{ s += {k} }}\n"));
    }
    body.push_str("i += 1\n}\nreturn s as bytes");
    body
}

fn bank_body() -> String {
    // Many highly-active slots: exercises web_plan / allocate_bank / frame_order ordering paths
    let mut body = String::new();
    for k in 0..12 {
        body.push_str(&format!("var v{k} = byte(data, {k}) as u64\n"));
    }
    body.push_str("var i = 0u64\nwhile i < 8 {\n");
    for k in 0..12 {
        body.push_str(&format!("v{k} = (v{k} * 31 + byte(data, (i + {k}) % 16)) % 1000003\n"));
    }
    body.push_str("i += 1\n}\n");
    let expr = (0..12).map(|k| format!("v{k}")).collect::<Vec<_>>().join(" + ");
    body.push_str(&format!("var r = {expr}\nreturn r as bytes"));
    body
}

const BODIES: [(&str, fn() -> String); 3] = [
    ("small_arith", || {
        "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\n\
         var b = 0u64\nvar j = 0u64\nwhile j < 8 { b = b * 256 + byte(data, j + 8)\nj += 1 }\n\
         var r = (a + b) * (a - b + 1)\nreturn r as bytes"
            .to_string()
    }),
    ("big_relax_4k7", big_relax_body),
    ("bank_pressure", bank_body),
];

#[test]
fn d04_eight_thread_concurrent_compile_is_byte_identical() {
    for (name, mk) in BODIES {
        let body = mk();
        // Serial baseline (established first, before any thread starts)
        let baseline = pack_remap_bytes(&body);
        let body = Arc::new(body);
        let baseline = Arc::new(baseline);
        let mut handles = Vec::new();
        for t in 0..8 {
            let body = Arc::clone(&body);
            let baseline = Arc::clone(&baseline);
            handles.push(std::thread::spawn(move || {
                let got = pack_remap_bytes(&body);
                assert_eq!(
                    got,
                    *baseline,
                    "thread {t}: concurrent compile diverged for {name}"
                );
                // Repeat once more inside each thread: repeated compiles under concurrency still agree
                let got2 = pack_remap_bytes(&body);
                assert_eq!(got2, *baseline, "thread {t}: repeat compile diverged for {name}");
            }));
        }
        for h in handles {
            h.join().expect("thread join");
        }
        // Serial re-check after the concurrency storm: shared env switches (FWD_DISABLED etc.) were not polluted
        let after = pack_remap_bytes(&body);
        assert_eq!(after, *baseline, "post-storm serial compile diverged for {name}");
    }
}

#[test]
fn d04_all_modes_concurrent_agree_serial() {
    // All three modes agree with their own serial baselines under concurrency (Imc and Im covered too)
    let body = bank_body();
    for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        let base = pack(&body, mode).encode().expect("encode");
        let base = Arc::new(base);
        let body = Arc::new(body.clone());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let body = Arc::clone(&body);
            let base = Arc::clone(&base);
            handles.push(std::thread::spawn(move || {
                assert_eq!(pack(&body, mode).encode().unwrap(), *base);
            }));
        }
        for h in handles {
            h.join().expect("join");
        }
    }
}
