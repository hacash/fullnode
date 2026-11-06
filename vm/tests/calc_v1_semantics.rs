//! v2 execution semantics are unconditional and pinned: every B / RVC
//! profile is admitted (the HASH bit is not), HALT runs, hash ecalls trap,
//! guest holes/code/rodata trap stores, the stack starts at the end of the
//! guest image (no guard band), and any instruction-boundary entry point is
//! accepted. `CalcFnObj::build_v1` uses the default chain caps.

use vm::calc::container::CalcContainer;
use vm::calc::interp;
use vm::calc::{
    CalcFnObj, CalcHostConfig, CalcRunLimits, HashKind, CALC_CODE_BASE, CALC_PROFILE_HASH,
    CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC, CALC_PROFILE_RV64IMC_B, CALC_PROFILE_RV64IM_B,
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
    CalcContainer::from_regions(profile, 8, entry, code, rodata, vec![], 64, 64)
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
fn build_admission_profile_matrix() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(CalcFnObj::build_v1(c).is_ok());
    // Every B / RVC combination is admitted.
    for profile in [
        CALC_PROFILE_RV64IM_B,
        CALC_PROFILE_RV64IMC,
        CALC_PROFILE_RV64IMC_B,
    ] {
        let c = container(profile, CALC_CODE_BASE as u32, halt_code(), vec![]);
        assert!(
            CalcFnObj::build_v1(c).is_ok(),
            "profile {profile} must be admitted"
        );
    }
    // The HASH bit stays rejected, alone or combined.
    for profile in [
        CALC_PROFILE_HASH,
        CALC_PROFILE_RV64IM | CALC_PROFILE_HASH,
        CALC_PROFILE_RV64IMC_B | CALC_PROFILE_HASH,
    ] {
        let c = container(profile, CALC_CODE_BASE as u32, halt_code(), vec![]);
        assert!(CalcFnObj::build_v1(c).is_err(), "profile {profile} must be rejected");
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
    assert!(exec.result.is_err(), "hash ecall must trap without HASH bit: {exec:?}");
    assert_eq!(
        exec.cycles_used,
        2 + vm::calc::CALC_SETUP_BASE_CYCLES + 8 * 16
    );
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
    // rodata itself, the alignment hole between rodata and `.data`, and the
    // code region all reject stores (read-only / unmapped). A real .data
    // segment is required: everything above the last segment is legal stack
    // now that the guard band is gone, so a trailing hole would otherwise
    // not exist. Each target runs in its own kernel so the first trap
    // cannot hide the others.
    for target in [rodata_base, rodata_base + 1, CALC_CODE_BASE] {
        let mut code = Vec::new();
        li(&mut code, 5, target);
        push(&mut code, w_sb(5));
        halt(&mut code);
        assert_eq!(code.len(), 28);
        let c = CalcContainer::from_regions(
            CALC_PROFILE_RV64IM,
            8,
            CALC_CODE_BASE as u32,
            code,
            vec![0xAB],
            vec![0u8; 16],
            64,
            64,
        );
        let on = CalcFnObj::build_v1(c).unwrap();
        assert!(
            on.run(&[], limits()).result.is_err(),
            "store at {target:#x} must trap"
        );
    }
}

#[test]
fn stack_reaches_guest_end() {
    // 28-byte store_at body: with the guard band removed, the first stack
    // byte is guest_end = CODE_BASE+28 and storing there is legal.
    let stack_first = CALC_CODE_BASE + 28;
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, store_at(stack_first), vec![]);
    let lay = c.layout();
    assert_eq!(lay.stack_bottom, stack_first);
    let on = CalcFnObj::build_v1(c).unwrap();
    assert!(
        on.run(&[], limits()).result.is_ok(),
        "a store at the first stack byte is legal"
    );
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
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    let m = vm::calc::verify::verify_container_with_cap(&c, &SpaceCap::new(0)).unwrap();
    assert!(interp::run(&m, &c, &[], 1_000_000, None).is_ok());
}

#[test]
fn hash_profile_is_not_admitted() {
    let profile = CALC_PROFILE_RV64IM | CALC_PROFILE_HASH;
    let c = container(profile, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert!(vm::calc::verify::verify_container_with_cap(&c, &SpaceCap::new(0)).is_err());
}

/// IALIGN: the executable segment BASE must satisfy the profile's fetch
/// alignment, not just its length. `push_stream` decodes by segment-relative
/// offset, so a vaddr like `0x1002` with 4-aligned lengths used to verify
/// — an image a spec-compliant RISC-V (and any zkVM) faults on at fetch.
/// Regression: an unaligned RX vaddr must be rejected by verify (direct
/// construction), by decode (wire path, which runs validate_invariants),
/// and by build (deploy path).
#[test]
fn unaligned_code_vaddr_is_rejected() {
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    assert_eq!(c.segs[0].vaddr % 4, 0, "test premise: builders align code");
    let mut c2 = c.clone();
    c2.segs[0].vaddr |= 0x02; // vaddr |= 2 — lengths stay 4-aligned
    let e = vm::calc::verify::verify_container(&c2).unwrap_err();
    assert!(
        format!("{e:?}").contains("vaddr not 4-byte aligned"),
        "expected the vaddr alignment rejection, got {e:?}"
    );
    let wire = c2.encode().unwrap();
    let derr = CalcContainer::decode(&wire).unwrap_err();
    assert!(format!("{derr:?}").contains("vaddr not 4-byte aligned"));
    assert!(CalcFnObj::build_v1(c2).is_err());
}

/// `with_bss` grows the last writable segment's declared size by `bss_len`.
/// An earlier revision assigned `memsz = filesz + bss_len`, silently
/// SHRINKING a segment that already reserved BSS. Regression: the appended
/// case still builds a valid chain image, and a pre-existing BSS grows.
#[test]
fn with_bss_appends_and_never_shrinks() {
    use vm::calc::container::{CalcSegment, SEG_W};
    let c = container(CALC_PROFILE_RV64IM, CALC_CODE_BASE as u32, halt_code(), vec![]);
    // Code-only container: with_bss appends a fresh RW BSS at guest_end and
    // the result is still a valid, verifiable chain image.
    let end = c.guest_end() as u32;
    let c1 = c.clone().with_bss(4096);
    let bss = c1.segs.last().unwrap();
    assert_eq!(bss.vaddr, end);
    assert_eq!(bss.filesz(), 0);
    assert_eq!(bss.memsz, 4096);
    assert_ne!(bss.flags & SEG_W, 0);
    assert!(CalcFnObj::build_v1(c1.clone()).is_ok());
    // A segment that already reserves BSS (memsz > filesz) must GROW by
    // bss_len; the old assignment would have left memsz = filesz + bss_len.
    let mut c2 = c.clone();
    c2.segs
        .push(CalcSegment::rw(end, vec![0xAB; 64], 64 + 1024));
    let c3 = c2.with_bss(512);
    let last = c3.segs.last().unwrap();
    assert_eq!(last.filesz(), 64);
    assert_eq!(last.memsz, 64 + 1024 + 512);
}
