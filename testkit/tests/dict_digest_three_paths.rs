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
//!   2. **calc interp** — the RV64IMC `sha256kernel` fixture
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
//! Run:
//!   cargo test -p testkit --test dict_digest_three_paths -- --nocapture
//!   cargo test -p testkit --features calc-aot --test dict_digest_three_paths -- --nocapture

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
    .expect("bss_len") as u64;
    // The container has no bss field: the fixture's zero-initialized span
    // is part of the init image.
    let mut init = read("image_init.bin");
    init.resize(init.len() + bss_len as usize, 0);
    pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        8,
        vm::calc::CALC_CODE_BASE as u32,
        256,
        32,
        read("image_code.bin"),
        read("image_rodata.bin"),
        init,
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
