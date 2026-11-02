//! v1 execution semantics are unconditional and pinned: every ISA profile
//! cell is admitted, hash builtins run and are charged, rodata/the
//! alignment hole/code are read-only, the stack guard band traps, and any
//! instruction-boundary entry point is accepted. `CalcConfig` no longer has
//! switches for these — turning one off would reject or reinterpret
//! previously legal containers — so these tests pin the behaviors directly.
//! `CalcFnObj::build` is pinned to `CALC_CONFIG_V1` (test/testkit-gated).

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::{
    CalcFnObj, CalcHostConfig, CalcRunLimits, HashKind, CALC_CODE_BASE, CALC_CONFIG_V1,
    CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC, CALC_PROFILE_RV64IMC_B, CALC_PROFILE_RV64IM_B,
};

fn push(code: &mut Vec<u8>, w: u32) {
    code.extend_from_slice(&w.to_le_bytes());
}

fn w_addi(rd: u8, rs1: u8, imm: i32) -> u32 {
    0x13u32 | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (((imm as u32) & 0xFFF) << 20)
}

fn w_lui(rd: u8, imm20: u32) -> u32 {
    0x37u32 | ((rd as u32) << 7) | ((imm20 & 0xFFFFF) << 12)
}

fn w_sb(rs1: u8) -> u32 {
    0x23u32 | ((rs1 as u32) << 15)
}

fn li(code: &mut Vec<u8>, rd: u8, addr: u64) {
    let mut hi = (addr >> 12) as i64;
    let mut lo = (addr & 0xFFF) as i64;
    if lo >= 0x800 {
        hi += 1;
        lo -= 0x1000;
    }
    push(code, w_lui(rd, hi as u32));
    push(code, w_addi(rd, rd, lo as i32));
}

fn halt(code: &mut Vec<u8>) {
    push(code, w_addi(10, 0, 0));
    push(code, w_addi(11, 0, 0));
    push(code, w_addi(17, 0, 0));
    push(code, 0x0000_0073);
}

fn halt_code() -> Vec<u8> {
    let mut code = Vec::new();
    halt(&mut code);
    code
}

fn container(profile: u8, entry: u32, code: Vec<u8>, rodata: Vec<u8>) -> CalcContainer {
    CalcContainer {
        profile,
        abi: 1,
        mem_kib: 8,
        entry,
        code,
        rodata,
        init: vec![],
        bss_len: 0,
        input_max: 64,
        output_max: 64,
    }
}

fn limits() -> CalcRunLimits {
    CalcRunLimits {
        cycles: 1_000_000,
        resource: i64::MAX,
        deadline: None,
    }
}

fn err_text(r: impl std::fmt::Debug) -> String {
    format!("{r:?}")
}

fn store_at(addr: u64) -> Vec<u8> {
    let mut code = Vec::new();
    li(&mut code, 5, addr);
    push(&mut code, w_sb(5));
    halt(&mut code);
    code
}

#[test]
fn build_admits_every_profile_cell() {
    for profile in [
        CALC_PROFILE_RV64IM,
        CALC_PROFILE_RV64IM_B,
        CALC_PROFILE_RV64IMC,
        CALC_PROFILE_RV64IMC_B,
    ] {
        let c = container(profile, CALC_CODE_BASE as u32, halt_code(), vec![]);
        assert!(CalcFnObj::build(c).is_ok(), "profile {profile}");
    }
}

#[test]
fn hash_builtins_run_and_are_charged() {
    let mut code = Vec::new();
    push(&mut code, w_addi(17, 0, 1));
    push(&mut code, 0x0000_0073);
    halt(&mut code);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, code, vec![]);
    let obj = CalcFnObj::build(c).unwrap();
    let exec = obj.run(&[], limits());
    assert!(exec.result.is_ok(), "{exec:?}");
    // First block addi+ecall (2), hash of 0 bytes (64), halt block (4), plus
    // the per-call setup charge (8 KiB container).
    let hash = HashKind::Sha2.cycles(0);
    assert_eq!(
        exec.cycles_used,
        hash + 2 + 4 + vm::calc::CALC_SETUP_BASE_CYCLES + 8 * 16
    );
}

#[test]
fn rodata_hole_and_code_stores_trap() {
    let code_len = 40u64;
    let rodata_base = (CALC_CODE_BASE + code_len + 15) & !15;
    let hole = rodata_base + 1;
    let data_base = (rodata_base + 1 + 15) & !15;
    assert!(hole < data_base);
    let mut code = Vec::new();
    li(&mut code, 5, rodata_base);
    push(&mut code, w_sb(5));
    li(&mut code, 6, hole);
    push(&mut code, w_sb(6));
    halt(&mut code);
    assert_eq!(code.len() as u64, code_len);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, code, vec![0xAB]);
    let on = CalcFnObj::build(c).unwrap();
    assert!(on.run(&[], limits()).result.is_err(), "rodata and hole stores trap");

    let code_store = container(
        CALC_PROFILE_RV64IM,
        CALC_CODE_BASE as u32,
        store_at(CALC_CODE_BASE),
        vec![],
    );
    let v1 = CalcFnObj::build(code_store).unwrap();
    assert!(v1.run(&[], limits()).result.is_err(), "code stays non-writable");
}

#[test]
fn stack_guard_band_traps() {
    // 28-byte store_at body: guard starts at align16(CODE_BASE+28) = 0x1020.
    let guard = 0x1020u64;
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, store_at(guard), vec![]);
    let lay = c.layout();
    assert_eq!(lay.guard_base, guard);
    let on = CalcFnObj::build(c).unwrap();
    assert!(on.run(&[], limits()).result.is_err(), "guard band traps");
}

#[test]
fn free_entry_accepts_any_instruction_boundary() {
    let mut code = vec![0x13, 0x00, 0x00, 0x00];
    halt(&mut code);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32 + 4, code, vec![]);
    let obj = CalcFnObj::build(c).unwrap();
    assert!(obj.run(&[], limits()).result.is_ok(), "entry past the first instruction runs");
    // An entry that is not an instruction boundary still fails header checks.
    let mut bad = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32 + 2, halt_code(), vec![]);
    let err = CalcFnObj::build(bad.clone()).unwrap_err();
    assert!(err_text(&err).contains("aligned"), "{err:?}");
    bad.entry = CALC_CODE_BASE as u32;
    assert!(CalcFnObj::build(bad).is_ok());
}

#[test]
fn image_id_lazy_equals_eager() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let lazy = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    let eager = CalcFnObj::build_with_host(
        c.clone(),
        CalcHostConfig::new().with_compute_image_id_on_load(true),
    )
    .unwrap();
    assert_eq!(lazy.image_id(), c.image_id());
    assert_eq!(lazy.image_id(), eager.image_id());
    let exec = lazy.run(&[], limits());
    assert_eq!(lazy.receipt(&[], &exec).image_id, eager.receipt(&[], &exec).image_id);
}

#[cfg(feature = "calc-aot")]
#[test]
fn aot_mount_is_a_host_knob_with_identical_results() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let mounted = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    assert!(mounted.aot_mounted());
    let plain = CalcFnObj::build_with_host(c, CalcHostConfig::new().with_aot_mount(false)).unwrap();
    let a = plain.run(&[], limits());
    let b = mounted.run(&[], limits());
    assert_eq!(a.result, b.result);
    assert_eq!(a.cycles_used, b.cycles_used);
    assert_eq!(a.resource, b.resource);
}

/// The interpreter-only entry observes the same module semantics: v1
/// verification accepts every profile, and the engine runs it.
#[test]
fn interp_runs_v1_verified_modules() {
    let c = container(CALC_PROFILE_RV64IM_B, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let m = vm::calc::verify::verify_container_with(&c, &CALC_CONFIG_V1).unwrap();
    assert!(interp::run(&m, &c, &[], 1_000_000, None).is_ok());
    let _ = CalcAbort::Trap("".to_string()); // abort type stays in the public surface
}
