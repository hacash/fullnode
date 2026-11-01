//! Each `CalcConfig` flag: v1 default versus the disabled toggle.
//! `CalcFnObj::build` is pinned to `CALC_CONFIG_V1`.

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::verify_container_with;
use vm::calc::{
    CalcFnObj, CalcRunLimits, HashKind, CALC_CODE_BASE, CALC_CONFIG_V1, CALC_PROFILE_RV64IM,
    CALC_PROFILE_RV64IM_B,
};
#[cfg(feature = "calc-aot")]
use vm::calc::CalcConfig;

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
        value_size: usize::MAX,
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
fn build_uses_v1() {
    let p0 = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(CalcFnObj::build(p0).is_ok());
    let p1 = container(CALC_PROFILE_RV64IM_B, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(CalcFnObj::build(p1).is_ok());
    let p2 = container(vm::calc::CALC_PROFILE_RV64IMC, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(CalcFnObj::build(p2).is_ok());
}

#[test]
fn profile1_bitmanip_gate() {
    let c = container(CALC_PROFILE_RV64IM_B, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(verify_container_with(&c, &CALC_CONFIG_V1).is_ok());
    let off_cfg = CALC_CONFIG_V1.with_profile1_bitmanip(false);
    let off = verify_container_with(&c, &off_cfg).unwrap_err();
    assert!(err_text(&off).contains("profile not enabled"), "{off:?}");
    let p0 = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(verify_container_with(&p0, &off_cfg).is_ok());
}

#[test]
fn compressed_gate() {
    let c = container(vm::calc::CALC_PROFILE_RV64IMC, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(verify_container_with(&c, &CALC_CONFIG_V1).is_ok());
    let off_cfg = CALC_CONFIG_V1.with_compressed(false);
    let off = verify_container_with(&c, &off_cfg).unwrap_err();
    assert!(err_text(&off).contains("profile not enabled"), "{off:?}");
    let both = container(vm::calc::CALC_PROFILE_RV64IMC_B, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(verify_container_with(&both, &off_cfg).is_err());
    assert!(verify_container_with(&both, &CALC_CONFIG_V1).is_ok());
}

#[test]
fn hash_builtins_gate() {
    let mut code = Vec::new();
    push(&mut code, w_addi(17, 0, 1));
    push(&mut code, 0x0000_0073);
    halt(&mut code);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, code, vec![]);
    let on_m = verify_container_with(&c, &CALC_CONFIG_V1).unwrap();
    let off_cfg = CALC_CONFIG_V1.with_hash_builtins(false);
    let off_m = verify_container_with(&c, &off_cfg).unwrap();
    let off = interp::run(&off_m, &c, &[], 1_000_000, None).unwrap_err();
    match &off.0 {
        CalcAbort::Trap(t) => assert!(t.contains("builtin not enabled"), "{t}"),
        other => panic!("expected builtin trap, got {other:?}"),
    }
    let ok = interp::run(&on_m, &c, &[], 1_000_000, None).unwrap();
    let hash = HashKind::Sha2.cycles(0);
    // first block is addi+ecall (1); halt block is 3 addis. Off path skips the hash charge.
    assert_eq!(off.1 + hash + 3, ok.cycles_used, "disabled hash must not be charged");
    let halt_only = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let obj = CalcFnObj::build(halt_only).unwrap();
    assert!(obj.run(&[], limits()).result.is_ok());
}

#[test]
fn rodata_readonly_gate() {
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
    let on = CalcFnObj::build(c.clone()).unwrap();
    assert!(on.run(&[], limits()).result.is_err(), "v1 rodata and hole stores trap");
    let off_cfg = CALC_CONFIG_V1.with_rodata_readonly(false);
    let unlocked = CalcFnObj::build_with_config(c, off_cfg).unwrap();
    assert!(unlocked.run(&[], limits()).result.is_ok(), "disabled rodata is writable");

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
fn stack_guard_gate() {
    // 28-byte store_at body: guard starts at align16(CODE_BASE+28) = 0x1020.
    let guard = 0x1020u64;
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, store_at(guard), vec![]);
    let lay = c.layout();
    assert_eq!(lay.guard_base, guard);
    let on = CalcFnObj::build(c.clone()).unwrap();
    assert!(on.run(&[], limits()).result.is_err(), "v1 guard band traps");
    let off_cfg = CALC_CONFIG_V1.with_stack_guard(false);
    let unlocked = CalcFnObj::build_with_config(c, off_cfg).unwrap();
    assert!(unlocked.run(&[], limits()).result.is_ok(), "disabled guard band is writable");
}

#[test]
fn free_entry_gate() {
    let mut code = vec![0x13, 0x00, 0x00, 0x00];
    halt(&mut code);
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32 + 4, code, vec![]);
    let m = verify_container_with(&c, &CALC_CONFIG_V1).unwrap();
    assert!(interp::run(&m, &c, &[], 1_000_000, None).is_ok());
    let off = CALC_CONFIG_V1.with_free_entry(false);
    let err = verify_container_with(&c, &off).unwrap_err();
    assert!(err_text(&err).contains("CODE_BASE"), "{err:?}");
}

#[test]
fn image_id_lazy_equals_eager() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let lazy = CalcFnObj::build(c.clone()).unwrap();
    let eager_cfg = CALC_CONFIG_V1.with_compute_image_id_on_load(true);
    let eager = CalcFnObj::build_with_config(c.clone(), eager_cfg).unwrap();
    assert_eq!(lazy.image_id(), c.image_id());
    assert_eq!(lazy.image_id(), eager.image_id());
    let exec = lazy.run(&[], limits());
    assert_eq!(lazy.receipt(&[], &exec).image_id, eager.receipt(&[], &exec).image_id);
}

#[cfg(feature = "calc-aot")]
#[test]
fn aot_mount_gate_and_parity() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let v1 = CalcFnObj::build_with_aot(c.clone()).unwrap();
    assert!(v1.aot_mounted());
    let on = CalcFnObj::build_with_config(c.clone(), CalcConfig::all_enabled()).unwrap();
    assert!(on.aot_mounted());
    let interp = CalcFnObj::build_with_config(
        c,
        CalcConfig::all_enabled().with_aot_mount(false),
    )
    .unwrap();
    let a = interp.run(&[], limits());
    let b = on.run(&[], limits());
    assert_eq!(a.result, b.result);
    assert_eq!(a.cycles_used, b.cycles_used);
    assert_eq!(a.resource, b.resource);
}
