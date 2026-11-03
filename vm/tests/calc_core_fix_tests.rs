//! Focused regression tests for the calc VM container/verifier/interpreter
//! hardening fixes: decode strictness, checked encode conversions, canonical
//! packing, header invariants and mid-block dynamic `jalr` pricing.

use std::time::{Duration, Instant};

use vm::calc::container::{
    CalcContainer, CALC_MAGIC, CALC_PROFILE_RV64IM, CALC_VERSION, CODE_BASE, HEADER_LEN,
};
use vm::calc::verify::verify_container;
use vm::calc::{pack_container, CalcAbort2, CalcFnObj, CalcRunLimits};
use vm::calc::SpaceCap;

/// HALT ecall with exit 0 and empty output.
fn ret_code() -> Vec<u8> {
    vec![
        0x13, 0x05, 0x00, 0x00, // addi a0, x0, 0
        0x93, 0x05, 0x00, 0x00, // addi a1, x0, 0
        0x93, 0x08, 0x00, 0x00, // addi a7, x0, 0
        0x73, 0x00, 0x00, 0x00, // ecall
    ]
}

/// A directly constructed, otherwise valid container (bypasses `decode`).
fn valid_container() -> CalcContainer {
    CalcContainer {
        profile: CALC_PROFILE_RV64IM,
        mem_kib: 4,
        entry: CODE_BASE as u32,
        code: ret_code(),
        rodata: Vec::new(),
        init: Vec::new(),
        input_max: 16,
        output_max: 16,
    }
}

fn packed() -> CalcContainer {
    pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        16,
        16,
        ret_code(),
        vec![],
        vec![],
    )
    .unwrap()
}

// ---------------------------------------------------------------- fix 3 ---

#[test]
fn decode_rejects_truncated_payload() {
    let mut bytes = packed().encode().unwrap();
    bytes.pop();
    assert!(CalcContainer::decode(&bytes).is_err());
}

#[test]
fn decode_rejects_trailing_bytes() {
    let mut bytes = packed().encode().unwrap();
    bytes.push(0);
    assert!(CalcContainer::decode(&bytes).is_err());
}

#[test]
fn decode_rejects_huge_declared_lengths_without_panicking() {
    let mut bytes = vec![0u8; HEADER_LEN];
    bytes[0..4].copy_from_slice(&CALC_MAGIC);
    bytes[4] = CALC_VERSION;
    bytes[8..12].copy_from_slice(&4u32.to_be_bytes()); // mem_kib
    bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes()); // code_len
    bytes[20..24].copy_from_slice(&u32::MAX.to_be_bytes()); // init_len
    bytes[24..26].copy_from_slice(&16u16.to_be_bytes()); // input_max
    bytes[26..28].copy_from_slice(&16u16.to_be_bytes()); // output_max
    bytes[28..32].copy_from_slice(&0u32.to_be_bytes()); // rodata_len
    assert!(CalcContainer::decode(&bytes).is_err());
}

#[test]
fn decode_rejects_nonzero_reserved_header_bytes() {
    // The 32-byte header has two reserved bytes (6..8). They are the only
    // future wire extension point and must stay zero for v1.
    for off in [6usize, 7] {
        let mut bytes = packed().encode().unwrap();
        bytes[off] = 1;
        assert!(
            CalcContainer::decode(&bytes).is_err(),
            "reserved byte {off} must be zero"
        );
    }
}

// ---------------------------------------------------------------- fix 4 ---

#[test]
fn encode_accepts_rodata_above_u16_and_pack_rejects_on_layout() {
    // rodata_len is u32, so encoding itself must not fail on a 64KiB+
    // rodata; the container still cannot pack because the declared 4KiB
    // memory cannot hold it (layout fit check).
    let mut c = valid_container();
    c.rodata = vec![0u8; 65_536];
    assert!(c.encode().is_ok());
    assert!(pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        16,
        16,
        ret_code(),
        vec![0u8; 65_536],
        vec![]
    )
    .is_err());
}

#[test]
fn encode_round_trips_valid_container() {
    let c = valid_container();
    let bytes = c.encode().unwrap();
    assert_eq!(bytes.len(), HEADER_LEN + ret_code().len());
    let (decoded, used) = CalcContainer::decode(&bytes).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(decoded, c);
}

// ---------------------------------------------------------------- fix 5 ---

#[test]
fn pack_container_rejects_mem_kib_above_cap() {
    assert!(pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        128,
        CODE_BASE as u32,
        16,
        16,
        ret_code(),
        vec![],
        vec![]
    )
    .is_err());
}

#[test]
fn pack_container_accepts_input_max_above_l1_call_data() {
    // The container has no HVM call-data cap. Chain admission applies
    // SpaceCap::call_data_size separately.
    assert!(pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        64,
        CODE_BASE as u32,
        6000,
        16,
        ret_code(),
        vec![],
        vec![]
    )
    .is_ok());
}

#[test]
fn pack_container_accepts_valid_layout() {
    assert!(pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        16,
        16,
        ret_code(),
        vec![],
        vec![]
    )
    .is_ok());
}

// ---------------------------------------------------------------- fix 6 ---

#[test]
fn verify_rejects_mem_kib_above_the_chain_cap() {
    let mut c = valid_container();
    c.mem_kib = 128;
    assert!(verify_container(&c).is_err());
}

#[test]
fn verify_leaves_call_data_cap_to_the_chain() {
    let mut c = valid_container();
    c.mem_kib = 64;
    c.input_max = 6000;
    assert!(verify_container(&c).is_ok());
}

// ---------------------------------------------------------------- fix 7 ---

#[test]
fn mid_block_jalr_is_not_overcharged() {
    // addi x5,x0,0 ; addi x5,x5,1 ; addi x6,x0,1 ; bne x5,x6,+16
    // auipc x5,0 ; addi x5,x5,-12 ; jalr x0,x5,0
    // addi a0,x0,0 ; jalr x0,x1,0
    let code: Vec<u8> = [
        [0x93, 0x02, 0x00, 0x00],
        [0x93, 0x82, 0x12, 0x00],
        [0x13, 0x03, 0x10, 0x00],
        [0x63, 0x98, 0x62, 0x00],
        [0x97, 0x02, 0x00, 0x00],
        [0x93, 0x82, 0x42, 0xFF],
        [0x67, 0x80, 0x02, 0x00],
        [0x13, 0x05, 0x00, 0x00],
        [0x93, 0x05, 0x00, 0x00],
        [0x93, 0x08, 0x00, 0x00],
        [0x73, 0x00, 0x00, 0x00],
    ]
    .concat();
    let c = pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        16,
        16,
        code,
        vec![],
        vec![],
    )
    .unwrap();
    let obj = CalcFnObj::build_v1(c).unwrap();
    // Entry block A (addi,addi,addi,bne) = 4; block B (auipc,addi,jalr) = 3;
    // the backward `jalr` re-enters block A at index 1, paying only the
    // remaining suffix (addi,addi,bne) = 3, not the skipped prefix; then the
    // taken branch enters the HALT block = 4. Static total 14, plus the
    // per-call setup charge (64 + 4KiB * 16 = 128) = 142.
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert!(exec.result.is_ok(), "{:?}", exec.result);
    assert_eq!(exec.cycles_used, 142);
}

// ---------------------------------------------------------------- precheck ---

#[test]
fn precheck_rejects_insufficient_resource_before_running() {
    let obj = CalcFnObj::build_v1(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 100,
            resource: 0,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::OutOfResource));
    assert_eq!(exec.cycles_used, 0);
}

/// The resource pre-flight reserves the declared `output_max`, not the
/// output the kernel happens to return: a budget that clears input + memory
/// but not the output reservation is rejected before the run, with zero
/// cycles charged and no post-run surprise.
#[test]
fn resource_preflight_reserves_output_max() {
    let obj = CalcFnObj::build_v1(packed()).unwrap();
    // 4 KiB container: mem 32, input 0, output_max 16 bytes -> 1.
    let exact = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1000,
            resource: 33,
            deadline: None,
        },
    );
    assert!(exact.result.is_ok(), "{:?}", exact.result);
    assert_eq!(exact.resource, 32); // actual output is empty
    let short = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1000,
            resource: 32,
            deadline: None,
        },
    );
    assert_eq!(short.result, Err(CalcAbort2::OutOfResource));
    assert_eq!(short.cycles_used, 0);
    assert_eq!(short.resource, 0);
}

#[test]
fn precheck_allows_input_above_value_size_within_input_max() {
    let c = pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        64,
        CODE_BASE as u32,
        4096,
        8,
        ret_code(),
        vec![],
        vec![],
    )
    .unwrap();
    let obj = CalcFnObj::build_v1(c).unwrap();
    let exec = obj.run(
        &vec![0xab; 2000],
        CalcRunLimits {
            cycles: 100_000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Ok(vec![]), "{:?}", exec.result);
    let over = obj.run(
        &vec![0u8; 4097],
        CalcRunLimits {
            cycles: 100_000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert_eq!(over.result, Err(CalcAbort2::Trap));
}

#[test]
fn precheck_rejects_expired_deadline() {
    let obj = CalcFnObj::build_v1(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            // Must cover the setup charge, otherwise OutOfGas fires first.
            cycles: 1000,
            resource: i64::MAX,
            deadline: Some(Instant::now() - Duration::from_secs(1)),
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::Deadline));
}

// ------------------------------------------------------------- v2 changes ---

/// Instruction word builders (little-endian encodings assembled the same way
/// the decoder extracts them; conformance itself is cross-checked against
/// qemu/ckb-vm in the differential suite).
fn i_type(f3: u32, rd: u8, rs1: u8, imm: u32, opcode: u32) -> [u8; 4] {
    (((imm & 0xFFF) << 20) | ((rs1 as u32) << 15) | (f3 << 12) | ((rd as u32) << 7) | opcode)
        .to_le_bytes()
}
fn r_type(f7: u32, f3: u32, rd: u8, rs1: u8, rs2: u8, opcode: u32) -> [u8; 4] {
    ((f7 << 25)
        | ((rs2 as u32) << 20)
        | ((rs1 as u32) << 15)
        | (f3 << 12)
        | ((rd as u32) << 7)
        | opcode)
        .to_le_bytes()
}
fn shift_imm(f6: u32, f3: u32, rd: u8, rs1: u8, shamt: u32, opcode: u32) -> [u8; 4] {
    ((f6 << 26)
        | (shamt << 20)
        | ((rs1 as u32) << 15)
        | (f3 << 12)
        | ((rd as u32) << 7)
        | opcode)
        .to_le_bytes()
}
fn s_type(f3: u32, rs1: u8, rs2: u8, imm: u32, opcode: u32) -> [u8; 4] {
    (((imm >> 5) << 25)
        | ((rs2 as u32) << 20)
        | ((rs1 as u32) << 15)
        | (f3 << 12)
        | ((imm & 0x1F) << 7)
        | opcode)
        .to_le_bytes()
}

/// Zbb kernel: a = 0x81; a = rori(a,4); b = sext.b(a); c = clz(a);
/// d = rev8(a); e = orc.b(d); f = min(a, d); store the six results into
/// the output buffer; return 24 bytes.
fn zbb_kernel() -> Vec<u8> {
    [
        i_type(0b000, 5, 0, 0x81, 0b0010011),        // addi x5, x0, 0x81
        shift_imm(0b011000, 0b101, 5, 5, 4, 0b0010011), // rori x5, x5, 4
        r_type(0b0110000, 0b001, 6, 5, 0b00100, 0b0010011), // sext.b x6, x5
        r_type(0b0110000, 0b001, 8, 5, 0b00000, 0b0010011), // clz x8, x5
        r_type(0b0110101, 0b101, 7, 5, 0b11000, 0b0010011), // rev8 x7, x5
        r_type(0b0010100, 0b101, 9, 7, 0b00111, 0b0010011), // orc.b x9, x7
        r_type(0b0000101, 0b100, 11, 5, 7, 0b0110011), // min x11, x5, x7
        s_type(0b010, 12, 6, 0, 0b0100011),          // sw x6, 0(x12)
        s_type(0b010, 12, 8, 4, 0b0100011),          // sw x8, 4(x12)
        s_type(0b010, 12, 9, 8, 0b0100011),          // sw x9, 8(x12)
        s_type(0b010, 12, 11, 12, 0b0100011),        // sw x11, 12(x12)
        s_type(0b010, 12, 7, 16, 0b0100011),         // sw x7, 16(x12)
        s_type(0b010, 12, 5, 20, 0b0100011),         // sw x5, 20(x12)
        i_type(0b000, 10, 0, 0, 0b0010011),          // addi a0, x0, 0
        i_type(0b000, 11, 0, 24, 0b0010011),         // addi a1, x0, 24
        i_type(0b000, 17, 0, 0, 0b0010011),          // addi a7, x0, 0
        [0x73, 0x00, 0x00, 0x00],                    // ecall HALT
    ]
    .concat()
}

#[test]
fn zbb_kernel_executes() {
    let c = pack_container(
        &SpaceCap::new(0),
        vm::calc::CALC_PROFILE_RV64IM_B,
        4,
        CODE_BASE as u32,
        16,
        32,
        zbb_kernel(),
        vec![],
        vec![],
    )
    .unwrap();
    let obj = CalcFnObj::build_v1(c).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    let out = exec.result.expect("zbb kernel must run");
    // rori(0x81, 4) = 0x1000_0000_0000_0008 (low nibble wraps to bit 60).
    let a: u64 = 0x1000_0000_0000_0008;
    let d: u64 = a.swap_bytes();
    let e: u64 = {
        let mut v = 0u64;
        for i in 0..8 {
            if (d >> (i * 8)) & 0xFF != 0 {
                v |= 0xFFu64 << (i * 8);
            }
        }
        v
    };
    let f: u64 = ((a as i64).min(d as i64)) as u64;
    let mut want = Vec::new();
    want.extend_from_slice(&(8u32).to_le_bytes()); // sext.b(0x...08) = 8
    want.extend_from_slice(&3u32.to_le_bytes()); // clz: highest set bit is 60
    want.extend_from_slice(&(e as u32).to_le_bytes());
    want.extend_from_slice(&(f as u32).to_le_bytes());
    want.extend_from_slice(&(d as u32).to_le_bytes());
    want.extend_from_slice(&(a as u32).to_le_bytes());
    assert_eq!(out, want);
}

#[test]
fn unreachable_padding_is_accepted() {
    // halt then a dead 32-bit nop: rustc/LLVM pads between functions, so
    // unreachable-but-decodable bytes must verify.
    let mut code = ret_code();
    code.extend_from_slice(&[0x13, 0x00, 0x00, 0x00]);
    let r = pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        16,
        16,
        code,
        vec![],
        vec![],
    );
    assert!(r.is_ok());
}

#[test]
fn fence_decodes_and_executes_as_nop() {
    // fence iorw, iorw (0x0ff0000f) before the return.
    let mut code = vec![0x0f, 0x00, 0xf0, 0x0f];
    code.extend_from_slice(&ret_code());
    let c = pack_container(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        4,
        CODE_BASE as u32,
        16,
        16,
        code,
        vec![],
        vec![],
    )
    .unwrap();
    let obj = CalcFnObj::build_v1(c).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert!(exec.result.is_ok());
}

#[test]
fn setup_cycles_charged_on_instant_return() {
    let obj = CalcFnObj::build_v1(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert!(exec.result.is_ok());
    // 4 KiB container: 64 + 4 * 16 = 128 setup + 4 halt cycles
    // (3 addi + ecall_base 1).
    assert_eq!(exec.cycles_used, 132);
}

#[test]
fn setup_cycles_rejected_when_budget_cannot_cover_them() {
    let obj = CalcFnObj::build_v1(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 10,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::OutOfGas));
    // Only the attempted setup charge is reported (4 KiB container:
    // 64 + 4 * 16 = 128); the rest of the budget is not burned — the tx
    // that reaches this abort cannot be included.
    assert_eq!(exec.cycles_used, 128);
}
