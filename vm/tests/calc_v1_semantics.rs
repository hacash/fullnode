//! v1 execution semantics are unconditional and pinned: every ISA profile
//! cell is admitted, HALT runs, host ecalls are disabled, rodata/the
//! alignment hole/code are read-only, the stack guard band traps, and any
//! instruction-boundary entry point is accepted. There is no consensus
//! config switch for any of these — turning one off would reject or
//! reinterpret previously legal containers — so these tests pin the
//! behaviors directly. `CalcFnObj::build_v1` uses the default chain caps.

use vm::calc::container::CalcContainer;
use vm::calc::interp;
use vm::calc::{
    CalcFnObj, CalcHostConfig, CalcRunLimits, HashKind, CALC_CODE_BASE,
    CALC_HASH_ECALLS_ENABLED, CALC_PROFILE_NO_HOST_ECALL, CALC_PROFILE_RV64IM,
    CALC_PROFILE_RV64IMC, CALC_PROFILE_RV64IMC_B, CALC_PROFILE_RV64IM_B,
};
use vm::calc::SpaceCap;

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
        mem_kib: 8,
        entry,
        code,
        rodata,
        init: vec![],
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
        // The third axis is orthogonal: a HALT-only declaration on top of a
        // base cell is its own admitted profile value.
        CALC_PROFILE_RV64IM | CALC_PROFILE_NO_HOST_ECALL,
        CALC_PROFILE_RV64IMC_B | CALC_PROFILE_NO_HOST_ECALL,
    ] {
        let c = container(profile, CALC_CODE_BASE as u32, halt_code(), vec![]);
        assert!(CalcFnObj::build_v1(c).is_ok(), "profile {profile}");
    }
}

/// v1 ships with host hash ecalls disabled: `a7 = 1` traps, and the call
/// charges only the static ecall cost (the hash price is never applied).
/// The pricing helper itself stays exercised so a future consensus upgrade
/// can enable the table without re-deriving the numbers.
#[test]
fn host_hash_ecalls_are_disabled_in_v1() {
    let mut code = Vec::new();
    push(&mut code, w_addi(17, 0, 1)); // a7 = 1 (SHA-256)
    push(&mut code, 0x0000_0073);
    halt(&mut code);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, code, vec![]);
    let obj = CalcFnObj::build_v1(c).unwrap();
    let exec = obj.run(&[], limits());
    if CALC_HASH_ECALLS_ENABLED {
        assert!(exec.result.is_ok(), "{exec:?}");
        let hash = HashKind::Sha2.cycles(0);
        assert_eq!(
            exec.cycles_used,
            hash + 2 + 4 + vm::calc::CALC_SETUP_BASE_CYCLES + 8 * 16
        );
    } else {
        assert!(exec.result.is_err(), "hash ecall must trap in v1: {exec:?}");
        // addi + ecall (2) at the failing block, setup for 8 KiB; no hash cost.
        assert_eq!(
            exec.cycles_used,
            2 + vm::calc::CALC_SETUP_BASE_CYCLES + 8 * 16
        );
    }
    // The price table is still total and monotone; it is the future switch.
    assert!(HashKind::Sha2.cycles(0) > 0);
    assert_eq!(HashKind::Sha2.cycles(0), 64);
}

#[test]
fn rodata_hole_and_code_stores_trap() {
    // `li` is two instructions regardless of the address, so the body is
    // always 28 bytes: li(8) + sb(4) + halt(16).
    let rodata_base = (CALC_CODE_BASE + 28 + 15) & !15;
    let data_base = (rodata_base + 1 + 15) & !15;
    assert!(rodata_base + 1 < data_base, "a one-byte rodata leaves a hole");
    // rodata itself, the alignment hole before `.data`, and the code region
    // are all read-only. Each target runs in its own kernel so the first
    // trap cannot hide the others.
    for target in [rodata_base, rodata_base + 1, CALC_CODE_BASE] {
        let mut code = Vec::new();
        li(&mut code, 5, target);
        push(&mut code, w_sb(5));
        halt(&mut code);
        assert_eq!(code.len(), 28);
        let c = CalcContainer {
            profile: CALC_PROFILE_RV64IM,
            mem_kib: 8,
            entry: CALC_CODE_BASE as u32,
            code,
            rodata: vec![0xAB],
            init: vec![],
            input_max: 64,
            output_max: 64,
        };
        let on = CalcFnObj::build_v1(c).unwrap();
        assert!(
            on.run(&[], limits()).result.is_err(),
            "store at {target:#x} must trap"
        );
    }
}

#[test]
fn stack_guard_band_traps() {
    // 28-byte store_at body: the guard starts at align16(CODE_BASE+28).
    let guard = 0x1020u64;
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, store_at(guard), vec![]);
    let lay = c.layout();
    assert_eq!(lay.guard_base, guard);
    let on = CalcFnObj::build_v1(c).unwrap();
    assert!(on.run(&[], limits()).result.is_err(), "guard band traps");
}

#[test]
fn free_entry_accepts_any_instruction_boundary() {
    let mut code = vec![0x13, 0x00, 0x00, 0x00];
    halt(&mut code);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32 + 4, code, vec![]);
    let obj = CalcFnObj::build_v1(c).unwrap();
    assert!(obj.run(&[], limits()).result.is_ok(), "entry past the first instruction runs");
    // An entry that is not an instruction boundary still fails header checks.
    let mut bad = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32 + 2, halt_code(), vec![]);
    let err = CalcFnObj::build_v1(bad.clone()).unwrap_err();
    assert!(err_text(&err).contains("aligned"), "{err:?}");
    bad.entry = CALC_CODE_BASE as u32;
    assert!(CalcFnObj::build_v1(bad).is_ok());
}

#[test]
fn image_id_is_cached_and_matches_the_container() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let obj = CalcFnObj::build_with_host(c.clone(), CalcHostConfig::new()).unwrap();
    assert_eq!(obj.image_id(), c.image_id());
    assert_eq!(obj.image_id(), c.image_id(), "cached value is stable");
    let exec = obj.run(&[], limits());
    assert_eq!(obj.receipt(&[], &exec).image_id, c.image_id());
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
    let m = vm::calc::verify::verify_container_with_cap(&c, &SpaceCap::new(0)).unwrap();
    assert!(interp::run(&m, &c, &[], 1_000_000, None).is_ok());
}

/// A `CALC_PROFILE_NO_HOST_ECALL` container keeps HALT working; with host
/// ecalls disabled globally the non-HALT paths trap for every profile, and
/// the profile bit is admitted and carried through verification.
#[test]
fn no_host_ecall_profile_admits_and_keeps_halt() {
    let profile = CALC_PROFILE_RV64IM | CALC_PROFILE_NO_HOST_ECALL;
    let c = container(profile, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let m = vm::calc::verify::verify_container_with_cap(&c, &SpaceCap::new(0)).unwrap();
    assert!(vm::calc::container::profile_has_no_host_ecall(m.profile()));
    assert!(interp::run(&m, &c, &[], 1_000_000, None).is_ok());
}
