//! Calc test-plan coverage suite (`vm/doc/calc-test-plan.loc.md`).
//!
//! Series implemented here:
//! - C: container wire-format negative matrix (every decode/invariant error
//!   string), mem_kib boundary sweep, image-size cap, pack_elf rejection
//!   matrix built from hand-rolled ELF64 images.
//! - D: decoder whitelist vectors for the full RV64IM profile-0 surface
//!   (field-assembler + clang-anchored values), bitmanip decode in profile B
//!   vs rejection in profile 0, and every decode fallback error path.
//! - E: ecall ABI decision branches, register-entry snapshot, echo scan,
//!   address wraparound / far-beyond-memory vectors, misaligned
//!   cross-region loads/stores.
//! - F: the region x operation permission matrix (load/store/fetch over
//!   every memory region) with interp/AOT lockstep on every cell.
//! - G: consensus constant + static-weight pinning, setup-charge semantics,
//!   deadline reporting.
//! - I: clean-return kernel AOT parity (output, cycles, abort class).

use std::time::Instant;

use vm::calc::container::{
    CalcContainer, CalcSegment, CODE_BASE, HALT_STUB, SEG_R, SEG_W, SEG_X,
    CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IM_B,
};
use vm::calc::decode::{
    decode_at, inst_cycles, BitImmOp, BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind,
    StoreOp, UnAluOp, WEIGHTS,
};
use vm::calc::interp::{self, CalcAbort, CalcRun};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::{
    pack_code, pack_container, pack_elf, pack_elf_min_mem, run_hash, CalcAbort2, CalcFnObj,
    CalcHostConfig, CalcRunLimits, HashKind, SpaceCap, CALC_MEM_RESOURCE_PER_KIB,
    CALC_SETUP_BASE_CYCLES, CALC_SETUP_PER_KIB_CYCLES, CYCLES_PER_COMPUTE,
};

// ------------------------------------------------------------- encoders ---

fn enc_i(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> [u8; 4] {
    let w = (((imm as u32) & 0xFFF) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op;
    w.to_le_bytes()
}
fn enc_r(f7: u32, rs2: u32, rs1: u32, f3: u32, rd: u32, op: u32) -> [u8; 4] {
    let w = (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op;
    w.to_le_bytes()
}
fn enc_s(imm: i32, rs2: u32, rs1: u32, f3: u32, op: u32) -> [u8; 4] {
    let u = imm as u32;
    let w = (((u >> 5) & 0x7F) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | ((u & 0x1F) << 7) | op;
    w.to_le_bytes()
}
#[allow(dead_code)]
fn enc_b(imm: i32, rs2: u32, rs1: u32, f3: u32, op: u32) -> [u8; 4] {
    let u = imm as u32;
    let w = (((u >> 12) & 1) << 31)
        | (((u >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | (((u >> 1) & 0xF) << 8)
        | (((u >> 11) & 1) << 7)
        | op;
    w.to_le_bytes()
}
#[allow(dead_code)]
fn enc_u(imm20: u32, rd: u32, op: u32) -> [u8; 4] {
    let w = ((imm20 & 0xFFFFF) << 12) | (rd << 7) | op;
    w.to_le_bytes()
}

/// Materialize `rd = v` with lui+addi (v < 2^31) or a single addi.
fn li(rd: u32, v: u64) -> Vec<u8> {
    if v < 0x800 {
        return enc_i(v as i32, 0, 0, rd, 0x13).to_vec();
    }
    assert!(v + 0x800 <= 0xFFFF_F800, "li range");
    let hi = ((v + 0x800) >> 12) as u32;
    let lo = (v as i64) - ((hi as i64) << 12);
    let mut out = enc_u(hi, rd, 0x37).to_vec();
    out.extend_from_slice(&enc_i(lo as i32, rd, 0, rd, 0x13));
    out
}

fn halt_empty() -> Vec<u8> {
    enc_i(0, 0, 0, 10, 0x13) // addi a0, x0, 0
        .into_iter()
        .chain(enc_i(0, 0, 0, 11, 0x13)) // addi a1, x0, 0
        .chain(enc_i(0, 0, 0, 17, 0x13)) // addi a7, x0, 0
        .chain([0x73, 0, 0, 0]) // ecall
        .collect()
}

/// HALT with exit 0, keeping the caller-set a1 (output length).
fn halt_keep_a1() -> Vec<u8> {
    enc_i(0, 0, 0, 10, 0x13)
        .into_iter()
        .chain(enc_i(0, 0, 0, 17, 0x13))
        .chain([0x73, 0, 0, 0])
        .collect()
}

// -------------------------------------------------------------- runners ---

fn code_container(code: Vec<u8>) -> CalcContainer {
    CalcContainer::from_code(CALC_PROFILE_RV64IM, 8, CODE_BASE as u32, code, 64, 64)
}

fn verify_container(c: &CalcContainer) -> vm::calc::verify::CalcModule {
    verify_container_with_cap(c, &SpaceCap::new(0)).expect("container verifies")
}

fn run(code: Vec<u8>, input: &[u8], budget: u64) -> Result<CalcRun, (CalcAbort, u64)> {
    let c = code_container(code);
    let m = verify_container(&c);
    interp::run(&m, &c, input, budget, None)
}

fn expect_trap(res: Result<CalcRun, (CalcAbort, u64)>, msg_part: &str) {
    match res {
        Err((CalcAbort::Trap(t), _)) => assert!(
            t.contains(msg_part),
            "trap message `{t}` must contain `{msg_part}`"
        ),
        other => panic!("expected trap containing `{msg_part}`, got {other:?}"),
    }
}

/// CalcFnObj lockstep (interp + AOT): returns (class, cycles_used incl. setup).
fn lockstep(c: &CalcContainer, input: &[u8], budget: u64) -> (Result<Vec<u8>, CalcAbort2>, u64) {
    let obj = CalcFnObj::build_with_aot(c.clone()).expect("build with aot");
    let exec = obj.run(
        input,
        CalcRunLimits {
            cycles: budget,
            resource: -1,
            deadline: None,
        },
    );
    (exec.result, exec.cycles_used)
}

fn setup_cycles(c: &CalcContainer) -> u64 {
    CALC_SETUP_BASE_CYCLES + (c.layout().usable_len / 1024) * CALC_SETUP_PER_KIB_CYCLES
}

/// interp vs AOT must agree on the consensus-observable surface.

// ================================================================= G: pinning

/// G-001/G-002: consensus constants are frozen. Any change here is a hard
/// fork and must go through the activation process (SPEC.md §8).
#[test]
fn consensus_constants_are_pinned() {
    assert_eq!(CYCLES_PER_COMPUTE, 56, "1 compute = 56 cycles (hard fork to change)");
    assert_eq!(CALC_MEM_RESOURCE_PER_KIB, 2);
    assert_eq!(CALC_SETUP_BASE_CYCLES, 64);
    assert_eq!(CALC_SETUP_PER_KIB_CYCLES, 16);
    assert_eq!(vm::calc::container::CALC_MEM_KIB_MIN, 4);
    assert_eq!(vm::calc::CALC_MIN_FREE_RW_GAP, 2048, "minimum stack only; no guard band");

    let cap = SpaceCap::new(0);
    assert_eq!(cap.calc_mem_kib_max, 64, "L1 default mem cap");
    assert_eq!(cap.calc_image_max, 16383, "L1 default image cap");
    assert_eq!(cap.call_data_size, 5120);
    assert_eq!(cap.value_size, 1280);
}

/// G-002: static weight table pinning (SPEC.md §8 table).
#[test]
fn static_weights_are_pinned() {
    assert_eq!(WEIGHTS.alu, 1);
    assert_eq!(WEIGHTS.load_store, 2);
    assert_eq!(WEIGHTS.mul, 3);
    assert_eq!(WEIGHTS.div, 12);
    assert_eq!(WEIGHTS.clmul, 24);
    assert_eq!(WEIGHTS.ecall_base, 1);

    let w = |code: &[u8]| inst_cycles(&decode_at(code, 0, CALC_PROFILE_RV64IM_B).unwrap().0);
    assert_eq!(w(&enc_i(0, 1, 0, 5, 0x13)), 1, "addi");
    assert_eq!(w(&enc_i(0, 1, 3, 5, 0x03)), 2, "ld");
    assert_eq!(w(&enc_s(0, 2, 1, 3, 0x23)), 2, "sd");
    assert_eq!(w(&enc_r(1, 2, 1, 0, 5, 0x33)), 3, "mul");
    assert_eq!(w(&enc_r(1, 2, 1, 4, 5, 0x33)), 12, "div");
    assert_eq!(w(&enc_r(1, 2, 1, 0, 5, 0x3B)), 3, "mulw");
    assert_eq!(w(&enc_r(1, 2, 1, 4, 5, 0x3B)), 12, "divw");
    assert_eq!(w(&enc_r(5, 2, 1, 1, 5, 0x33)), 24, "clmul");
    assert_eq!(w(&[0x73, 0, 0, 0]), 1, "ecall base");
    assert_eq!(w(&[0x73, 0x00, 0x10, 0x00]), 1, "ebreak (Trap)");
    assert_eq!(w(&enc_i(0, 0, 0, 0, 0x0F)), 1, "fence");
}

/// G-002: exact block charges through the interpreter (no setup at this
/// layer): every class is billed per the pinned table.
#[test]
fn cycle_charges_match_weights() {
    // 4 ALU (trampoline-return style halt) = 4.
    let res = run(halt_empty(), &[], 1000).unwrap();
    assert_eq!(res.cycles_used, 4);

    // store(2) + halt(4) = 6: sw x0, 0(a2)
    let mut code = enc_s(0, 0, 12, 2, 0x23).to_vec();
    code.extend_from_slice(&halt_empty());
    assert_eq!(run(code, &[], 1000).unwrap().cycles_used, 6);

    // load(2) + halt(4) = 6: ld t0, 0(a0) — input region is readable
    let mut code = enc_i(0, 10, 3, 5, 0x03).to_vec();
    code.extend_from_slice(&halt_empty());
    assert_eq!(run(code, b"abc", 1000).unwrap().cycles_used, 6);

    // mul(3) + halt(4) = 7: mul t0, x0, x0
    let mut code = enc_r(1, 0, 0, 0, 5, 0x33).to_vec();
    code.extend_from_slice(&halt_empty());
    assert_eq!(run(code, &[], 1000).unwrap().cycles_used, 7);

    // div(12) + halt(4) = 16: div t0, x0, x0 (0 / 0 -> all-ones, no trap)
    let mut code = enc_r(1, 0, 0, 4, 5, 0x33).to_vec();
    code.extend_from_slice(&halt_empty());
    assert_eq!(run(code, &[], 1000).unwrap().cycles_used, 16);

    // fence(1) + halt(4) = 5
    let mut code = enc_i(0, 0, 0, 0, 0x0F).to_vec();
    code.extend_from_slice(&halt_empty());
    assert_eq!(run(code, &[], 1000).unwrap().cycles_used, 5);

    // ebreak: static 1 cycle, then trap (charge kept).
    let mut code = [0x73u8, 0x00, 0x10, 0x00].to_vec(); // ebreak
    code.extend_from_slice(&halt_empty());
    expect_trap(run(code, &[], 1000), "trap instruction");

    // unknown ecall: addi a7,32 (block 1) + ecall (block 1) = 2.
    let mut code = enc_i(32, 0, 0, 17, 0x13).to_vec();
    code.extend_from_slice(&[0x73, 0, 0, 0]);
    let res = run(code, &[], 1000);
    expect_trap(res, "hash builtins not declared");
}

/// G-003: setup charge semantics — kept on trap, rejected when the budget
/// cannot cover it, exact on instant return.
#[test]
fn setup_charge_semantics() {
    let c = CalcContainer::from_code(CALC_PROFILE_RV64IM, 4, CODE_BASE as u32, halt_empty(), 16, 16);
    let setup = CALC_SETUP_BASE_CYCLES + 4 * CALC_SETUP_PER_KIB_CYCLES;
    assert_eq!(setup, 128);

    // budget < setup -> terminal OutOfGas reporting exactly the setup
    let obj = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: setup - 1,
            resource: -1,
            deadline: None,
        },
    );
    assert!(matches!(exec.result, Err(CalcAbort2::OutOfGas)));
    assert_eq!(exec.cycles_used, setup);

    // budget = setup + engine cost -> exact
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: setup + 4,
            resource: -1,
            deadline: None,
        },
    );
    assert!(exec.result.is_ok());
    assert_eq!(exec.cycles_used, setup + 4);

    // trap keeps the setup charge: ebreak kernel costs setup + 1
    let tc = CalcContainer::from_code(
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        vec![0x73, 0x00, 0x10, 0x00], // ebreak
        16,
        16,
    );
    let obj = CalcFnObj::build_with_host(tc, CalcHostConfig::new()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 10_000,
            resource: -1,
            deadline: None,
        },
    );
    assert!(matches!(exec.result, Err(CalcAbort2::Trap)));
    assert_eq!(exec.cycles_used, setup + 1);
}

/// G-008: host wall-clock deadline is observed by the interpreter; the run
/// reports Deadline with the interp-side cycle count.
#[test]
fn deadline_abort_reports_interp_cycles() {
    // jal x0, 0 self-loop never halts; an expired deadline must stop it.
    let c = code_container(0x6F_u32.to_le_bytes().to_vec());
    let obj = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1_000_000,
            resource: -1,
            deadline: Some(Instant::now() - std::time::Duration::from_secs(1)),
        },
    );
    assert!(matches!(exec.result, Err(CalcAbort2::Deadline)), "{:?}", exec.result);
    let setup = setup_cycles(&c);
    assert_eq!(exec.cycles_used, setup, "expired deadline stops before the first block");

    // The same kernel without a deadline runs OutOfGas instead.
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1_000,
            resource: -1,
            deadline: None,
        },
    );
    assert!(matches!(exec.result, Err(CalcAbort2::OutOfGas)));
}

// ================================================================= C: container format

fn encode_of(c: &CalcContainer) -> Vec<u8> {
    c.encode().unwrap()
}

fn decode_err_contains(bytes: &[u8], part: &str) {
    match CalcContainer::decode(bytes) {
        Err(e) => assert!(
            e.1.contains(part),
            "error `{}` must contain `{part}`",
            e.1
        ),
        Ok(_) => panic!("decode must fail with `{part}`"),
    }
}

fn verify_err_contains(c: &CalcContainer, part: &str) {
    match verify_container_with_cap(c, &SpaceCap::new(0)) {
        Err(e) => assert!(e.1.contains(part), "error `{}` must contain `{part}`", e.1),
        Ok(_) => panic!("verify must fail with `{part}`"),
    }
}

fn valid_packed() -> CalcContainer {
    pack_code(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        8,
        CODE_BASE as u32,
        16,
        16,
        halt_empty(),
    )
    .unwrap()
}

/// C-2xx: every decode-layer error string.
#[test]
fn container_decode_error_matrix() {
    let bytes = encode_of(&valid_packed());

    // too short
    decode_err_contains(&bytes[..10], "too short");
    // magic
    let mut b = bytes.clone();
    b[0] = b'X';
    decode_err_contains(&b, "magic mismatch");
    // version
    for v in [0u8, 2] {
        let mut b = bytes.clone();
        b[4] = v;
        decode_err_contains(&b, "version mismatch");
    }
    // reserved header byte
    let mut b = bytes.clone();
    b[7] = 1;
    decode_err_contains(&b, "reserved header bytes must be zero");
    // segment count
    let mut b = bytes.clone();
    b[6] = 0;
    decode_err_contains(&b, "segment count invalid");
    let mut b = bytes.clone();
    b[6] = 17;
    decode_err_contains(&b, "segment count invalid");
    // declared table longer than payload
    let mut b = bytes.clone();
    b[6] = 2;
    decode_err_contains(&b, "truncated payload");
    // trailing bytes
    let mut b = bytes.clone();
    b.push(0);
    decode_err_contains(&b, "trailing bytes");
    // segment padding must be zero (first seg header reserved byte 13)
    let mut b = bytes.clone();
    b[20 + 13] = 1;
    decode_err_contains(&b, "segment padding must be zero");
    // huge declared lengths must not panic
    let mut b = vec![0u8; 20 + 16];
    b[0..4].copy_from_slice(b"HCF1");
    b[4] = 1;
    b[6] = 1;
    b[8..12].copy_from_slice(&4u32.to_be_bytes());
    b[12..14].copy_from_slice(&16u16.to_be_bytes());
    b[14..16].copy_from_slice(&16u16.to_be_bytes());
    b[16..20].copy_from_slice(&(CODE_BASE as u32).to_be_bytes());
    b[20..24].copy_from_slice(&(CODE_BASE as u32).to_be_bytes());
    b[24..28].copy_from_slice(&u32::MAX.to_be_bytes());
    b[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
    b[32] = SEG_R | SEG_X;
    decode_err_contains(&b, "");
}

/// C-2xx: invariant-layer error strings, driven through decode.
#[test]
fn container_invariant_error_matrix() {
    let bad = |segs: Vec<CalcSegment>, entry: u32, mem_kib: u32, imax: u16, omax: u16| {
        CalcContainer {
            profile: CALC_PROFILE_RV64IM,
            mem_kib,
            entry,
            segs,
            input_max: imax,
            output_max: omax,
        }
        .encode()
        .unwrap()
    };
    let code = halt_empty();

    // unknown / admitted / not-admitted profiles: every B / RVC combination
    // (bytes 0..=3) decodes; the HASH bit (4) and unknown values do not.
    let b = bad(vec![CalcSegment::rx(CODE_BASE as u32, code.clone())], CODE_BASE as u32, 8, 16, 16);
    let mut b = b.clone();
    b[5] = 8;
    decode_err_contains(&b, "unknown isa profile");
    for p in [1u8, 2, 3] {
        let mut ok = b.clone();
        ok[5] = p;
        assert!(
            CalcContainer::decode(&ok).is_ok(),
            "profile {p} must be admitted"
        );
    }
    let mut b = b.clone();
    b[5] = 4;
    decode_err_contains(&b, "isa profile not admitted");

    // mem_kib out of range (decode-level engine bounds)
    let mk = |kib: u32| {
        bad(vec![CalcSegment::rx(CODE_BASE as u32, code.clone())], CODE_BASE as u32, kib, 16, 16)
    };
    decode_err_contains(&mk(3), "mem_kib out of range");
    decode_err_contains(&mk(2_097_148), "mem_kib out of range");
    // engine ceiling itself decodes fine
    assert!(CalcContainer::decode(&mk(2_097_147)).is_ok());

    // output_max == 0 (input_max == 0 stays legal)
    decode_err_contains(&bad(vec![CalcSegment::rx(CODE_BASE as u32, code.clone())], CODE_BASE as u32, 8, 16, 0), "io size invalid");
    assert!(CalcContainer::decode(&bad(
        vec![CalcSegment::rx(CODE_BASE as u32, code.clone())],
        CODE_BASE as u32,
        8,
        0,
        16
    ))
    .is_ok());

    // segment flags outside R|W|X
    let mut seg = CalcSegment::rx(CODE_BASE as u32, code.clone());
    seg.flags |= 0x8;
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "segment flags invalid");

    // missing R bit
    let seg = CalcSegment {
        vaddr: CODE_BASE as u32,
        memsz: code.len() as u32,
        flags: SEG_X,
        data: code.clone(),
    };
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "not readable");

    // W^X violation
    let seg = CalcSegment {
        vaddr: CODE_BASE as u32,
        memsz: code.len() as u32,
        flags: SEG_R | SEG_W | SEG_X,
        data: code.clone(),
    };
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "W^X violated");

    // filesz > memsz
    let seg = CalcSegment::rw(CODE_BASE as u32, vec![0u8; 32], 16);
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "filesz exceeds memsz");

    // empty segment
    let seg = CalcSegment::rw(CODE_BASE as u32, vec![], 0);
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "empty segment");

    // vaddr + memsz overflow
    // vaddr + memsz beyond u32: the u64-based "segment overflow" guard is
    // unreachable with u32 wire fields; the layout fit check rejects instead.
    let seg = CalcSegment::rx(0xFFFF_FFF0, code.clone());
    decode_err_contains(&bad(vec![seg], 0xFFFF_FFF0, 8, 16, 16), "memory overflows");

    // below CODE_BASE
    let seg = CalcSegment::rx(0x800, code.clone());
    decode_err_contains(&bad(vec![seg], 0x800, 8, 16, 16), "below CODE_BASE");

    // unsorted / overlapping
    let s1 = CalcSegment::rx(CODE_BASE as u32, code.clone());
    let s2 = CalcSegment::ro(CODE_BASE as u32 + 4, vec![0, 0, 0, 0]);
    decode_err_contains(&bad(vec![s1.clone(), s2], CODE_BASE as u32, 8, 16, 16), "overlap or unsorted");
    let s3 = CalcSegment::rx(CODE_BASE as u32 + 0x100, code.clone());
    decode_err_contains(&bad(vec![s3, s1], CODE_BASE as u32, 8, 16, 16), "overlap or unsorted");

    // executable BSS
    let seg = CalcSegment {
        vaddr: CODE_BASE as u32,
        memsz: code.len() as u32 + 8,
        flags: SEG_R | SEG_X,
        data: code.clone(),
    };
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "executable bss not allowed");

    // code length not 4-byte aligned (profile 0)
    let seg = CalcSegment::rx(CODE_BASE as u32, vec![0x13, 0, 0]);
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "code_len invalid");

    // no executable segment
    let seg = CalcSegment::ro(CODE_BASE as u32, code.clone());
    decode_err_contains(&bad(vec![seg], CODE_BASE as u32, 8, 16, 16), "no executable segment");

    // entry outside the executable segment
    let b = bad(
        vec![
            CalcSegment::rx(CODE_BASE as u32, code.clone()),
            CalcSegment::ro(CODE_BASE as u32 + 0x100, vec![0, 0, 0, 0]),
        ],
        CODE_BASE as u32 + 0x100,
        8,
        16,
        16,
    );
    decode_err_contains(&b, "entry outside code");

    // entry not 4-byte aligned
    let b = bad(vec![CalcSegment::rx(CODE_BASE as u32, code.clone())], CODE_BASE as u32 + 1, 8, 16, 16);
    decode_err_contains(&b, "entry not 4-byte aligned");
}

/// C-2xx: layout overflow and chain-cap verification.
#[test]
fn container_layout_and_cap_errors() {
    // 4 KiB declared memory cannot hold a 4 KiB output window on top of code
    let c = CalcContainer::from_code(CALC_PROFILE_RV64IM, 4, CODE_BASE as u32, halt_empty(), 0, 4096);
    verify_err_contains(&c, "memory overflows");

    // mem_kib above the chain cap: decode accepts 65, verify rejects it
    let c = CalcContainer::from_code(CALC_PROFILE_RV64IM, 65, CODE_BASE as u32, halt_empty(), 16, 16);
    assert!(CalcContainer::decode(&c.encode().unwrap()).is_ok());
    verify_err_contains(&c, "mem_kib exceeds chain cap");

    // a raised sidechain cap admits it
    let mut cap = SpaceCap::new(0);
    cap.calc_mem_kib_max = 128;
    assert!(verify_container_with_cap(&c, &cap).is_ok());

    // the chain cap never lowers the engine ceiling: 2 GiB passes a huge cap
    let mut cap = SpaceCap::new(0);
    cap.calc_mem_kib_max = 2_097_147;
    let c = CalcContainer::from_code(CALC_PROFILE_RV64IM, 2_097_147, CODE_BASE as u32, halt_empty(), 16, 16);
    assert!(verify_container_with_cap(&c, &cap).is_ok());
}

/// C-311: image size cap — 16383 default, u16 hard ceiling, sidechain raise.
#[test]
fn image_size_cap_matrix() {
    let big = vec![0x13u8, 0, 0, 0].repeat(5000); // 20000 bytes of nop
    // mem_kib must fit guest_end + gap
    let err = pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        32,
        CODE_BASE as u32,
        16,
        16,
        vec![CalcSegment::rx(CODE_BASE as u32, big.clone())],
    )
    .unwrap_err();
    assert!(err.1.contains("calcfunc size overflow"), "{}", err.1);

    // raising the cap admits it (still below the u16 body ceiling)
    let mut cap = SpaceCap::new(0);
    cap.calc_image_max = 65535;
    assert!(pack_container(
        &cap,
        CALC_PROFILE_RV64IM,
        32,
        CODE_BASE as u32,
        16,
        16,
        vec![CalcSegment::rx(CODE_BASE as u32, big)],
    )
    .is_ok());
}

/// C-109: mem_kib boundary sweep through decode.
#[test]
fn mem_kib_boundary_sweep() {
    let mk = |kib: u32| {
        CalcContainer::from_code(CALC_PROFILE_RV64IM, kib, CODE_BASE as u32, halt_empty(), 16, 16)
            .encode()
            .unwrap()
    };
    assert!(CalcContainer::decode(&mk(4)).is_ok(), "minimum");
    assert!(CalcContainer::decode(&mk(5)).is_ok());
    assert!(CalcContainer::decode(&mk(64)).is_ok(), "L1 cap");
    assert!(CalcContainer::decode(&mk(65)).is_ok(), "above L1 cap still decodes");
    assert!(CalcContainer::decode(&mk(3)).is_err(), "below minimum");
    assert!(CalcContainer::decode(&mk(0)).is_err());
    assert!(CalcContainer::decode(&mk(2_097_147)).is_ok(), "engine ceiling");
    assert!(CalcContainer::decode(&mk(2_097_148)).is_err(), "above engine ceiling");
}

// ------------------------------------------------------------- C: pack_elf

const EH_SIZE: usize = 64;
const PH_SIZE: usize = 56;

struct SegSpec {
    ptype: u32,
    flags: u32,
    vaddr: u64,
    memsz: u64,
    data: Vec<u8>,
}

fn build_elf(
    e_type: u16,
    e_machine: u16,
    e_flags: u32,
    entry: u64,
    segs: &[SegSpec],
    rela_shdr: bool,
) -> Vec<u8> {
    let mut f = Vec::new();
    let data_start = EH_SIZE + segs.len() * PH_SIZE;
    let mut offs = Vec::new();
    let mut cur = data_start as u64;
    for s in segs {
        offs.push(cur);
        cur += s.data.len() as u64;
    }
    let mut ident = vec![0x7Fu8, b'E', b'L', b'F', 2, 1, 1, 0];
    ident.extend_from_slice(&[0u8; 8]);
    f.extend_from_slice(&ident);
    f.extend_from_slice(&e_type.to_le_bytes());
    f.extend_from_slice(&e_machine.to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes()); // e_version
    f.extend_from_slice(&entry.to_le_bytes());
    f.extend_from_slice(&(EH_SIZE as u64).to_le_bytes()); // e_phoff
    let shoff = if rela_shdr { cur } else { 0 };
    f.extend_from_slice(&shoff.to_le_bytes());
    f.extend_from_slice(&e_flags.to_le_bytes());
    f.extend_from_slice(&(EH_SIZE as u16).to_le_bytes());
    f.extend_from_slice(&(PH_SIZE as u16).to_le_bytes());
    f.extend_from_slice(&(segs.len() as u16).to_le_bytes());
    f.extend_from_slice(&(64u16).to_le_bytes()); // e_shentsize
    f.extend_from_slice(&(if rela_shdr { 2u16 } else { 0 }).to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx

    for (i, s) in segs.iter().enumerate() {
        f.extend_from_slice(&s.ptype.to_le_bytes());
        f.extend_from_slice(&s.flags.to_le_bytes());
        f.extend_from_slice(&offs[i].to_le_bytes()); // p_offset
        f.extend_from_slice(&s.vaddr.to_le_bytes());
        f.extend_from_slice(&s.vaddr.to_le_bytes()); // p_paddr
        f.extend_from_slice(&(s.data.len() as u64).to_le_bytes()); // p_filesz
        f.extend_from_slice(&s.memsz.to_le_bytes());
        f.extend_from_slice(&16u64.to_le_bytes()); // p_align
    }
    for s in segs {
        f.extend_from_slice(&s.data);
    }
    if rela_shdr {
        // two 64-byte section headers: NULL (also the shstrtab) + SHT_RELA.
        // The NULL section's sh_size points at the NUL-terminated ASCII
        // bytes of the ELF header itself so name lookups resolve.
        let shoff = f.len() as u64;
        let mut sh = Vec::new();
        let mut null_hdr = vec![0u8; 64];
        null_hdr[32..40].copy_from_slice(&8u64.to_le_bytes()); // sh_size = 8
        sh.extend_from_slice(&null_hdr);
        sh.extend_from_slice(&0u32.to_le_bytes()); // sh_name
        sh.extend_from_slice(&4u32.to_le_bytes()); // sh_type = SHT_RELA
        sh.extend_from_slice(&0u64.to_le_bytes()); // sh_flags
        sh.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
        sh.extend_from_slice(&0u64.to_le_bytes()); // sh_offset
        sh.extend_from_slice(&24u64.to_le_bytes()); // sh_size
        sh.extend_from_slice(&0u32.to_le_bytes()); // sh_link
        sh.extend_from_slice(&0u32.to_le_bytes()); // sh_info
        sh.extend_from_slice(&1u64.to_le_bytes()); // sh_addralign
        sh.extend_from_slice(&24u64.to_le_bytes()); // sh_entsize
        f.extend_from_slice(&sh);
        f[40..48].copy_from_slice(&shoff.to_le_bytes()); // e_shoff
    }
    f
}

fn elf_halt_code() -> Vec<u8> {
    halt_empty()
}

fn good_elf() -> Vec<u8> {
    build_elf(
        2,
        243,
        0,
        CODE_BASE as u64,
        &[SegSpec {
            ptype: 1,
            flags: 5, // R|X
            vaddr: CODE_BASE as u64,
            memsz: 16,
            data: elf_halt_code(),
        }],
        false,
    )
}

fn elf_err(elf: &[u8], part: &str) {
    match pack_elf(&SpaceCap::new(0), elf, CALC_PROFILE_RV64IM, 8, 16, 16) {
        Err(err) => assert!(err.1.contains(part), "error `{}` must contain `{part}`", err.1),
        Ok(_) => panic!("pack_elf must fail with `{part}`, got Ok"),
    }
}

/// C-3xx: pack_elf acceptance and the rejection matrix.
#[test]
fn pack_elf_accepts_static_exec() {
    let c = pack_elf(&SpaceCap::new(0), &good_elf(), CALC_PROFILE_RV64IM, 8, 16, 16).unwrap();
    assert_eq!(c.entry, CODE_BASE as u32);
    assert_eq!(c.segs.len(), 1);
    // min_mem picks the smallest fitting mem_kib (4 for this tiny image)
    let c = pack_elf_min_mem(&SpaceCap::new(0), &good_elf(), CALC_PROFILE_RV64IM, 16, 16).unwrap();
    assert_eq!(c.mem_kib, 4);
}

#[test]
fn pack_elf_rejection_matrix() {
    // bad magic
    let mut e = good_elf();
    e[0] = 0;
    elf_err(&e, "bad magic");
    // ELF32
    let mut e = good_elf();
    e[4] = 1;
    elf_err(&e, "not ELF64");
    // big-endian
    let mut e = good_elf();
    e[5] = 2;
    elf_err(&e, "not little-endian");
    // ET_DYN / ET_REL / ET_CORE
    for t in [3u16, 1, 4] {
        let e = build_elf(t, 243, 0, CODE_BASE as u64, &[], false);
        elf_err(&e, "not ET_EXEC");
    }
    // wrong machine
    let e = build_elf(2, 62, 0, CODE_BASE as u64, &[], false);
    elf_err(&e, "not EM_RISCV");
    // EF_RISCV_RVC set
    let e = build_elf(
        2,
        243,
        1,
        CODE_BASE as u64,
        &[SegSpec { ptype: 1, flags: 5, vaddr: CODE_BASE as u64, memsz: 16, data: elf_halt_code() }],
        false,
    );
    elf_err(&e, "EF_RISCV_RVC");
    // PT_DYNAMIC / PT_INTERP / PT_TLS
    for ptype in [2u32, 3, 7] {
        let e = build_elf(
            2,
            243,
            0,
            CODE_BASE as u64,
            &[SegSpec { ptype, flags: 5, vaddr: CODE_BASE as u64, memsz: 16, data: elf_halt_code() }],
            false,
        );
        elf_err(&e, "dynamic/interp/tls not allowed");
    }
    // W^X
    let e = build_elf(
        2,
        243,
        0,
        CODE_BASE as u64,
        &[SegSpec { ptype: 1, flags: 7, vaddr: CODE_BASE as u64, memsz: 16, data: elf_halt_code() }],
        false,
    );
    elf_err(&e, "W^X violated");
    // executable BSS
    let e = build_elf(
        2,
        243,
        0,
        CODE_BASE as u64,
        &[SegSpec { ptype: 1, flags: 5, vaddr: CODE_BASE as u64, memsz: 64, data: elf_halt_code() }],
        false,
    );
    elf_err(&e, "cannot have bss");
    // no PT_LOAD (phdrs exist but none is PT_LOAD — PT_NOTE is skipped)
    let e = build_elf(
        2,
        243,
        0,
        CODE_BASE as u64,
        &[SegSpec { ptype: 8, flags: 4, vaddr: CODE_BASE as u64, memsz: 16, data: vec![0u8; 16] }],
        false,
    );
    elf_err(&e, "no PT_LOAD segments");
    // filesz > memsz
    let e = build_elf(
        2,
        243,
        0,
        CODE_BASE as u64,
        &[SegSpec { ptype: 1, flags: 6, vaddr: CODE_BASE as u64, memsz: 8, data: vec![0u8; 32] }],
        false,
    );
    elf_err(&e, "p_filesz exceeds p_memsz");
    // PT_LOAD out of file (p_offset lives at phdr offset 8)
    let mut e = good_elf();
    e[64 + 8..64 + 16].copy_from_slice(&999999u64.to_le_bytes()); // p_offset
    elf_err(&e, "PT_LOAD out of file");
    // relocation section present
    let e = build_elf(
        2,
        243,
        0,
        CODE_BASE as u64,
        &[SegSpec { ptype: 1, flags: 5, vaddr: CODE_BASE as u64, memsz: 16, data: elf_halt_code() }],
        true,
    );
    elf_err(&e, "relocation section not allowed");
    // missing program headers
    let mut e = good_elf();
    e[56..58].copy_from_slice(&0u16.to_le_bytes()); // e_phnum = 0
    elf_err(&e, "missing program headers");
}

// ================================================================= D: decoder

trait EncWord {
    fn enc_bytes(&self) -> Vec<u8>;
}
impl EncWord for u32 {
    fn enc_bytes(&self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }
}
impl EncWord for [u8; 4] {
    fn enc_bytes(&self) -> Vec<u8> {
        self.to_vec()
    }
}

fn d32(word: impl EncWord) -> Inst {
    decode_at(&word.enc_bytes(), 0, CALC_PROFILE_RV64IM).unwrap().0
}
#[allow(dead_code)]
fn d32b(word: impl EncWord) -> Inst {
    decode_at(&word.enc_bytes(), 0, CALC_PROFILE_RV64IM_B).unwrap().0
}

/// D-1xx: full RV64IM whitelist decode vectors (field-assembler built; the
/// clang-18 anchors live in `isa_tests::decode_32bit_vectors`).
#[test]
fn decode_whitelist_full_vectors() {
    // LUI: positive and negative imm20
    assert_eq!(d32(enc_u(0x12345, 15, 0x37)), Inst::Lui { rd: 15, val: 0x12345000 });
    assert_eq!(d32(enc_u(0xFFFFF, 5, 0x37)), Inst::Lui { rd: 5, val: -4096 });
    assert_eq!(d32(enc_u(0x80000, 5, 0x37)), Inst::Lui { rd: 5, val: -2147483648 });
    // AUIPC
    assert_eq!(d32(enc_u(0x10000, 9, 0x17)), Inst::Auipc { rd: 9, val: 0x10000000 });
    // JAL forward / backward
    assert_eq!(d32(enc_j(|t: i64| t, 8, 1, 0x6F)), Inst::Jal { rd: 1, target: 8 });
    assert_eq!(d32(enc_j(|t: i64| t, -4096, 0, 0x6F)), Inst::Jal { rd: 0, target: -4096 });
    // JALR
    assert_eq!(d32(enc_i(-4, 4, 0, 3, 0x67)), Inst::Jalr { rd: 3, rs1: 4, imm: -4 });
    assert_eq!(d32(enc_i(2047, 1, 0, 1, 0x67)), Inst::Jalr { rd: 1, rs1: 1, imm: 2047 });
    // branches: all six kinds, negative and positive targets
    let br = |f3, t| d32(enc_b(t, 11, 10, f3, 0x63));
    assert_eq!(br(0, -8), Inst::Branch { kind: BrKind::Eq, rs1: 10, rs2: 11, target: -8 });
    assert_eq!(br(1, 4094), Inst::Branch { kind: BrKind::Ne, rs1: 10, rs2: 11, target: 4094 });
    assert_eq!(br(4, -2), Inst::Branch { kind: BrKind::Lt, rs1: 10, rs2: 11, target: -2 });
    assert_eq!(br(5, 16), Inst::Branch { kind: BrKind::Ge, rs1: 10, rs2: 11, target: 16 });
    assert_eq!(br(6, -4096), Inst::Branch { kind: BrKind::Ltu, rs1: 10, rs2: 11, target: -4096 });
    assert_eq!(br(7, 2046), Inst::Branch { kind: BrKind::Geu, rs1: 10, rs2: 11, target: 2046 });
    // loads: all seven widths
    let ld = |f3, imm, rd| d32(enc_i(imm, 10, f3, rd, 0x03));
    assert_eq!(ld(0, -3, 6), Inst::Load { op: LoadOp::Lb, rd: 6, rs1: 10, imm: -3 });
    assert_eq!(ld(1, 1, 6), Inst::Load { op: LoadOp::Lh, rd: 6, rs1: 10, imm: 1 });
    assert_eq!(ld(2, 0x100, 6), Inst::Load { op: LoadOp::Lw, rd: 6, rs1: 10, imm: 0x100 });
    assert_eq!(ld(3, -8, 6), Inst::Load { op: LoadOp::Ld, rd: 6, rs1: 10, imm: -8 });
    assert_eq!(ld(4, 7, 6), Inst::Load { op: LoadOp::Lbu, rd: 6, rs1: 10, imm: 7 });
    assert_eq!(ld(5, -2, 6), Inst::Load { op: LoadOp::Lhu, rd: 6, rs1: 10, imm: -2 });
    assert_eq!(ld(6, 4, 6), Inst::Load { op: LoadOp::Lwu, rd: 6, rs1: 10, imm: 4 });
    // stores: all four widths
    let st = |f3, imm, rs2| d32(enc_s(imm, rs2, 10, f3, 0x23));
    assert_eq!(st(0, -1, 11), Inst::Store { op: StoreOp::Sb, rs1: 10, rs2: 11, imm: -1 });
    assert_eq!(st(1, 2, 11), Inst::Store { op: StoreOp::Sh, rs1: 10, rs2: 11, imm: 2 });
    assert_eq!(st(2, 0x100, 11), Inst::Store { op: StoreOp::Sw, rs1: 10, rs2: 11, imm: 0x100 });
    assert_eq!(st(3, -8, 11), Inst::Store { op: StoreOp::Sd, rs1: 10, rs2: 11, imm: -8 });
    // op-imm: all six base forms
    let oi = |f3, imm, rd| d32(enc_i(imm, 10, f3, rd, 0x13));
    assert_eq!(oi(0, -5, 6), Inst::ImmAlu { op: ImmAluOp::Addi, rd: 6, rs1: 10, imm: -5 });
    assert_eq!(oi(2, -1, 6), Inst::ImmAlu { op: ImmAluOp::Slti, rd: 6, rs1: 10, imm: -1 });
    assert_eq!(oi(3, -1, 6), Inst::ImmAlu { op: ImmAluOp::Sltiu, rd: 6, rs1: 10, imm: -1 });
    assert_eq!(oi(4, 0xFF, 6), Inst::ImmAlu { op: ImmAluOp::Xori, rd: 6, rs1: 10, imm: 0xFF });
    assert_eq!(oi(6, 0x7F, 6), Inst::ImmAlu { op: ImmAluOp::Ori, rd: 6, rs1: 10, imm: 0x7F });
    assert_eq!(oi(7, -128, 6), Inst::ImmAlu { op: ImmAluOp::Andi, rd: 6, rs1: 10, imm: -128 });
    // shifts: immediate, all kinds + word forms, shamt boundaries
    let sh = |f6: u32, f3, shamt, word| {
        let imm = ((f6 << 6) | shamt) as i32;
        let op = if word { 0x1B } else { 0x13 };
        d32(enc_i(imm, 10, f3, 6, op))
    };
    assert_eq!(sh(0, 1, 0, false), Inst::ShiftImm { kind: ShiftKind::Sll, rd: 6, rs1: 10, shamt: 0, word: false });
    assert_eq!(sh(0, 1, 63, false), Inst::ShiftImm { kind: ShiftKind::Sll, rd: 6, rs1: 10, shamt: 63, word: false });
    assert_eq!(sh(0, 5, 32, false), Inst::ShiftImm { kind: ShiftKind::Srl, rd: 6, rs1: 10, shamt: 32, word: false });
    assert_eq!(sh(0x10, 5, 63, false), Inst::ShiftImm { kind: ShiftKind::Sra, rd: 6, rs1: 10, shamt: 63, word: false });
    assert_eq!(sh(0, 1, 31, true), Inst::ShiftImm { kind: ShiftKind::Sll, rd: 6, rs1: 10, shamt: 31, word: true });
    assert_eq!(sh(0, 5, 31, true), Inst::ShiftImm { kind: ShiftKind::Srl, rd: 6, rs1: 10, shamt: 31, word: true });
    assert_eq!(sh(0x10, 5, 31, true), Inst::ShiftImm { kind: ShiftKind::Sra, rd: 6, rs1: 10, shamt: 31, word: true });
    // addiw
    assert_eq!(d32(enc_i(-1, 10, 0, 6, 0x1B)), Inst::ImmAlu { op: ImmAluOp::Addiw, rd: 6, rs1: 10, imm: -1 });
    // register-register: base + M + W forms
    let rr = |f7, f3, rd, op| d32(enc_r(f7, 11, 10, f3, rd, op));
    assert_eq!(rr(0, 0, 6, 0x33), Inst::RegAlu { op: RegAluOp::Add, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0x20, 0, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sub, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sll, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 2, 6, 0x33), Inst::RegAlu { op: RegAluOp::Slt, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 3, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sltu, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 4, 6, 0x33), Inst::RegAlu { op: RegAluOp::Xor, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 5, 6, 0x33), Inst::RegAlu { op: RegAluOp::Srl, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0x20, 5, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sra, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 6, 6, 0x33), Inst::RegAlu { op: RegAluOp::Or, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 7, 6, 0x33), Inst::RegAlu { op: RegAluOp::And, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 0, 6, 0x33), Inst::RegAlu { op: RegAluOp::Mul, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Mulh, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 2, 6, 0x33), Inst::RegAlu { op: RegAluOp::Mulhsu, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 3, 6, 0x33), Inst::RegAlu { op: RegAluOp::Mulhu, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 4, 6, 0x33), Inst::RegAlu { op: RegAluOp::Div, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 5, 6, 0x33), Inst::RegAlu { op: RegAluOp::Divu, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 6, 6, 0x33), Inst::RegAlu { op: RegAluOp::Rem, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 7, 6, 0x33), Inst::RegAlu { op: RegAluOp::Remu, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 0, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Addw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0x20, 0, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Subw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 1, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Sllw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0, 5, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Srlw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(0x20, 5, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Sraw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 0, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Mulw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 4, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Divw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 5, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Divuw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 6, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Remw, rd: 6, rs1: 10, rs2: 11 });
    assert_eq!(rr(1, 7, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Remuw, rd: 6, rs1: 10, rs2: 11 });
    // fence / ecall / ebreak / unimp
    assert_eq!(d32(enc_i(0, 0, 0, 0, 0x0F)), Inst::Fence);
    assert_eq!(d32(0x73), Inst::Ecall);
    assert_eq!(d32(0x0010_0073), Inst::Trap);
    assert_eq!(d32(0xC000_1073), Inst::Trap);
}

/// JAL immediate assembler for the vector test above.
fn enc_j(imm: impl Fn(i64) -> i64, v: i64, rd: u32, op: u32) -> [u8; 4] {
    let u = imm(v) as u32;
    let w = (((u >> 20) & 1) << 31)
        | (((u >> 1) & 0x3FF) << 21)
        | (((u >> 11) & 1) << 20)
        | (((u >> 12) & 0xFF) << 12)
        | (rd << 7)
        | op;
    w.to_le_bytes()
}

/// D-2xx: bitmanip decodes under profile B (the decoder keeps the table for a
/// future profile id) and profile-0 rejection for a representative subset.
#[test]
fn decode_bitmanip_profile_b_and_v1_rejection() {
    let cases: Vec<([u8; 4], Inst)> = vec![
        (enc_i(0x600, 4, 1, 3, 0x13), Inst::UnAlu { op: UnAluOp::Clz, rd: 3, rs1: 4 }),
        (enc_i(0x601, 4, 1, 3, 0x13), Inst::UnAlu { op: UnAluOp::Ctz, rd: 3, rs1: 4 }),
        (enc_i(0x602, 4, 1, 3, 0x13), Inst::UnAlu { op: UnAluOp::Cpop, rd: 3, rs1: 4 }),
        (enc_i(0x604, 4, 1, 3, 0x13), Inst::UnAlu { op: UnAluOp::Sextb, rd: 3, rs1: 4 }),
        (enc_i(0x605, 4, 1, 3, 0x13), Inst::UnAlu { op: UnAluOp::Sexth, rd: 3, rs1: 4 }),
        // orc.b (OP-IMM funct3=5, funct6=001010, rs2 selector 00111)
        (enc_i(0b001010 << 6 | 0b00111, 4, 5, 3, 0x13), Inst::UnAlu { op: UnAluOp::Orcb, rd: 3, rs1: 4 }),
        // rev8 (funct6=011010, bit25=1, rs2 selector 11000)
        (enc_i(0b0110101 << 5 | 0b11000, 4, 5, 3, 0x13), Inst::UnAlu { op: UnAluOp::Rev8, rd: 3, rs1: 4 }),
        // rori / roriw
        (enc_i(0b011000 << 6 | 7, 4, 5, 3, 0x13), Inst::ShiftImm { kind: ShiftKind::Ror, rd: 3, rs1: 4, shamt: 7, word: false }),
        (enc_i(0b011000 << 6 | 7, 4, 5, 3, 0x1B), Inst::ShiftImm { kind: ShiftKind::Ror, rd: 3, rs1: 4, shamt: 7, word: true }),
        // bseti / bclri / binvi / bexti
        (enc_i(0b001010 << 6 | 5, 4, 1, 3, 0x13), Inst::BitImm { op: BitImmOp::Set, rd: 3, rs1: 4, shamt: 5 }),
        (enc_i(0b010010 << 6 | 5, 4, 1, 3, 0x13), Inst::BitImm { op: BitImmOp::Clr, rd: 3, rs1: 4, shamt: 5 }),
        (enc_i(0b011010 << 6 | 5, 4, 1, 3, 0x13), Inst::BitImm { op: BitImmOp::Inv, rd: 3, rs1: 4, shamt: 5 }),
        (enc_i(0b010010 << 6 | 5, 4, 5, 3, 0x13), Inst::BitImm { op: BitImmOp::Ext, rd: 3, rs1: 4, shamt: 5 }),
        // slli.uw
        (enc_i(0b000010 << 6 | 5, 4, 1, 3, 0x1B), Inst::Slliuw { rd: 3, rs1: 4, shamt: 5 }),
        // clzw / ctzw / cpopw
        (enc_i(0b011000 << 6, 4, 1, 3, 0x1B), Inst::UnAlu { op: UnAluOp::Clzw, rd: 3, rs1: 4 }),
        (enc_i(0b011000 << 6 | 1, 4, 1, 3, 0x1B), Inst::UnAlu { op: UnAluOp::Ctzw, rd: 3, rs1: 4 }),
        (enc_i(0b011000 << 6 | 2, 4, 1, 3, 0x1B), Inst::UnAlu { op: UnAluOp::Cpopw, rd: 3, rs1: 4 }),
        // zext.h: OP-32 only
        (enc_r(0b0000100, 0, 4, 4, 3, 0x3B), Inst::UnAlu { op: UnAluOp::Zexth, rd: 3, rs1: 4 }),
        // andn / orn / xnor
        (enc_r(0b0100000, 11, 10, 7, 6, 0x33), Inst::RegAlu { op: RegAluOp::Andn, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0100000, 11, 10, 6, 6, 0x33), Inst::RegAlu { op: RegAluOp::Orn, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0100000, 11, 10, 4, 6, 0x33), Inst::RegAlu { op: RegAluOp::Xnor, rd: 6, rs1: 10, rs2: 11 }),
        // rol / ror / rolw / rorw
        (enc_r(0b0110000, 11, 10, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Rol, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0110000, 11, 10, 5, 6, 0x33), Inst::RegAlu { op: RegAluOp::Ror, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0110000, 11, 10, 1, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Rolw, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0110000, 11, 10, 5, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Rorw, rd: 6, rs1: 10, rs2: 11 }),
        // min / minu / max / maxu
        (enc_r(0b0000101, 11, 10, 4, 6, 0x33), Inst::RegAlu { op: RegAluOp::Min, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0000101, 11, 10, 5, 6, 0x33), Inst::RegAlu { op: RegAluOp::Minu, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0000101, 11, 10, 6, 6, 0x33), Inst::RegAlu { op: RegAluOp::Max, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0000101, 11, 10, 7, 6, 0x33), Inst::RegAlu { op: RegAluOp::Maxu, rd: 6, rs1: 10, rs2: 11 }),
        // clmul / clmulr / clmulh
        (enc_r(0b0000101, 11, 10, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Clmul, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0000101, 11, 10, 2, 6, 0x33), Inst::RegAlu { op: RegAluOp::Clmulr, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0000101, 11, 10, 3, 6, 0x33), Inst::RegAlu { op: RegAluOp::Clmulh, rd: 6, rs1: 10, rs2: 11 }),
        // sh1add / sh2add / sh3add + .uw
        (enc_r(0b0010000, 11, 10, 2, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sh1add, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0010000, 11, 10, 4, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sh2add, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0010000, 11, 10, 6, 6, 0x33), Inst::RegAlu { op: RegAluOp::Sh3add, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0010000, 11, 10, 2, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Sh1adduw, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0010000, 11, 10, 4, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Sh2adduw, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0010000, 11, 10, 6, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Sh3adduw, rd: 6, rs1: 10, rs2: 11 }),
        // add.uw
        (enc_r(0b0000100, 11, 10, 0, 6, 0x3B), Inst::RegAlu { op: RegAluOp::Adduw, rd: 6, rs1: 10, rs2: 11 }),
        // bset / bclr / binv / bext
        (enc_r(0b0010100, 11, 10, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Bset, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0100100, 11, 10, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Bclr, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0110100, 11, 10, 1, 6, 0x33), Inst::RegAlu { op: RegAluOp::Binv, rd: 6, rs1: 10, rs2: 11 }),
        (enc_r(0b0100100, 11, 10, 5, 6, 0x33), Inst::RegAlu { op: RegAluOp::Bext, rd: 6, rs1: 10, rs2: 11 }),
    ];
    for (word, want) in cases {
        let word = word.enc_bytes();
        let word_u = u32::from_le_bytes(word[..4].try_into().unwrap());
        match decode_at(&word, 0, CALC_PROFILE_RV64IM_B) {
            Ok(got) => assert_eq!(got.0, want, "profile B decode of {word_u:#010x}"),
            Err(e) => panic!("profile B decode of {word_u:#010x} failed: {}", e.1),
        }
        // v1 rejects the whole bitmanip surface on profile 0
        let err = decode_at(&word, 0, CALC_PROFILE_RV64IM).unwrap_err();
        assert!(
            err.1.contains("bitmanip instruction not in this profile"),
            "{word_u:#010x}: {}",
            err.1
        );
    }
}

/// D-3xx: every decode fallback error path.
#[test]
fn decode_rejection_matrix() {
    fn err(word: impl EncWord, part: &str) {
        let bytes = word.enc_bytes();
        let e = decode_at(&bytes, 0, CALC_PROFILE_RV64IM_B).unwrap_err();
        let word_u = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        assert!(e.1.contains(part), "{word_u:#010x}: `{}` must contain `{part}`", e.1);
    }
    err(enc_i(0, 4, 1, 3, 0x67), "jalr funct3");
    err(enc_b(8, 11, 10, 2, 0x63), "branch funct3");
    err(enc_b(8, 11, 10, 3, 0x63), "branch funct3");
    err(enc_i(0, 10, 7, 6, 0x03), "load funct3");
    err(enc_s(0, 11, 10, 4, 0x23), "store funct3");
    // slli with a funct6 outside the whitelist
    err(enc_i(0b1111111 << 5 | 1, 4, 1, 3, 0x13), "slli/zbb-imm funct6");
    // srli slot with a bad funct6
    err(enc_i(0b1111111 << 5 | 1, 4, 5, 3, 0x13), "srli/srai/rori funct6");
    // zbb unary with a bad rs2 selector
    err(enc_i(0b011000 << 6 | 3, 4, 1, 3, 0x13), "zbb unary rs2 selector");
    // slliw with bit25 set (RV64 reserved)
    err(enc_i(0b100000 << 6 | 1, 4, 1, 3, 0x1B), "slliw");
    // sraiw slot with a bad funct7
    err(enc_i(0b1111111 << 5, 4, 5, 3, 0x1B), "srliw/sraiw/roriw funct7");
    // OP with bad funct7/funct3
    err(enc_r(0b1111111, 1, 2, 0, 3, 0x33), "op funct7/funct3");
    // OP-32 with bad funct7/funct3
    err(enc_r(0b1111111, 1, 2, 0, 3, 0x3B), "op-32 funct7/funct3");
    // zext.h in the RV32 OP form is illegal
    err(enc_r(0b0000100, 0, 4, 4, 3, 0x33), "op funct7/funct3");
    // fence.i
    err(enc_i(0, 0, 1, 0, 0x0F), "fence funct3");
    // CSR (csrrs x3, cycle, x0)
    err(0xC000_21F3, "system instruction not allowed");
    // A extension / F extension / custom opcode
    err(enc_r(1, 1, 2, 2, 3, 0x2F), "opcode not in RV64IM whitelist");
    err(enc_r(1, 1, 2, 0, 3, 0x07), "opcode not in RV64IM whitelist");
    err(enc_r(1, 1, 2, 0, 3, 0x0B), "opcode not in RV64IM whitelist");
    // compressed encodings in a no-C decode
    for half in [0x0001u16, 0x0808, 0x9002, 0x0000] {
        let bytes = [(half & 0xFF) as u8, (half >> 8) as u8, 0, 0];
        let e = decode_at(&bytes, 0, CALC_PROFILE_RV64IM).unwrap_err();
        assert!(e.1.contains("compressed instruction not allowed"), "{half:#06x}: {}", e.1);
    }
    // misaligned / truncated fetch
    let code = [0x13u8, 0, 0, 0, 0x13, 0, 0, 0];
    let e = decode_at(&code, 2, CALC_PROFILE_RV64IM).unwrap_err();
    assert!(e.1.contains("not 4-byte aligned"), "{}", e.1);
    let e = decode_at(&code[..3], 0, CALC_PROFILE_RV64IM).unwrap_err();
    assert!(e.1.contains("truncated"), "{}", e.1);
    // unknown profile byte
    let e = decode_at(&code, 0, 9).unwrap_err();
    assert!(e.1.contains("unknown isa profile"), "{}", e.1);
}

// ================================================================= E/F: memory regions

/// Region sample addresses for the mem_kib=8 / 64+64 IO layout:
/// mem_len = 0x3000, input_base = 0x2FC0, output_base = 0x2F80,
/// code [0x1000, code_end), hole [code_end, 0x2000), RW [0x2000, 0x2080),
/// RO [0x2080, 0x20C0), stack [0x20C0, 0x2F80) — no guard band, the stack
/// starts exactly at the end of the guest image.
const LOW: u64 = 0x0800;
const STUB: u64 = 0x0FF0;
const CODE0: u64 = 0x1000;
const HOLE: u64 = 0x1800;
const RW0: u64 = 0x2010;
const RO0: u64 = 0x2088;
const GUEST_END0: u64 = 0x20C8;
const STACK0: u64 = 0x2200;
const OUT0: u64 = 0x2F88;
const IN0: u64 = 0x2FC8;
const BEYOND: u64 = 0x3004;

fn matrix_container(code: Vec<u8>) -> CalcContainer {
    CalcContainer {
        profile: CALC_PROFILE_RV64IM,
        mem_kib: 8,
        entry: CODE_BASE as u32,
        segs: vec![
            CalcSegment::rx(CODE_BASE as u32, code),
            CalcSegment::rw(0x2000, vec![0u8; 0x80], 0x80),
            CalcSegment::ro(0x2080, vec![0u8; 0x40]),
        ],
        input_max: 64,
        output_max: 64,
    }
}

/// F-001 cell: run one access against one region through interp + lockstep.
fn access_case(kind: AccessKind, addr: u64) -> Result<CalcRun, (CalcAbort, u64)> {
    let mut code = li(5, addr); // li t0, addr
    match kind {
        AccessKind::Load => code.extend_from_slice(&enc_i(0, 5, 3, 6, 0x03)), // ld t1, 0(t0)
        AccessKind::Store => code.extend_from_slice(&enc_s(0, 5, 5, 3, 0x23)), // sd t0, 0(t0)
        AccessKind::Fetch => code.extend_from_slice(&enc_i(0, 5, 0, 0, 0x67)), // jalr x0, 0(t0)
    }
    code.extend_from_slice(&halt_empty());
    let c = matrix_container(code);
    let m = verify_container(&c);
    let res = interp::run(&m, &c, &[], 10_000, None);
    // every cell must also agree in lockstep
    let (lock_res, lock_cycles) = lockstep(&c, &[], 10_000);
    match &res {
        Ok(r) => {
            assert!(
                matches!(lock_res, Ok(ref o) if o == &r.output),
                "fetch/load/store ok: lockstep output mismatch"
            );
            assert_eq!(lock_cycles, setup_cycles(&c) + r.cycles_used);
        }
        Err((CalcAbort::Trap(_), used)) => {
            assert!(matches!(lock_res, Err(CalcAbort2::Trap)), "lockstep class mismatch");
            assert_eq!(lock_cycles, setup_cycles(&c) + used);
        }
        Err((CalcAbort::OutOfGas, used)) => {
            assert!(matches!(lock_res, Err(CalcAbort2::OutOfGas)));
            assert_eq!(lock_cycles, setup_cycles(&c) + used);
        }
        Err((CalcAbort::GuestExit(_), used)) => {
            assert!(matches!(lock_res, Err(CalcAbort2::Trap)));
            assert_eq!(lock_cycles, setup_cycles(&c) + used);
        }
        Err((CalcAbort::Deadline, _)) => unreachable!(),
    }
    res
}

#[derive(Clone, Copy)]
enum AccessKind {
    Load,
    Store,
    Fetch,
}

/// F-001: the full region x operation permission matrix.
#[test]
fn permission_matrix_load_store_fetch() {
    use AccessKind::*;
    // (region, load expectation) — loads: Ok everywhere mapped.
    for (name, addr, ok) in [
        ("low unmapped", LOW, false),
        ("halt stub", STUB, true),
        ("code rx", CODE0, true),
        ("hole", HOLE, false),
        ("rw", RW0, true),
        ("ro", RO0, true),
        ("stack head", GUEST_END0, true),
        ("stack", STACK0, true),
        ("output", OUT0, true),
        ("input", IN0, true),
        ("beyond mem", BEYOND, false),
    ] {
        let res = access_case(Load, addr);
        assert_eq!(res.is_ok(), ok, "load {name}: {res:?}");
        if !ok {
            expect_trap(res, "unmapped");
        }
    }
    // stores: only RW-classified regions accept writes.
    for (name, addr, msg) in [
        ("low unmapped", LOW, Some("read-only or unmapped")),
        ("halt stub", STUB, Some("read-only or unmapped")),
        ("code rx", CODE0, Some("read-only or unmapped")),
        ("hole", HOLE, Some("read-only or unmapped")),
        ("rw", RW0, None),
        ("ro", RO0, Some("read-only or unmapped")),
        ("stack head", GUEST_END0, None),
        ("stack", STACK0, None),
        ("output", OUT0, None),
        ("input", IN0, None),
        ("beyond mem", BEYOND, Some("read-only or unmapped")),
    ] {
        let res = access_case(Store, addr);
        match msg {
            None => assert!(res.is_ok(), "store {name} must succeed: {res:?}"),
            Some(m) => expect_trap(res, m),
        }
    }
    // fetch: only instruction boundaries inside RX segments or the HALT
    // stub are valid targets; everything else traps (see fetch tests below).
    for (_name, addr) in [
        ("low unmapped", LOW),
        ("hole", HOLE),
        ("rw", RW0),
        ("ro", RO0),
        ("stack head", GUEST_END0),
        ("stack", STACK0),
        ("output", OUT0),
        ("input", IN0),
        ("beyond mem", BEYOND),
    ] {
        expect_trap(access_case(Fetch, addr), "jalr to invalid target");
    }
}

/// F-001/E-301: fetch into a code boundary and into the HALT stub succeed.
#[test]
fn fetch_into_code_boundary_and_halt_stub() {
    // li t0, TARGET ; jalr x0,0(t0) ; TARGET: halt — jump over the jalr
    let target = CODE_BASE as u64 + 12;
    let mut code = li(5, target);
    code.extend_from_slice(&enc_i(0, 5, 0, 0, 0x67));
    code.extend_from_slice(&halt_empty());
    let res = run(code, &[], 1000).unwrap();
    assert_eq!(res.output, Vec::<u8>::new());

    // fetch to the HALT stub: a0=0, a1=0 -> clean HALT through the trampoline
    let mut code = enc_i(0, 0, 0, 10, 0x13).to_vec(); // addi a0, x0, 0
    code.extend_from_slice(&enc_i(0, 0, 0, 11, 0x13)); // addi a1, x0, 0
    code.extend_from_slice(&li(5, STUB));
    code.extend_from_slice(&enc_i(0, 5, 0, 0, 0x67));
    code.extend_from_slice(&halt_empty());
    let res = run(code, &[], 1000).unwrap();
    assert_eq!(res.output, Vec::<u8>::new());
}

/// E-202: misaligned accesses crossing region boundaries — the full
/// [addr, addr+size) range is permission-checked.
#[test]
fn misaligned_cross_region_access() {
    let case_w = |addr: u64, off: i32, load: bool, f3: u32| -> Result<CalcRun, (CalcAbort, u64)> {
        let mut code = li(5, addr);
        if load {
            code.extend_from_slice(&enc_i(off, 5, f3, 6, 0x03));
        } else {
            code.extend_from_slice(&enc_s(off, 5, 5, f3, 0x23));
        }
        code.extend_from_slice(&halt_empty());
        let c = matrix_container(code);
        let m = verify_container(&c);
        interp::run(&m, &c, &[], 10_000, None)
    };
    let case = |addr: u64, off: i32, load: bool| case_w(addr, off, load, 3); // 8-byte ld/sd
    // load RW tail crossing into RO: both readable -> ok
    assert!(case(0x2080, -4, true).is_ok());
    // load RO tail crossing into the stack start: stack is readable -> ok
    assert!(case(0x20C4, -8, true).is_ok());
    // load stack tail crossing into the output region: RW -> ok
    assert!(case(0x2F80, -4, true).is_ok());
    // load crossing the end of memory -> trap
    expect_trap(case(0x3000, -4, true), "unmapped");
    // load entirely inside the hole -> trap
    expect_trap(case(0x2000, -8, true), "unmapped");
    // load straddling hole -> RW boundary -> trap
    expect_trap(case(0x2000, -4, true), "unmapped");
    // store RW tail crossing into RO -> trap
    expect_trap(case(0x2080, -4, false), "read-only");
    // store entirely inside RW up to its end -> ok
    assert!(case(0x2080, -8, false).is_ok());
    // store RO tail crossing into the stack start: RO byte -> trap read-only
    expect_trap(case(0x20C4, -8, false), "read-only");
    // store entirely inside the stack at its start -> ok (no guard band)
    assert!(case(0x20C4, -4, false).is_ok());
    // store at the former guard/stack boundary -> plain stack RW now
    assert!(case(0x21C0, -4, false).is_ok());
    // store at stack bottom -> ok
    assert!(case(0x21C0, 0, false).is_ok());
    // 4-byte store ending exactly at mem_len -> ok (input region)
    assert!(case_w(0x3000, -4, false, 2).is_ok());
    // 8-byte store crossing mem_len -> trap
    expect_trap(case(0x3000, -4, false), "read-only or unmapped");
}

/// E-203: register-computed wraparound addresses.
#[test]
fn address_wraparound_traps() {
    // t0 = -1 (all ones): 8-byte load overflows the checked address add
    let mut code = enc_i(-1, 0, 0, 5, 0x13).to_vec();
    code.extend_from_slice(&enc_i(0, 5, 3, 6, 0x03));
    code.extend_from_slice(&halt_empty());
    expect_trap(run(code, &[], 1000), "load address overflow");

    let mut code = enc_i(-1, 0, 0, 5, 0x13).to_vec();
    code.extend_from_slice(&enc_s(0, 5, 5, 3, 0x23));
    code.extend_from_slice(&halt_empty());
    expect_trap(run(code, &[], 1000), "store address overflow");

    // jalr to a >32-bit target must not alias into the guest space
    let mut code = enc_i(-1, 0, 0, 5, 0x13).to_vec();
    code.extend_from_slice(&enc_i(0, 5, 0, 0, 0x67));
    code.extend_from_slice(&halt_empty());
    expect_trap(run(code, &[], 1000), "jalr to invalid target");

    // far-beyond-memory positive address (u32 boundary region)
    let mut code = li(5, 0x7FFF_F000);
    code.extend_from_slice(&enc_i(0, 5, 3, 6, 0x03));
    code.extend_from_slice(&halt_empty());
    expect_trap(run(code, &[], 1000), "unmapped");
}

/// E-302: register state at entry matches SPEC §5 exactly.
#[test]
fn entry_register_snapshot() {
    // dump ra/sp/a0/a1/a3 into the output region, then HALT with n=40
    let mut code = Vec::new();
    code.extend_from_slice(&enc_s(0, 1, 12, 3, 0x23)); // sd ra, 0(a2)
    code.extend_from_slice(&enc_s(8, 2, 12, 3, 0x23)); // sd sp, 8(a2)
    code.extend_from_slice(&enc_s(16, 10, 12, 3, 0x23)); // sd a0, 16(a2)
    code.extend_from_slice(&enc_s(24, 11, 12, 3, 0x23)); // sd a1, 24(a2)
    code.extend_from_slice(&enc_s(32, 13, 12, 3, 0x23)); // sd a3, 32(a2)
    code.extend_from_slice(&enc_i(40, 0, 0, 11, 0x13)); // addi a1, x0, 40
    code.extend_from_slice(&halt_keep_a1());
    let input = b"hello";
    let res = run(code, input, 1000).unwrap();
    assert_eq!(res.output.len(), 40);
    let rd = |off: usize| -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&res.output[off..off + 8]);
        u64::from_le_bytes(b)
    };
    let c = code_container(halt_empty());
    let lay = c.layout();
    assert_eq!(rd(0), HALT_STUB, "ra must be the HALT trampoline");
    assert_eq!(rd(8), lay.stack_top, "sp must be stack_top");
    assert_eq!(rd(16), lay.input_base, "a0 must be input_base");
    assert_eq!(rd(24), input.len() as u64, "a1 must be the input length");
    assert_eq!(rd(32), 64, "a3 must be output_max");
}

/// E-303: input echo across the declared input_max sweep.
#[test]
fn input_echo_sweep() {
    // lb t0,0(a0); sb t0,0(a2); halt with a1=1
    let mut code = enc_i(0, 10, 0, 5, 0x03).to_vec(); // lb t0, 0(a0)
    code.extend_from_slice(&enc_s(0, 5, 12, 0, 0x23)); // sb t0, 0(a2)
    code.extend_from_slice(&enc_i(1, 0, 0, 11, 0x13)); // addi a1, x0, 1
    code.extend_from_slice(&halt_keep_a1());
    for len in [0usize, 1, 15, 16, 31, 63, 64] {
        let input: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
        if len == 0 {
            // lb of the first input byte still reads a mapped byte; echo is
            // well-defined for len 0 input too (reads a stale byte, writes it
            // out). Just require success.
            assert!(run(code.clone(), &input, 1000).is_ok());
            continue;
        }
        let res = run(code.clone(), &input, 1000).unwrap();
        assert_eq!(res.output, vec![input[0]]);
    }
    // input above input_max is a 0-cycle pre-flight trap
    let big = vec![0u8; 65];
    let c = code_container(code);
    let obj = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    let exec = obj.run(
        &big,
        CalcRunLimits {
            cycles: 10_000,
            resource: -1,
            deadline: None,
        },
    );
    assert!(matches!(exec.result, Err(CalcAbort2::Trap)));
    assert_eq!(exec.cycles_used, 0, "pre-flight input rejection charges nothing");
    assert_eq!(exec.resource, 0);
}

// ================================================================= E: ecall branches

/// E-401/E-403: HALT output-length sweep and check ordering.
#[test]
fn halt_output_length_branches() {
    let halt_with_len = |n: i32| {
        let mut code = enc_i(n, 0, 0, 11, 0x13).to_vec(); // addi a1, x0, n
        code.extend_from_slice(&halt_keep_a1());
        code
    };
    for n in [0, 1, 63, 64] {
        let res = run(halt_with_len(n), b"x", 1000).unwrap();
        assert_eq!(res.output.len(), n as usize);
    }
    expect_trap(run(halt_with_len(65), b"x", 1000), "output length exceeds output_max");

    // ordering: n > output_max must trap even when a0 is nonzero
    let mut code = enc_i(7, 0, 0, 10, 0x13).to_vec(); // addi a0, x0, 7
    code.extend_from_slice(&enc_i(65, 0, 0, 11, 0x13)); // addi a1, x0, 65
    code.extend_from_slice(&[0x73, 0, 0, 0]);
    expect_trap(run(code, b"x", 1000), "output length exceeds output_max");
}

/// E-404/E-405: every non-HALT ecall number traps on a profile-0 image.
#[test]
fn ecall_number_branch_matrix() {
    for a7 in [1i32, 2, 3, 4, 5, 6] {
        let mut code = enc_i(a7, 0, 0, 17, 0x13).to_vec();
        code.extend_from_slice(&[0x73, 0, 0, 0]);
        expect_trap(run(code, &[], 1000), "hash builtins not declared");
    }
    // reserved 7..=15, HINT 16..=31, unknown >= 32
    // on a profile-0 image the hash gate fires before HINT/unknown
    // classification, so every non-HALT ecall number traps the same way.
    for a7 in [7i32, 8, 15, 16, 20, 31, 32, 63, 2047, -1] {
        let mut code = enc_i(a7, 0, 0, 17, 0x13).to_vec();
        code.extend_from_slice(&[0x73, 0, 0, 0]);
        expect_trap(run(code, &[], 1000), "hash builtins not declared");
    }
}

/// E-407: hash builtin unit surface (v1 traps them on the guest path; the
/// implementations stay for a future profile id — this pins their semantics).
#[test]
fn hash_builtins_unit_surface() {
    use vm::calc::ECALL_HALT;
    assert_eq!(ECALL_HALT, 0);
    for a7 in 1u64..=6 {
        assert!(HashKind::from_a7(a7).is_some());
    }
    for a7 in [0u64, 7, 8, 15, 16, 31, 32, 100] {
        assert!(HashKind::from_a7(a7).is_none(), "a7={a7}");
    }
    // cost formula: 64 + ceil(len/64) * per_block
    let c = |k: HashKind, len: u64| k.cycles(len);
    assert_eq!(c(HashKind::Sha2, 0), 64);
    assert_eq!(c(HashKind::Sha2, 1), 64 + 96);
    assert_eq!(c(HashKind::Sha2, 64), 64 + 96);
    assert_eq!(c(HashKind::Sha2, 65), 64 + 2 * 96);
    assert_eq!(c(HashKind::Sha3, 64), 64 + 128);
    assert_eq!(c(HashKind::Ripemd160, 64), 64 + 128);
    assert_eq!(c(HashKind::Keccak256, 64), 64 + 128);
    assert_eq!(c(HashKind::Blake2s256, 64), 64 + 64);
    assert_eq!(c(HashKind::Blake2b256, 64), 64 + 64);
    // digest lengths
    assert_eq!(HashKind::Ripemd160.digest_len(), 20);
    for k in [HashKind::Sha2, HashKind::Sha3, HashKind::Keccak256, HashKind::Blake2s256, HashKind::Blake2b256] {
        assert_eq!(k.digest_len(), 32);
    }
    // sha256("abc") golden vector through run_hash
    let mut out = [0u8; 32];
    run_hash(HashKind::Sha2, b"abc", &mut out);
    assert_eq!(
        out,
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad
        ]
    );
}

// ================================================================= I: AOT parity

/// I-001: clean-return kernels over every instruction class must agree
/// between interpreter and AOT on output, cycles and abort class.
#[test]
fn clean_return_kernels_aot_parity() {
    // 1. echo with a length register (a1 = 1)
    let mut echo = enc_i(0, 10, 0, 5, 0x03).to_vec(); // lb t0, 0(a0)
    echo.extend_from_slice(&enc_s(0, 5, 12, 0, 0x23)); // sb t0, 0(a2)
    echo.extend_from_slice(&enc_i(1, 0, 0, 11, 0x13)); // addi a1, x0, 1
    echo.extend_from_slice(&halt_keep_a1());

    // 2. arithmetic class mix: mul/div/shift/slt with results written out
    let mut arith = Vec::new();
    arith.extend_from_slice(&enc_r(1, 0, 0, 0, 5, 0x33)); // mul t0, x0, x0
    arith.extend_from_slice(&enc_r(1, 0, 0, 4, 6, 0x33)); // div t1, x0, x0
    arith.extend_from_slice(&enc_i(63, 5, 1, 5, 0x13)); // slli t0, t0, 63
    arith.extend_from_slice(&enc_i(-1, 0, 0, 7, 0x13)); // addi t2, x0, -1
    arith.extend_from_slice(&enc_r(0, 7, 5, 2, 8, 0x33)); // slt t3, t0, t2 -> 0
    arith.extend_from_slice(&enc_s(0, 8, 12, 3, 0x23)); // sd t3, 0(a2)
    arith.extend_from_slice(&enc_i(8, 0, 0, 11, 0x13)); // addi a1, x0, 8
    arith.extend_from_slice(&halt_keep_a1());

    // 3. branch loop: sum 1..=10 through t0/t1 and write it out
    let mut branch_loop = Vec::new();
    branch_loop.extend_from_slice(&enc_i(0, 0, 0, 5, 0x13)); // addi t0, x0, 0
    branch_loop.extend_from_slice(&enc_i(0, 0, 0, 6, 0x13)); // addi t1, x0, 0
    let loop_head = branch_loop.len() as i64;
    branch_loop.extend_from_slice(&enc_i(1, 6, 0, 6, 0x13)); // addi t1, t1, 1
    branch_loop.extend_from_slice(&enc_r(0, 6, 5, 0, 5, 0x33)); // add t0, t0, t1
    // blt t1, t2, loop_head (signed backward branch)
    branch_loop.extend_from_slice(&enc_i(10, 0, 0, 7, 0x13)); // addi t2, x0, 10
    let back = loop_head - branch_loop.len() as i64;
    branch_loop.extend_from_slice(&enc_b(back as i32, 7, 6, 4, 0x63)); // blt t1, t2, loop
    branch_loop.extend_from_slice(&enc_s(0, 5, 12, 3, 0x23)); // sd t0, 0(a2)
    branch_loop.extend_from_slice(&enc_i(8, 0, 0, 11, 0x13)); // addi a1, x0, 8
    branch_loop.extend_from_slice(&halt_keep_a1());

    // 4. deep stack use: push/pop below sp
    let mut stack = Vec::new();
    stack.extend_from_slice(&enc_s(-8, 5, 2, 3, 0x23)); // sd t0, -8(sp)
    stack.extend_from_slice(&enc_i(-8, 2, 3, 5, 0x03)); // ld t0, -8(sp)
    stack.extend_from_slice(&enc_s(0, 5, 12, 3, 0x23)); // sd t0, 0(a2)
    stack.extend_from_slice(&enc_i(8, 0, 0, 11, 0x13)); // addi a1, x0, 8
    stack.extend_from_slice(&halt_keep_a1());

    let cases: Vec<(String, Vec<u8>, Vec<u8>)> = vec![
        ("echo".into(), echo, b"Q".to_vec()),
        ("arith".into(), arith, vec![]),
        ("branch_loop".into(), branch_loop, vec![]),
        ("stack".into(), stack, vec![]),
    ];
    for (name, code, input) in cases {
        let c = code_container(code);
        let m = verify_container(&c);
        let r = interp::run(&m, &c, &input, 10_000, None)
            .unwrap_or_else(|e| panic!("{name}: interp failed {e:?}"));
        let (lock_out, lock_cycles) = lockstep(&c, &input, 10_000);
        assert_eq!(lock_out.as_ref().ok(), Some(&r.output), "{name}: output mismatch");
        assert_eq!(lock_cycles, setup_cycles(&c) + r.cycles_used, "{name}: cycles mismatch");
    }
}

/// I-001: the permission matrix and boundary kernels double as lockstep
/// coverage (asserted inside `access_case`); here we add the trap-side
/// kernels so both abort classes are exercised through CalcFnObj too.
#[test]
fn trap_class_kernels_aot_parity() {
    // store into the read-only segment through CalcFnObj: Trap, cycles =
    // setup + consumed (the old guard-band store-trap cell became a legal
    // stack write when the guard was removed)
    let c = matrix_container({
        let mut code = li(5, RO0);
        code.extend_from_slice(&enc_s(0, 5, 5, 3, 0x23));
        code.extend_from_slice(&halt_empty());
        code
    });
    let obj = CalcFnObj::build_with_aot(c.clone()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 10_000,
            resource: -1,
            deadline: None,
        },
    );
    assert!(matches!(exec.result, Err(CalcAbort2::Trap)));
    // the kernel is one basic block (no internal terminators): li(1+1) +
    // sd(2) + halt(4) = 8 cycles prepaid at block entry, trap keeps them all
    assert_eq!(exec.cycles_used, setup_cycles(&c) + 8);

    // OutOfGas boundary: exact budget boundary sweep on the self-loop
    let c = code_container(0x6F_u32.to_le_bytes().to_vec()); // jal x0, 0
    let obj = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    for budget in [setup_cycles(&c) + 1, setup_cycles(&c) + 4] {
        let exec = obj.run(
            &[],
            CalcRunLimits {
                cycles: budget,
                resource: -1,
                deadline: None,
            },
        );
        assert!(matches!(exec.result, Err(CalcAbort2::OutOfGas)));
        assert_eq!(exec.cycles_used, budget);
    }
}
