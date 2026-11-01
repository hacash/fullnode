//! Research-phase microbenchmarks: ns per HVM opcode (for the calc-func
//! compute/cycle exchange-rate derivation). Run manually:
//! `cargo test -p vm --release bench_calc -- --nocapture --ignored`

#![cfg(test)]

use base::GasBuckets;
use std::time::Instant;

use crate::machine::test_ctx::TestCtx;
use crate::rt::{Bytecode::*, CodeType};

fn run_program(codes: Vec<u8>) -> (GasBuckets, f64) {
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let t = Instant::now();
    let (gas, _v) = crate::main_call(&mut ctx, CodeType::Bytecode, codes.into()).unwrap();
    (gas, t.elapsed().as_nanos() as f64)
}

/// Time `n` repetitions of `op_bytes` (after `prologue`), subtracting the
/// prologue+END baseline, and report ns/op with the gas delta per op.
fn bench(name: &str, prologue: &[u8], op_bytes: &[u8], n: usize) {
    let mut prog = prologue.to_vec();
    for _ in 0..n {
        prog.extend_from_slice(op_bytes);
    }
    prog.push(END as u8);
    let mut base = prologue.to_vec();
    base.push(END as u8);
    let (gas_op, t_op) = run_program(prog);
    let (gas_base, t_base) = run_program(base);
    let ns = (t_op - t_base) / n as f64;
    let g = GasBuckets {
        compute: (gas_op.compute - gas_base.compute) / n as i64,
        resource: (gas_op.resource - gas_base.resource) / n as i64,
        storage: (gas_op.storage - gas_base.storage) / n as i64,
    };
    println!(
        "{:<24} {:>8.2} ns/op   compute={:>3} resource={:>3} storage={:>3}",
        name, ns, g.compute, g.resource, g.storage
    );
}

use crate::calc::CalcRunLimits;

fn no_limits() -> CalcRunLimits {
    CalcRunLimits { cycles: u64::MAX, resource: i64::MAX, value_size: usize::MAX, deadline: None }
}

#[test]
#[ignore]
fn bench_calc_opcodes() {
    let n = 200_000;
    let c64 = [PU8 as u8, 0, CU64 as u8]; // push u64 0 as seed value
    bench("ADD(u64)", &c64, &[P2 as u8, ADD as u8], n);
    bench(
        "MUL+DIV(u64)",
        &c64,
        &[PU8 as u8, 7, MUL as u8, PU8 as u8, 7, DIV as u8],
        n,
    );
    bench("DIV(u64)", &[PU8 as u8, 7, CU64 as u8], &[PU8 as u8, 7, DIV as u8], n);
    bench("BSHR(u64)", &c64, &[P1 as u8, BSHR as u8], n);
    bench("LT(u64)", &c64, &[DUP as u8, P2 as u8, LT as u8, POP as u8], n);
    bench("POP(after P2)", &c64, &[P2 as u8, POP as u8], n);
    // CAT: each op block is stack-neutral (push,push,cat(-1),pop)
    bench(
        "CAT(1+3B)",
        &[],
        &[PBUF as u8, 1, b'c', PBUF as u8, 2, b'a', b'b', CAT as u8, POP as u8],
        50_000,
    );
    // NTFUNC sha2: push 8B buf, NTFUNC idx=1 (sha2), POP the digest
    bench(
        "NTFUNC sha2(8B)",
        &[P0 as u8],
        &[PBUF as u8, 8, 1, 2, 3, 4, 5, 6, 7, 8, NTFUNC as u8, 1, POP as u8],
        5_000,
    );
}

#[test]
#[ignore]
fn bench_calc_sha256_kernel() {
    use crate::calc::container::CalcContainer;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sha256kernel");
    let code = std::fs::read(format!("{dir}/image_code.bin")).unwrap();
    let rodata = std::fs::read(format!("{dir}/image_rodata.bin")).unwrap();
    let entry: u32 = std::fs::read_to_string(format!("{dir}/entry.txt"))
        .ok()
        .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok())
        .unwrap_or(crate::calc::CALC_CODE_BASE as u32);
    let container = CalcContainer {
        profile: crate::calc::CALC_PROFILE_RV64IM,
        abi: 1,
        mem_kib: 16,
        entry,
        code,
        rodata,
        init: vec![],
        bss_len: 0x400,
        input_max: 1024,
        output_max: 64,
    };
    let input: Vec<u8> = (0..64u8).map(|i| i.wrapping_mul(7).wrapping_add(3)).collect();
    let f = crate::calc::CalcFnObj::build(container).expect("build");
    let t = Instant::now();
    let exec = f.run(&input, no_limits());
    let dur = t.elapsed();
    let out = exec.result.expect("sha256 ok");
    assert_eq!(out.len(), 32);
    let compute = 16 + (exec.cycles_used + 3) / 4;
    println!(
        "sha256 calc kernel (64B in): {} cycles -> compute {} resource {} | {:?} | {:.1} ns/cycle",
        exec.cycles_used,
        compute,
        exec.resource,
        dur,
        dur.as_nanos() as f64 / exec.cycles_used as f64
    );
    let input2: Vec<u8> = (0..255u8).chain([255u8]).map(|i: u8| i.wrapping_mul(13)).collect();
    let t = Instant::now();
    let exec2 = f.run(&input2, no_limits());
    let dur2 = t.elapsed();
    println!(
        "sha256 calc kernel (256B in): {} cycles -> compute {} resource {} | {:?} | {:.1} ns/cycle",
        exec2.cycles_used,
        16 + (exec2.cycles_used + 3) / 4,
        exec2.resource,
        dur2,
        dur2.as_nanos() as f64 / exec2.cycles_used as f64
    );
}

#[test]
#[ignore]
fn bench_hvm_sha256_loop() {
    // HVM u32 arithmetic is CHECKED: a wrapping add (the atom of any hash
    // compression) cannot be written directly. The loop below measures the
    // canonical half-word expansion `wadd`: lo = (a&0xFFFF)+(b&0xFFFF);
    // hi = (a>>16)+(b>>16)+(lo>>16); result = ((hi&0xFFFF)<<16)|(lo&0xFFFF).
    let body = r#"
var a = 4008636142u32
var b = 2987654321u32
var i = 0
var r = 0u32
var lo = 0u32
var hi = 0u32
while i < 64 {
    lo = (a & 65535u32) + (b & 65535u32)
    hi = (a >> 16) + (b >> 16) + (lo >> 16)
    r = ((hi & 65535u32) << 16) | (lo & 65535u32)
    a = r
    i = i + 1
}
return r
"#;
    let codes = crate::lang::lang_to_bytecode(body).expect("lang compile");
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let (gas, _v) = crate::main_call(&mut ctx, crate::rt::CodeType::Bytecode, codes.into())
        .expect("run");
    println!(
        "HVM wrapping-add (half-word expansion) x64: compute {} resource {} | per-add compute {:.1}",
        gas.compute,
        gas.resource,
        (gas.compute - 48 - 2 * 21) as f64 / 64.0 // minus main base + loop overhead estimate
    );
    // per-op baseline for the calc side: 1 wrapping ADD = 1 cycle = 0.25 compute
}
