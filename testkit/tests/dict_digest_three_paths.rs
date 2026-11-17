//! Neutral three-path comparison on one workload: a `{k=v,...}` flat-kv
//! document parsed, walked (`start` → value → next key … until `end`) and
//! digested (sha256 over the concatenated values).
//!
//! Paths, all running the identical semantics on identical inputs:
//!   1. **fitsh HVM** — `vm/tests/fixtures/dictdigest/dict_digest.fitsh`:
//!      the native `ascii_parse_flat_kv` (flags=0 grammar, LOWER keys /
//!      ALNUM values) + map ITEMGET walk opcodes + native `sha2`. Compiled
//!      by fitshc, deployed through the real `ContractDeploy` action,
//!      invoked via one CALLEXT per document. This is the shape an HVM
//!      author would write: natives where natives exist, opcodes for the
//!      walk.
//!   2. **calc interp** — the RV64IM `sha256kernel` fixture
//!      (`vm/tests/fixtures/sha256kernel`, dict-walk mode): the same parse,
//!      walk and sha256 in pure machine code, interpreter-executed.
//!   3. **calc AOT** — the same container translated to x86-64 and run
//!      natively (mountable, non-consensus; requires `--features
//!      calc-aot`).
//!
//! Neutrality rules: every path is cross-validated against an independent
//! Rust reference (BTreeMap parse + walk, `vm::calc::run_hash` for the final
//! digest) before any comparison; timing and gas are reported, never
//! asserted against each other.
//!
//! Adversarial corpus: the `dict_digest_adversarial_corpus_three_path_alignment`
//! test drives a byte-exact corpus of illegal and boundary documents through
//! all mounted paths and asserts the success/failure binary split agrees
//! three ways, with byte-equal digests on the success face and the pinned
//! per-path failure mapping (kernel Ok-empty / fitsh empty-Ok vs call-abort)
//! on the failure face. Real cross-path divergences found while building the
//! corpus are pinned as `#[ignore]` `dict_divergence_*` tests (run with
//! `-- --ignored`); they document observed behavior and are never silently
//! folded into the alignment face.
//!
//! Run:
//!   cargo test -p testkit --test dict_digest_three_paths -- --nocapture
//!   cargo test -p testkit --features calc-aot --test dict_digest_three_paths -- --nocapture
//!   cargo test -p testkit --test dict_digest_three_paths -- --ignored --nocapture

use std::sync::Arc;
use std::time::Instant;

use app::standard_registry_with_params;
use base::{
    ActionExecute, ExecutionServices, VmGasUsage, hac_add, init_block_contract_storage_budget,
};
use field::{Address, Amount, Uint4, UNIT_238};
use testkit::sim::context::TestContext;
use testkit::sim::tx::StubTxBuilder;
use vm::action::{ContractDeploy, contract_deploy_charge_bytes, contract_protocol_cost_min};
use vm::calc::{
    pack_container, run_hash, CalcFnObj, CalcRunLimits, HashKind, SpaceCap, CALC_PROFILE_RV64IM,
    CYCLES_PER_COMPUTE,
};
use vm::fitshc::compiler;
use vm::contract::ContractSto;
use vm::{CodeType, ContractAddress, Value, build_call_codes, main_call};

// ---------------------------------------------------------------------------
// independent Rust reference (the neutrality anchor)
// ---------------------------------------------------------------------------

/// Host-side parse+walk mirroring the flags=0 grammar of
/// `ascii_parse_flat_kv` (keys `a-z`, values alphanumeric, no whitespace,
/// distinct delimiters, no duplicate keys) and the kernel walk (start at
/// `start`, values name the next key, `end` terminates and is appended,
/// 64-step cap). Returns the concatenated values, or `None` on any error —
/// exactly the success/failure split the kernel expresses as 32B/empty.
fn ref_walk(doc: &[u8]) -> Option<Vec<u8>> {
    let lower = |c: u8| c.is_ascii_lowercase();
    let alnum = |c: u8| c.is_ascii_alphanumeric();
    if doc.first() != Some(&b'{') {
        return None;
    }
    let mut map = std::collections::BTreeMap::new();
    let mut i = 1;
    loop {
        if i >= doc.len() || doc[i] == b'}' {
            i += 1;
            break;
        }
        let k0 = i;
        while i < doc.len() && lower(doc[i]) {
            i += 1;
        }
        if i == k0 || i >= doc.len() || doc[i] != b'=' {
            return None;
        }
        let key = &doc[k0..i];
        i += 1;
        let v0 = i;
        while i < doc.len() && alnum(doc[i]) {
            i += 1;
        }
        let val = &doc[v0..i];
        if v0 == i || i >= doc.len() || doc[i] != b',' && doc[i] != b'}' {
            return None;
        }
        if map.insert(key.to_vec(), val.to_vec()).is_some() {
            return None; // duplicate key
        }
        if doc[i] == b',' {
            i += 1;
        }
    }
    if i != doc.len() || map.is_empty() {
        return None;
    }
    let mut acc = Vec::new();
    let mut key = b"start".to_vec();
    for _ in 0..64 {
        let val = map.get(&key)?;
        acc.extend_from_slice(val);
        if val.as_slice() == b"end" {
            return Some(acc);
        }
        key = val.clone();
    }
    None // walk exceeded the 64-step cap
}

fn ref_digest(acc: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    run_hash(HashKind::Sha2, acc, &mut out);
    out
}

/// A chain of `n` single-letter keys: start→a→b→…→end, plus two filler
/// pairs the walk never visits.
fn chain_doc(n: usize) -> Vec<u8> {
    assert!(n >= 1 && n <= 24, "single-letter keys a..x");
    let mut doc = b"{start=a".to_vec();
    for i in 0..n - 1 {
        doc.push(b',');
        doc.push(b'a' + i as u8);
        doc.push(b'=');
        doc.push(b'b' + i as u8);
    }
    let last = b'a' + n as u8 - 1;
    doc.extend_from_slice(&format!(",{}=end,other=42,zz=9}}", last as char).into_bytes());
    doc
}

fn chain_acc(n: usize) -> Vec<u8> {
    let mut acc = Vec::new();
    for i in 0..n {
        acc.push(b'a' + i as u8);
    }
    acc.extend_from_slice(b"end");
    acc
}

/// Valid docs paired with the acc the walk must digest, every one
/// cross-checked against the reference before use.
fn checked_docs() -> Vec<(Vec<u8>, [u8; 32])> {
    let mut docs = vec![
        (
            b"{start=beta,beta=gamma,gamma=end,other=42,zz=9}".to_vec(),
            b"betagammaend".to_vec(),
        ),
        (b"{start=end}".to_vec(), b"end".to_vec()),
        (b"{zz=1,start=aa,aa=bb,bb=end}".to_vec(), b"aabbend".to_vec()),
    ];
    for n in [1usize, 4, 12, 24] {
        docs.push((chain_doc(n), chain_acc(n)));
    }
    docs.into_iter()
        .map(|(doc, acc)| {
            let walked = ref_walk(&doc).expect("reference walks every valid doc");
            assert_eq!(walked, acc, "reference acc for {doc:?}");
            let digest = ref_digest(&acc);
            (doc, digest)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// path harnesses
// ---------------------------------------------------------------------------

const KERNEL_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vm/tests/fixtures/sha256kernel");
const FITSH_SRC: &str = include_str!("../../vm/tests/fixtures/dictdigest/dict_digest.fitsh");

fn limits() -> CalcRunLimits {
    CalcRunLimits {
        cycles: 1_000_000, // the per-call consensus cycle cap
        resource: i64::MAX,
        deadline: None,
    }
}

fn kernel_obj(c: vm::calc::container::CalcContainer, aot: bool) -> CalcFnObj {
    CalcFnObj::build_with_host(c, vm::calc::CalcHostConfig::new().with_aot_mount(aot))
        .expect("kernel container builds")
}

fn kernel_container() -> vm::calc::container::CalcContainer {
    let read = |n: &str| std::fs::read(format!("{KERNEL_DIR}/{n}")).expect("fixture image");
    let bss_len = u16::from_str_radix(
        std::fs::read_to_string(format!("{KERNEL_DIR}/bss_len.txt"))
            .expect("bss_len.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("bss_len") as u32;
    let tmp = vm::calc::container::CalcContainer::from_regions(
        CALC_PROFILE_RV64IM,
        // 10 KiB: guard band + minimum stack above the 0x400 BSS no longer
        // fit in 8 KiB. Same headroom as vm/tests/sha256kernel_fixture.rs.
        10,
        vm::calc::CALC_CODE_BASE as u32,
        read("image_code.bin"),
        read("image_rodata.bin"),
        read("image_init.bin"),
        256,
        32,
    )
    .with_bss(bss_len);
    pack_container(
        &SpaceCap::new(0),
        tmp.profile,
        tmp.mem_kib,
        tmp.entry,
        tmp.input_max,
        tmp.output_max,
        tmp.segs,
    )
    .expect("kernel container packs")
}

fn kernel_fn() -> CalcFnObj {
    kernel_obj(kernel_container(), false)
}

#[cfg(feature = "calc-aot")]
fn kernel_fn_aot() -> CalcFnObj {
    kernel_obj(kernel_container(), true)
}

fn deploy_dict_digest() -> TestContext {
    let (contract, _, _, _) = compiler::compile(FITSH_SRC).expect("dict_digest.fitsh compiles");
    let sto: ContractSto = contract.into_sto();

    let main_addr = {
        let mut bytes = [0u8; Address::SIZE];
        bytes[20] = 7;
        Address::from(bytes)
    };
    let nonce = Uint4::from(1);
    let caddr = ContractAddress::calculate(&main_addr, &nonce);
    // Timing repeats the same CALLEXT hundreds of times: mainnet per-block
    // resource buckets (6,100) would trip on iteration two. The unlimited
    // limit-byte config only lifts the bucket ceiling; charged gas numbers
    // are unaffected.
    let mut hp = hacash_params::MAINNET_PARAMS;
    hp.protocol.vm.compute_limit_byte = 0;
    hp.protocol.vm.resource_limit_byte = 0;
    hp.protocol.vm.storage_limit_byte = 0;
    let hp: &'static hacash_params::HacashParams = Box::leak(Box::new(hp));
    let services: Arc<dyn ExecutionServices> = Arc::new(standard_registry_with_params(hp).unwrap());
    let tx = StubTxBuilder::new()
        .ty(3)
        .main(main_addr)
        .addrs(vec![main_addr, caddr.to_addr()])
        .build();
    let mut ctx = TestContext::new(services.clone(), Arc::new(tx));
    ctx.set_vm(
        services
            .assign_vm(ctx.env.block.height)
            .expect("registry assigns a vm"),
    );
    hac_add(
        &mut ctx,
        &main_addr,
        &Amount::coin_u128(10_000_000_000_000, UNIT_238),
    )
    .unwrap();
    let params = *services.vm_params().unwrap();
    init_block_contract_storage_budget(&mut ctx.layer, &params, ctx.env.block.height).unwrap();

    let charge = contract_deploy_charge_bytes(&sto);
    let cost = contract_protocol_cost_min(&ctx, charge, 10_000).unwrap();
    let mut act = ContractDeploy::new();
    act.nonce = nonce;
    act.protocol_cost = cost;
    act.contract = sto;
    act.execute(&mut ctx).expect("dict_digest contract deploys");
    ctx
}

fn fitsh_call(ctx: &mut TestContext, doc: &[u8]) -> Result<(VmGasUsage, Vec<u8>), String> {
    let codes: Arc<[u8]> = build_call_codes("dict_digest", &[Value::Bytes(doc.to_vec())])
        .map_err(|e| e.to_string())?
        .into();
    match main_call(ctx, CodeType::Bytecode, codes) {
        Ok((gas, Value::Bytes(out))) => Ok((gas, out)),
        Ok((_gas, other)) => Err(format!("non-bytes return {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// the comparison
// ---------------------------------------------------------------------------

#[test]
fn dict_digest_fitsh_vs_calc_interp_cross_validation() {
    let docs = checked_docs();

    // ---- calc interpreter vs reference
    let kernel = kernel_fn();
    for (doc, digest) in &docs {
        let e = kernel.run(doc, limits());
        let out = e.result.expect("kernel dict mode succeeds");
        assert_eq!(&out[..], &digest[..], "interp digest for {doc:?}");
    }

    // ---- fitsh HVM vs reference (real deploy + CALLEXT)
    let mut ctx = deploy_dict_digest();
    for (doc, digest) in &docs {
        let (_gas, out) = fitsh_call(&mut ctx, doc)
            .unwrap_or_else(|e| panic!("fitsh call for {doc:?}: {e}"));
        assert_eq!(&out[..], &digest[..], "fitsh digest for {doc:?}");
    }
}

#[test]
fn dict_digest_error_paths_agree() {
    let kernel = kernel_fn();
    let mut ctx = deploy_dict_digest();
    for doc in [
        &b"{start=beta}"[..],             // broken chain: no beta key
        b"{start=beta,beta=end,start=1}", // duplicate key
        b"{Start=beta,beta=end}",         // key class violation
        b"{start=be ta,beta=end}",        // value class violation
        b"{start=beta,beta=end",          // missing close
        b"{start=beta,beta=end}x",        // trailing bytes
        b"{}",                            // empty doc
    ] {
        // kernel: empty output (len 0), never a VM-level abort
        let e = kernel.run(doc, limits());
        let out = e.result.unwrap_or_default();
        assert!(out.is_empty(), "kernel must yield empty for {doc:?}, got {out:?}");

        // fitsh: grammar errors return empty bytes (the errno-checked
        // contract), the broken chain aborts the call (CompoNoFindItem) —
        // both are failures, one as an error value, one as a runtime error.
        match fitsh_call(&mut ctx, doc) {
            Ok((_gas, out)) => {
                assert!(out.is_empty(), "fitsh error doc must be empty: {doc:?}")
            }
            Err(_) => { /* runtime-error channel: also a failure */ }
        }
    }
}

#[cfg(feature = "calc-aot")]
#[test]
fn dict_digest_calc_aot_parity() {
    let plain = kernel_fn();
    let mounted = kernel_fn_aot();
    for (doc, digest) in checked_docs() {
        let e_i = plain.run(&doc, limits());
        let e_a = mounted.run(&doc, limits());
        assert_eq!(e_i.result, e_a.result, "AOT output parity on {doc:?}");
        assert_eq!(&e_a.result.unwrap()[..], &digest[..], "AOT digest for {doc:?}");
        assert_eq!(
            e_i.cycles_used, e_a.cycles_used,
            "AOT cycle parity on {doc:?}"
        );
    }
}

#[test]
fn dict_digest_timing_and_gas_report() {
    let docs = checked_docs();
    let (bench_doc, digest) = &docs[3]; // the 4-step chain — mid-size representative
    let acc = ref_walk(bench_doc).unwrap();

    // ---- calc interpreter
    let kernel = kernel_fn();
    let e = kernel.run(bench_doc, limits());
    assert_eq!(&e.result.unwrap()[..], &digest[..]);
    let interp_cycles = e.cycles_used;
    let interp_compute_charge = interp_cycles.div_ceil(CYCLES_PER_COMPUTE as u64) as i64;

    const ITERS_INTERP: u32 = 1_000;
    let t = Instant::now();
    for _ in 0..ITERS_INTERP {
        let e = kernel.run(bench_doc, limits());
        assert!(e.result.is_ok());
    }
    let interp_ns = t.elapsed().as_nanos() as f64 / ITERS_INTERP as f64;

    // ---- fitsh HVM (deploy once, CALLEXT per call)
    let mut ctx = deploy_dict_digest();
    let (fitsh_gas, out) = fitsh_call(&mut ctx, bench_doc).expect("fitsh bench call");
    assert_eq!(&out[..], &digest[..]);
    const ITERS_FITSH: u32 = 200;
    let t = Instant::now();
    for _ in 0..ITERS_FITSH {
        let (_gas, out) = fitsh_call(&mut ctx, bench_doc).expect("fitsh bench call");
        let _ = out;
    }
    let fitsh_ns = t.elapsed().as_nanos() as f64 / ITERS_FITSH as f64;

    println!(
        "\n== dict_digest three-path report, debug-profile timings (doc {} B, acc {} B, 1 compute = {CYCLES_PER_COMPUTE} cycles) ==",
        bench_doc.len(),
        acc.len()
    );
    println!("calc interp : {interp_cycles:>7} cycles  ~{interp_compute_charge:>5} compute           {interp_ns:>10.0} ns/call");
    println!(
        "fitsh  HVM  : {:>7} compute {:>5} resource {:>5} storage   {fitsh_ns:>10.0} ns/call",
        fitsh_gas.compute, fitsh_gas.resource, fitsh_gas.storage
    );

    // ---- calc AOT (only when the backend is mounted)
    #[cfg(feature = "calc-aot")]
    {
        let mounted = kernel_fn_aot();
        let e = mounted.run(bench_doc, limits());
        assert_eq!(&e.result.unwrap()[..], &digest[..]);
        assert_eq!(
            e.cycles_used, interp_cycles,
            "AOT cycle accounting must equal the interpreter"
        );
        const ITERS_AOT: u32 = 20_000;
        let t = Instant::now();
        for _ in 0..ITERS_AOT {
            let e = mounted.run(bench_doc, limits());
            assert!(e.result.is_ok());
        }
        let aot_ns = t.elapsed().as_nanos() as f64 / ITERS_AOT as f64;
        println!("calc AOT    : {interp_cycles:>7} cycles  (identical charge)       {aot_ns:>10.0} ns/call");
        println!(
            "speedup: fitsh/interp {:.1}x, fitsh/AOT {:.1}x, interp/AOT {:.1}x",
            fitsh_ns / interp_ns,
            fitsh_ns / aot_ns,
            interp_ns / aot_ns
        );
    }
    #[cfg(not(feature = "calc-aot"))]
    println!("calc AOT    : (mount with --features calc-aot)");
}

// ===========================================================================
// adversarial corpus (byte-exact) + three-path success/failure alignment
// ===========================================================================
//
// Capacity landscape the corpus is designed against (read off the three
// implementations, not assumed):
//   golden `ref_walk`     : no pair cap, no acc cap, no doc-length cap
//   fitsh HVM (flags=0)   : the doc itself is one call-arg `Bytes`, so the
//                           `SpaceCap::value_size` (1280) window binds; the
//                           native parse caps pairs at `compo_length` (128)
//                           and dup-keys at `ASCII_ERR_DUP_KEY`; the walk
//                           caps at 64 lookups and the acc is a string
//                           (≤ 1280 through `++`).
//   calc kernel fixture   : container input window (256B on the consensus-
//                           shaped container; a test-side wide container
//                           reaches deeper for boundary work), MAX_PAIRS=64,
//                           MAX_ACC=512, 64-lookup step cap.
// Duplicate-key semantics (confirmed against the implementations, not the
// task text): the native returns `ASCII_ERR_DUP_KEY`, the kernel explicitly
// scans for duplicates and refuses, the golden returns None — a duplicate
// key is an ERROR everywhere, never last-one-wins.
//
// Docs that fall outside one path's capacity window while remaining inside
// the others' are REAL divergences: they are pinned in the `#[ignore]`
// `dict_divergence_*` tests below, never hidden from the report.

/// The consensus-shaped container's input window (`kernel_container`, above).
const STD_INPUT_MAX: u16 = 256;
/// Test-side packing choice (the fixture semantics are input-window
/// independent) so the walk/parse boundaries beyond the 256B call-data
/// window — 64-pair MAX_PAIRS edge, MAX_ACC=512 edge, 1280B fitsh doc-arg
/// edge — can be exercised on the calc paths at all.
const WIDE_INPUT_MAX: u16 = 4096;

/// Pinned fitsh failure form. The contract collapses every parse errno and
/// every 64-step-cap miss (`done == false`) into an empty-bytes return; only
/// a missing ITEMGET key (`start` absent or a chain link naming no key)
/// aborts the CALLEXT itself. Two structurally different failure channels,
/// pinned per corpus entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FitshFail {
    /// contract returns Ok(empty bytes)
    EmptyOk,
    /// the CALLEXT aborts (runtime-error channel)
    CallErr,
}

enum Expect {
    /// walk succeeds; carries the exact acc the golden must concatenate
    Success(Vec<u8>),
    Fail { fitsh: FitshFail },
}

struct Case {
    name: &'static str,
    tier: &'static str,
    doc: Vec<u8>,
    expect: Expect,
}

fn succ(name: &'static str, tier: &'static str, doc: Vec<u8>, acc: Vec<u8>) -> Case {
    Case { name, tier, doc, expect: Expect::Success(acc) }
}

fn fail(name: &'static str, tier: &'static str, doc: Vec<u8>, fitsh: FitshFail) -> Case {
    Case { name, tier, doc, expect: Expect::Fail { fitsh } }
}

/// `{k=v,...}` from (key, value) pairs — the corpus serializer; docs stay
/// byte-exact because every pair is spelled out or built by a named helper.
fn pairs_doc(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut d = String::from("{");
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            d.push(',');
        }
        d.push_str(k);
        d.push('=');
        d.push_str(v);
    }
    d.push('}');
    d.into_bytes()
}

/// Bijective base-26 lowercase key names: a..z, aa, ab, … Distinct by
/// construction; never collides with `start`/`end` below i=702 (both are
/// 3+ letters and the 3-letter space starts at "aaa").
fn chain_key(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'a' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    s.into_iter().collect()
}

/// `{start=end}` plus `n_fillers` distinct filler pairs the walk never
/// visits. Filler keys run a..z then aa.. (values "0", alnum-legal).
fn start_end_plus_fillers(n_fillers: usize) -> Vec<u8> {
    let mut pairs: Vec<(String, String)> = vec![("start".to_string(), "end".to_string())];
    for i in 0..n_fillers {
        pairs.push((chain_key(i), "0".to_string()));
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    pairs_doc(&refs)
}

/// n-lookup chain document: start→k1→…→k_{n-1}→"end", spelled with EXACTLY
/// n pairs and no fillers — at n=64 that is simultaneously the kernel
/// MAX_PAIRS edge (a 65th pair would conflate the pair cap with the step
/// cap) and the 64-lookup step-cap edge. Returns (doc, acc).
fn walk_chain_doc(n: usize) -> (Vec<u8>, Vec<u8>) {
    assert!(1 <= n && n <= 130, "chain length range");
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(n);
    let mut acc = Vec::new();
    for i in 0..n {
        let key = if i == 0 { "start".to_string() } else { chain_key(i - 1) };
        let val = if i + 1 == n { "end".to_string() } else { chain_key(i) };
        acc.extend_from_slice(val.as_bytes());
        pairs.push((key, val));
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    (pairs_doc(&refs), acc)
}

/// Document whose walked acc length is exactly `acc_len` (the kernel
/// MAX_ACC = 512 edge): a chain start→K0→K1→…→Km→end where each visited
/// value is the next key (so every rung length counts once into the acc),
/// built from 72-char rungs — short enough that ITEMGET lookups stay inside
/// the fitsh kv-key window — plus one tail rung that lands the total
/// exactly. Rungs use distinct repeated letters, so all keys are distinct.
fn acc_boundary_doc(acc_len: usize) -> (Vec<u8>, Vec<u8>) {
    const RUNG: usize = 72;
    assert!(acc_len >= RUNG + 4 && acc_len <= 1280, "rung-doc reach");
    let rest = acc_len - 3; // the final "end"
    let full = (rest - 1) / RUNG; // number of full 72-char rungs (≥ 1)
    let tail = rest - full * RUNG; // last rung length, 1..=RUNG
    assert!(tail <= 128, "tail rung must stay a legal ITEMGET key length");
    let rungs = full + 1;
    let name = |r: usize| -> String {
        let len = if r + 1 == rungs { tail } else { RUNG };
        std::iter::repeat((b'a' + r as u8) as char).take(len).collect()
    };
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(rungs + 1);
    let mut acc = Vec::new();
    pairs.push(("start".to_string(), name(0)));
    acc.extend_from_slice(name(0).as_bytes());
    for r in 0..rungs {
        if r + 1 == rungs {
            pairs.push((name(r), "end".to_string()));
        } else {
            pairs.push((name(r), name(r + 1)));
            acc.extend_from_slice(name(r + 1).as_bytes());
        }
    }
    acc.extend_from_slice(b"end");
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    (pairs_doc(&refs), acc)
}

/// The full adversarial corpus. Grammar/`walk`/`dup` entries run on the
/// consensus-shaped container; entries longer than the 256B input window
/// (`boundary`/`length` tiers and the 65/100 chains) run on the wide
/// container — the driver picks per doc length and counts both.
fn adversarial_corpus() -> Vec<Case> {
    let mut v: Vec<Case> = Vec::new();

    // ---- syntactically illegal: all start with '{'. Inputs not starting
    //      with '{' never enter the kernel's dict mode at all (they fall to
    //      the bare sha256 branch) — that is a pinned divergence, outside the
    //      alignment face.
    let e = FitshFail::EmptyOk;
    v.push(fail("missing_close", "grammar", b"{start=beta,beta=end".to_vec(), e));
    v.push(fail("missing_kv_sep", "grammar", b"{ab=start,cd}".to_vec(), e));
    v.push(fail("empty_key", "grammar", b"{=v}".to_vec(), e));
    v.push(fail("empty_value", "grammar", b"{a=}".to_vec(), e));
    v.push(fail("empty_pair", "grammar", b"{a=b,,c=d}".to_vec(), e));
    v.push(fail("upper_key", "grammar", b"{Start=beta,beta=end}".to_vec(), e));
    v.push(fail("symbol_key", "grammar", b"{a$b=c}".to_vec(), e));
    v.push(fail("space_key", "grammar", b"{a =b}".to_vec(), e));
    v.push(fail("space_value", "grammar", b"{a=b c}".to_vec(), e));
    v.push(fail("space_lead", "grammar", b"{ start=a}".to_vec(), e));
    v.push(fail("symbol_value", "grammar", b"{a=b-c}".to_vec(), e));
    v.push(fail("delim_mix_semi", "grammar", b"{a=b;c=d}".to_vec(), e));
    v.push(fail("delim_mix_eq", "grammar", b"{a=b=c}".to_vec(), e));
    v.push(fail("delim_mix_eqempty", "grammar", b"{a==b}".to_vec(), e));
    v.push(fail("trailing_bytes", "grammar", b"{a=b}x".to_vec(), e));
    v.push(fail("double_close", "grammar", b"{a=b}}".to_vec(), e));
    v.push(fail("empty_doc", "grammar", b"{}".to_vec(), e));
    v.push(fail("comma_only", "grammar", b"{,}".to_vec(), e));
    v.push(fail("brace_only", "grammar", b"{".to_vec(), e));
    v.push(fail("high_byte_key", "grammar", b"{a\xC3=b}".to_vec(), e));
    v.push(fail("del_value", "grammar", b"{a=b\x7F}".to_vec(), e));

    // ---- walk-chain semantic edges: broken chain / missing start → fitsh takes the ITEMGET runtime-error channel
    let c = FitshFail::CallErr;
    v.push(fail("start_missing", "walk", b"{a=1}".to_vec(), c));
    v.push(fail("broken_chain", "walk", b"{start=beta}".to_vec(), c));
    // "end" must match as a whole value: endx is just another broken-chain key
    v.push(fail("chain_prefix_end", "walk", b"{start=endx}".to_vec(), c));
    // values are ALNUM (digits included), keys are LOWER: a numeric value cannot be the next hop key
    v.push(fail("digit_value_key", "walk", b"{start=0}".to_vec(), c));

    // ---- walk-chain cap edges: cycles / self-loops / the 65-long chain.
    //      All fail via the 64-lookup cap: fitsh collapses to an empty return
    //      (EmptyOk), the kernel returns Ok(empty), golden None. cycle_two
    //      lands exactly on the cap: the 64th lookup still runs, the 65th is
    //      rejected.
    v.push(fail("cap_64_cycle_two", "walk-cap", b"{start=a,a=b,b=a}".to_vec(), e));
    v.push(fail("self_loop", "walk-cap", b"{start=a,a=a}".to_vec(), e));
    v.push(fail("self_loop_start", "walk-cap", b"{start=start}".to_vec(), e));
    let (d, _) = walk_chain_doc(65); // kernel rejects via MAX_PAIRS (65 pairs); fitsh/golden reject via the 64-step cap — the binary split agrees
    v.push(fail("chain_65_step_cap", "walk-cap", d, e));
    let (d, _) = walk_chain_doc(100); // the 100-key tier (failure side)
    v.push(fail("chain_100_over_caps", "walk-cap", d, e));

    // ---- duplicate keys: semantics pinned — native ASCII_ERR_DUP_KEY /
    //      kernel explicit dedup / golden None; all three report an error and
    //      never let the later key overwrite. Duplicates off the chain are
    //      rejected the same way (the walk never even reaches that key pair).
    v.push(fail("dup_on_chain", "dup", b"{start=beta,beta=end,start=1}".to_vec(), e));
    v.push(fail("dup_off_chain", "dup", b"{a=1,a=2,start=end}".to_vec(), e));
    v.push(fail("dup_diff_len", "dup", b"{a=x,a=yy,start=end}".to_vec(), e));

    // ---- size tiers (success side): 54 pairs hug the 256B input window; 62 pairs <= the kernel MAX_PAIRS
    let d54 = start_end_plus_fillers(53);
    assert!(d54.len() <= STD_INPUT_MAX as usize, "54-pair doc must fit the std input window, got {}", d54.len());
    v.push(succ("pairs_54_near_input_cap", "scale", d54, b"end".to_vec()));

    // ---- success-face corpus (including pre-existing documents, all re-verified against golden)
    v.push(succ("basic", "walk", b"{start=beta,beta=gamma,gamma=end,other=42,zz=9}".to_vec(), b"betagammaend".to_vec()));
    v.push(succ("start_eq_end", "walk", b"{start=end}".to_vec(), b"end".to_vec()));
    v.push(succ("start_not_first", "walk", b"{zz=1,start=aa,aa=bb,bb=end}".to_vec(), b"aabbend".to_vec()));
    for n in [1usize, 4, 12, 24] {
        v.push(succ("chain_chain_doc", "walk", chain_doc(n), chain_acc(n)));
    }
    // shortest value tier: single-character values
    v.push(succ("chain_shortest_values", "walk", b"{start=a,a=end}".to_vec(), b"aend".to_vec()));
    v.push(succ("multi_char_keys", "walk", b"{start=alpha,alpha=beta,beta=end,other=42}".to_vec(), b"alphabetaend".to_vec()));
    // "end" may also be a key: the walk terminates on reaching the value "end" at start; later pairs are not visited
    v.push(succ("end_as_key_unvisited", "walk", b"{start=end,end=zz,zz=1}".to_vec(), b"end".to_vec()));

    // ---- boundary tier (> 256B, wide container): 64 pairs = exactly
    //      MAX_PAIRS, 64 lookups = exactly the step cap; acc = exactly
    //      MAX_ACC(512)
    let (d, a) = walk_chain_doc(64);
    v.push(succ("chain_64_exact", "boundary", d, a));
    let (d, a) = acc_boundary_doc(512);
    v.push(succ("acc_512_exact", "boundary", d, a));

    // ---- length tier (> 256B, wide container): longest/shortest alnum
    //      values; the fitsh call argument (doc) is exactly value_size(1280).
    //      The next tier, 1281B, is a pinned divergence.
    let v600 = "x".repeat(600);
    let dlv = pairs_doc(&[("start", "end"), ("big", v600.as_str())]);
    v.push(succ("long_unvisited_value", "length", dlv, b"end".to_vec()));
    let big_key: String = "k".repeat(600);
    let dlk = pairs_doc(&[(big_key.as_str(), "0"), ("start", "end")]);
    v.push(succ("long_unvisited_key", "length", dlk, b"end".to_vec()));
    let d1280 = format!("{{start=end,big={}}}", "x".repeat(1264)).into_bytes();
    assert_eq!(d1280.len(), 1280);
    v.push(succ("doc_arg_1280_exact", "length", d1280, b"end".to_vec()));

    v
}

#[derive(Default)]
struct AlignmentStats {
    docs: usize,
    success: usize,
    fail: usize,
    std_container: usize,
    wide_container: usize,
    fitsh_empty_ok: usize,
    fitsh_call_err: usize,
    aot_checked: usize,
    agree: usize,
    tiers: std::collections::BTreeMap<&'static str, usize>,
}

/// Three-path alignment over the adversarial corpus. For every doc:
///   * golden `ref_walk` outcome is cross-checked against the corpus table;
///   * the success/failure binary split must agree across calc interp, calc
///     AOT (when mounted) and fitsh HVM;
///   * on the success face all digests are byte-equal to the golden digest
///     (a mismatch panics printing the document bytes);
///   * on the failure face the golden sync holds in both directions —
///     golden None ⟺ kernel Ok(empty) ⟺ fitsh failure, with the per-doc
///     pinned fitsh failure form (empty-Ok vs call-abort).
#[test]
fn dict_digest_adversarial_corpus_three_path_alignment() {
    let corpus = adversarial_corpus();
    let kernel_std = kernel_fn();
    let kernel_wide = kernel_fn_wide();
    #[cfg(feature = "calc-aot")]
    let aot_std = kernel_fn_aot();
    #[cfg(feature = "calc-aot")]
    let aot_wide = kernel_fn_wide_aot();
    let mut ctx = deploy_dict_digest();
    let mut st = AlignmentStats::default();

    for case in &corpus {
        let doc = &case.doc;
        st.docs += 1;
        *st.tiers.entry(case.tier).or_insert(0) += 1;

        // ---- golden (independent Rust reference), cross-checked vs table
        let ref_out = ref_walk(doc);
        let want_digest = match (&case.expect, &ref_out) {
            (Expect::Success(acc), Some(walked)) => {
                assert_eq!(walked, acc, "golden acc mismatch on {}: doc bytes {doc:?}", case.name);
                ref_digest(walked)
            }
            (Expect::Fail { .. }, None) => {
                st.fail += 1;
                [0u8; 32] // unused on the failure face
            }
            (Expect::Success(_), None) => {
                panic!("golden refuses a corpus success doc {}: {doc:?}", case.name)
            }
            (Expect::Fail { .. }, Some(_)) => {
                panic!("golden walks a corpus failure doc {}: {doc:?}", case.name)
            }
        };

        // ---- calc paths (container chosen by the doc's input window)
        let wide = doc.len() > STD_INPUT_MAX as usize;
        if wide {
            st.wide_container += 1;
        } else {
            st.std_container += 1;
        }
        let kernel = if wide { &kernel_wide } else { &kernel_std };
        let e_i = kernel.run(doc, limits());
        let i_success = matches!(&e_i.result, Ok(out) if out.len() == 32);

        // ---- fitsh path (real deploy + CALLEXT)
        let f_res = fitsh_call(&mut ctx, doc);
        let f_success = matches!(&f_res, Ok((_, out)) if out.len() == 32);

        // ---- AOT path (same container as the interpreter)
        #[cfg(feature = "calc-aot")]
        let a_success = {
            let aot = if wide { &aot_wide } else { &aot_std };
            let e_a = aot.run(doc, limits());
            assert_eq!(
                e_i.result, e_a.result,
                "AOT result parity on {}: doc bytes {doc:?}",
                case.name
            );
            assert_eq!(
                e_i.cycles_used, e_a.cycles_used,
                "AOT cycle parity on {}: doc bytes {doc:?}",
                case.name
            );
            st.aot_checked += 1;
            matches!(&e_a.result, Ok(out) if out.len() == 32)
        };
        #[cfg(not(feature = "calc-aot"))]
        let a_success = i_success;

        // ---- success/failure binary split must agree three ways
        if i_success == f_success && i_success == a_success {
            st.agree += 1;
        }
        assert!(
            i_success == f_success && i_success == a_success,
            "SPLIT DIVERGENCE on {} (interp={i_success}, aot={a_success}, fitsh={f_success}): doc bytes {doc:?}",
            case.name
        );

        if i_success {
            st.success += 1;
            // ---- success face: digests byte-equal, panic prints doc bytes
            let d_i = e_i.result.as_ref().unwrap();
            assert_eq!(
                d_i.as_slice(),
                &want_digest[..],
                "DIGEST MISMATCH interp vs golden on {}: doc bytes {doc:?}",
                case.name
            );
            let (_, d_f) = f_res.as_ref().unwrap_or_else(|_| panic!("fitsh lost a success doc {}: doc {doc:?}", case.name));
            assert_eq!(
                d_f.as_slice(),
                &want_digest[..],
                "DIGEST MISMATCH fitsh vs golden on {}: doc bytes {doc:?}",
                case.name
            );
        } else {
            // ---- failure face: golden None ⟺ kernel Ok(empty) ⟺ fitsh fail
            assert!(
                e_i.result.is_ok() && e_i.result.as_ref().unwrap().is_empty(),
                "kernel must fail with Ok(empty) on {}: got {:?}, doc bytes {doc:?}",
                case.name,
                e_i.result
            );
            match (&case.expect, &f_res) {
                (Expect::Fail { fitsh: FitshFail::EmptyOk }, Ok((_, out))) => {
                    st.fitsh_empty_ok += 1;
                    assert!(out.is_empty(), "pinned fitsh form empty-Ok broken on {}: got {out:?}", case.name);
                }
                (Expect::Fail { fitsh: FitshFail::CallErr }, Err(err)) => {
                    st.fitsh_call_err += 1;
                    let _ = err;
                }
                (Expect::Fail { fitsh: FitshFail::EmptyOk }, Err(err)) => panic!(
                    "pinned fitsh form broken on {}: expected empty-Ok, got call abort ({err}); doc bytes {doc:?}",
                    case.name
                ),
                (Expect::Fail { fitsh: FitshFail::CallErr }, Ok((_, out))) => panic!(
                    "pinned fitsh form broken on {}: expected CALLEXT abort, got Ok({out:?}); doc bytes {doc:?}",
                    case.name
                ),
                _ => unreachable!("corpus expectation already cross-checked"),
            }
        }
    }

    let aot_note = if cfg!(feature = "calc-aot") {
        format!("AOT checked on {} docs, ", st.aot_checked)
    } else {
        String::new()
    };
    println!(
        "\n== dict_digest adversarial alignment: {} docs ({} success / {} fail; std-container {}, wide-container {}) ==",
        st.docs, st.success, st.fail, st.std_container, st.wide_container
    );
    println!(
        "   three-path split agreement: {}/{} ({})",
        st.agree, st.docs, aot_note
    );
    println!(
        "   failure-face fitsh form: {} empty-Ok (parse errno / walk cap), {} call-abort (missing ITEMGET key)",
        st.fitsh_empty_ok, st.fitsh_call_err
    );
    let tiers: Vec<String> = st.tiers.iter().map(|(t, n)| format!("{t}:{n}")).collect();
    println!("   corpus tiers: {}", tiers.join(", "));
}

fn kernel_container_with_input(input_max: u16) -> vm::calc::container::CalcContainer {
    let read = |n: &str| std::fs::read(format!("{KERNEL_DIR}/{n}")).expect("fixture image");
    let bss_len = u16::from_str_radix(
        std::fs::read_to_string(format!("{KERNEL_DIR}/bss_len.txt"))
            .expect("bss_len.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("bss_len") as u32;
    let probe = vm::calc::container::CalcContainer::from_regions(
        CALC_PROFILE_RV64IM,
        10,
        vm::calc::CALC_CODE_BASE as u32,
        read("image_code.bin"),
        read("image_rodata.bin"),
        read("image_init.bin"),
        input_max,
        32,
    )
    .with_bss(bss_len);
    let mem_kib = vm::calc::container::min_mem_kib_for(probe.guest_end(), input_max as u64, 32)
        .expect("wide input window fits the fixture image");
    let tmp = vm::calc::container::CalcContainer::from_regions(
        CALC_PROFILE_RV64IM,
        mem_kib,
        vm::calc::CALC_CODE_BASE as u32,
        read("image_code.bin"),
        read("image_rodata.bin"),
        read("image_init.bin"),
        input_max,
        32,
    )
    .with_bss(bss_len);
    pack_container(
        &SpaceCap::new(0),
        tmp.profile,
        tmp.mem_kib,
        tmp.entry,
        tmp.input_max,
        tmp.output_max,
        tmp.segs,
    )
    .expect("wide kernel container packs")
}

fn kernel_fn_wide() -> CalcFnObj {
    kernel_obj(kernel_container_with_input(WIDE_INPUT_MAX), false)
}

#[cfg(feature = "calc-aot")]
fn kernel_fn_wide_aot() -> CalcFnObj {
    kernel_obj(kernel_container_with_input(WIDE_INPUT_MAX), true)
}

// ===========================================================================
// pinned real divergences (#[ignore]; run with `-- --ignored`)
// ===========================================================================
//
// Each test below pins an OBSERVED three-way split on a minimized document.
// They are the corpus entries that cannot live on the alignment face because
// one path's capacity window excludes what the others accept. If a future
// change removes a divergence, the pinning assert fails loudly — update the
// pin and promote the doc back into the aligned corpus.

/// Non-'{' input never enters the kernel's dict mode: the fixture falls back
/// to plain sha256 over the raw input and SUCCEEDS, while the fitsh contract
/// refuses (empty) and the golden returns None. One semantic, one path
/// answering a different question.
#[test]
#[ignore = "real divergence: kernel hash-mode fallback on non-'{' input"]
fn dict_divergence_hash_mode_fallback() {
    let kernel = kernel_fn();
    let mut ctx = deploy_dict_digest();
    for doc in [
        &b"start=beta,beta=end}"[..], // the only missing-`{` grammar case
        b"",                          // empty doc
        b"}",
        b"x",
        b"\xC3", // non-ASCII byte
    ] {
        let e = kernel.run(doc, limits());
        let mut want = [0u8; 32];
        run_hash(HashKind::Sha2, doc, &mut want);
        assert_eq!(
            e.result.as_ref().unwrap(),
            &want,
            "kernel hashes non-'{{' input: doc bytes {doc:?}"
        );
        let f = fitsh_call(&mut ctx, doc);
        assert!(
            matches!(&f, Ok((_, out)) if out.is_empty()),
            "fitsh must refuse non-'{{' input: {f:?}, doc bytes {doc:?}"
        );
        assert!(ref_walk(doc).is_none(), "golden must refuse: {doc:?}");
        println!("pinned {doc:?}: kernel=sha256(input), fitsh=empty-Ok, golden=None");
    }
}

/// 65..128 pairs: the fitsh native parses (compo_length 128) and the golden
/// walks (no pair cap), but the kernel's MAX_PAIRS=64 parse table refuses —
/// fitsh+golden succeed, kernel returns empty. On the consensus-shaped
/// container the same doc additionally traps at input admission (> 256B).
#[test]
#[ignore = "real divergence: kernel MAX_PAIRS=64 vs fitsh compo_length=128 / golden no cap"]
fn dict_divergence_pair_cap_65_to_128() {
    let kernel_std = kernel_fn();
    let kernel_wide = kernel_fn_wide();
    let mut ctx = deploy_dict_digest();
    for n_fillers in [64usize, 127usize] {
        // 65 and 128 total pairs — the whole window where fitsh parses
        let doc = start_end_plus_fillers(n_fillers);
        let walked = ref_walk(&doc).expect("golden walks (no pair cap)");
        let want = ref_digest(&walked);

        let (_gas, d_f) = fitsh_call(&mut ctx, &doc)
            .unwrap_or_else(|e| panic!("fitsh must parse {} pairs: {e}, doc bytes {doc:?}", n_fillers + 1));
        assert_eq!(d_f, want, "fitsh walks a {}-pair doc the kernel refuses", n_fillers + 1);

        let e_w = kernel_wide.run(&doc, limits());
        assert!(
            e_w.result.is_ok() && e_w.result.as_ref().unwrap().is_empty(),
            "kernel must refuse {} pairs with Ok(empty): {:?}",
            n_fillers + 1,
            e_w.result
        );

        let e_std = kernel_std.run(&doc, limits());
        assert!(
            matches!(e_std.result, Err(vm::calc::CalcAbort2::Trap)),
            "std container must trap at input admission ({}B > 256B): {:?}",
            doc.len(),
            e_std.result
        );
        println!(
            "pinned {}-pair doc ({}B): fitsh=Ok(digest), kernel=Ok(empty) [std container: Trap], golden=Ok(digest)",
            n_fillers + 1,
            doc.len()
        );
    }
}

/// The kernel walk's MAX_ACC=512 caps the concatenated values; the fitsh
/// contract's `++` reaches 1280 and the golden has no cap. A walk whose acc
/// is 513+ bytes: fitsh+golden succeed, kernel returns empty.
#[test]
#[ignore = "real divergence: kernel MAX_ACC=512 vs fitsh 1280 string / golden no cap"]
fn dict_divergence_kernel_acc_cap_512() {
    let kernel_wide = kernel_fn_wide();
    let mut ctx = deploy_dict_digest();
    for acc_len in [513usize, 519usize] {
        let (doc, acc) = acc_boundary_doc(acc_len);
        assert_eq!(acc.len(), acc_len, "builder must land acc exactly, doc bytes {doc:?}");
        let walked = ref_walk(&doc).expect("golden walks (no acc cap)");
        assert_eq!(walked, acc);
        let want = ref_digest(&walked);

        let (_gas, d_f) = fitsh_call(&mut ctx, &doc)
            .unwrap_or_else(|e| panic!("fitsh must walk acc {acc_len}B: {e}, doc bytes {doc:?}"));
        assert_eq!(d_f, want, "fitsh walks acc {acc_len}B the kernel refuses");

        let e = kernel_wide.run(&doc, limits());
        assert!(
            e.result.is_ok() && e.result.as_ref().unwrap().is_empty(),
            "kernel must refuse acc {acc_len}B > MAX_ACC with Ok(empty): {:?}, doc bytes {doc:?}",
            e.result
        );
        println!(
            "pinned acc {acc_len}B doc ({}B): fitsh=Ok(digest), kernel=Ok(empty), golden=Ok(digest)",
            doc.len()
        );
    }
}

/// The fitsh doc travels as one call-arg `Bytes`: at 1281B it leaves the
/// `SpaceCap::value_size` (1280) window and the CALLEXT refuses outright,
/// while the wide-container kernel and the golden digest it fine. The
/// 1280-exact sibling is on the aligned face (`doc_arg_1280_exact`).
#[test]
#[ignore = "real divergence: fitsh call-arg value_size=1280 vs kernel/golden no doc cap"]
fn dict_divergence_fitsh_doc_arg_1280() {
    let kernel_wide = kernel_fn_wide();
    let mut ctx = deploy_dict_digest();
    let doc = format!("{{start=end,big={}}}", "x".repeat(1265)).into_bytes();
    assert_eq!(doc.len(), 1281);

    let e = kernel_wide.run(&doc, limits());
    let d_i = e.result.as_ref().expect("kernel digests a 1281B doc").clone();
    let want = ref_digest(b"end");
    assert_eq!(d_i, want);
    assert!(ref_walk(&doc).is_some());

    let f = fitsh_call(&mut ctx, &doc);
    assert!(f.is_err(), "fitsh must refuse a 1281B doc arg: {f:?}");
    println!("pinned 1281B doc: fitsh=call-refused, kernel=Ok(digest), golden=Ok(digest)");
}

/// Beyond 128 pairs the fitsh native parse itself fails (`compo_length`),
/// while the golden STILL walks — here the golden disagrees with BOTH
/// implementations, which agree with each other.
#[test]
#[ignore = "real divergence: golden succeeds where both impls refuse (>128 pairs)"]
fn dict_divergence_fitsh_pair_cap_129() {
    let kernel_wide = kernel_fn_wide();
    let mut ctx = deploy_dict_digest();
    let doc = start_end_plus_fillers(129); // 130 pairs, ~631B
    let walked = ref_walk(&doc).expect("golden walks (no pair cap)");
    assert_eq!(walked, b"end", "short chain over a 130-pair doc");

    let f = fitsh_call(&mut ctx, &doc).expect("call itself must go through");
    assert!(f.1.is_empty(), "fitsh must collapse the parse cap to empty: {f:?}");

    let e = kernel_wide.run(&doc, limits());
    assert!(
        e.result.is_ok() && e.result.as_ref().unwrap().is_empty(),
        "kernel must refuse 130 pairs: {:?}",
        e.result
    );
    println!(
        "pinned 130-pair doc ({}B): fitsh=empty-Ok, kernel=Ok(empty), golden=Ok(digest) — golden vs both impls",
        doc.len()
    );
}
