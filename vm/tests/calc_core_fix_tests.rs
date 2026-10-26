//! Focused regression tests for the calc VM container/verifier/interpreter
//! hardening fixes: decode strictness, checked encode conversions, canonical
//! packing, header invariants and mid-block dynamic `jalr` pricing.

use std::time::{Duration, Instant};

use vm::calc::container::{
    CalcContainer, CALC_ABI_BYTES_TO_BYTES, CALC_ISA_RV64IMC, CALC_MAGIC, CALC_VERSION,
    HEADER_LEN,
};
use vm::calc::verify::verify_container;
use vm::calc::{pack_container, CalcAbort2, CalcFnObj, CalcRunLimits};

/// `addi a0, x0, 0 ; jalr x0, x1, 0` - returns immediately with an empty
/// output via the canonical return sentinel.
fn ret_code() -> Vec<u8> {
    vec![0x13, 0x05, 0x00, 0x00, 0x67, 0x80, 0x00, 0x00]
}

/// A directly constructed, otherwise valid container (bypasses `decode`).
fn valid_container() -> CalcContainer {
    CalcContainer {
        isa_profile: CALC_ISA_RV64IMC,
        abi: CALC_ABI_BYTES_TO_BYTES,
        fncnf: 0b11,
        mem_kib: 4,
        entry: 0,
        code: ret_code(),
        rodata: Vec::new(),
        init: Vec::new(),
        input_max: 16,
        output_max: 16,
    }
}

fn packed() -> CalcContainer {
    pack_container(4, 0, 16, 16, ret_code(), vec![], vec![]).unwrap()
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
    bytes[5] = CALC_ISA_RV64IMC;
    bytes[6] = CALC_ABI_BYTES_TO_BYTES;
    bytes[7] = 0b11;
    bytes[8..10].copy_from_slice(&4u16.to_be_bytes()); // mem_kib
    bytes[14..18].copy_from_slice(&u32::MAX.to_be_bytes()); // code_len
    bytes[18..22].copy_from_slice(&u32::MAX.to_be_bytes()); // init_len
    bytes[22..24].copy_from_slice(&16u16.to_be_bytes()); // input_max
    bytes[24..26].copy_from_slice(&16u16.to_be_bytes()); // output_max
    bytes[26..28].copy_from_slice(&0u16.to_be_bytes()); // rodata_len
    assert!(CalcContainer::decode(&bytes).is_err());
}

// ---------------------------------------------------------------- fix 4 ---

#[test]
fn encode_rejects_rodata_longer_than_u16() {
    let mut c = valid_container();
    c.rodata = vec![0u8; 65_536];
    assert!(c.encode().is_err());
}

#[test]
fn encode_round_trips_valid_container() {
    let c = valid_container();
    let bytes = c.encode().unwrap();
    let (decoded, used) = CalcContainer::decode(&bytes).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(decoded, c);
}

// ---------------------------------------------------------------- fix 5 ---

#[test]
fn pack_container_rejects_mem_kib_above_max() {
    assert!(pack_container(128, 0, 16, 16, ret_code(), vec![], vec![]).is_err());
}

#[test]
fn pack_container_rejects_input_max_above_io_max() {
    assert!(pack_container(64, 0, 5000, 16, ret_code(), vec![], vec![]).is_err());
}

#[test]
fn pack_container_accepts_valid_layout() {
    assert!(pack_container(4, 0, 16, 16, ret_code(), vec![], vec![]).is_ok());
}

// ---------------------------------------------------------------- fix 6 ---

#[test]
fn verify_rejects_bad_isa_profile() {
    let mut c = valid_container();
    c.isa_profile = 99;
    assert!(verify_container(&c).is_err());
}

#[test]
fn verify_rejects_bad_abi() {
    let mut c = valid_container();
    c.abi = 99;
    assert!(verify_container(&c).is_err());
}

#[test]
fn verify_rejects_bad_fncnf() {
    let mut c = valid_container();
    c.fncnf = 0;
    assert!(verify_container(&c).is_err());
}

#[test]
fn verify_rejects_mem_kib_out_of_range() {
    let mut c = valid_container();
    c.mem_kib = 128;
    assert!(verify_container(&c).is_err());
}

#[test]
fn verify_rejects_input_max_above_io_max() {
    let mut c = valid_container();
    c.input_max = 5000;
    assert!(verify_container(&c).is_err());
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
        [0x67, 0x80, 0x00, 0x00],
    ]
    .concat();
    let c = pack_container(4, 0, 16, 16, code, vec![], vec![]).unwrap();
    let obj = CalcFnObj::build(c).unwrap();
    // The backward `jalr` re-enters the first block at index 1, skipping
    // `addi x5,x0,0`; only the remaining suffix (3 cycles) is charged.
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 100,
            resource: i64::MAX,
            value_size: 1280,
            deadline: None,
        },
    );
    assert!(exec.result.is_ok());
    assert_eq!(exec.cycles_used, 12);
}

// ---------------------------------------------------------------- precheck ---

#[test]
fn precheck_rejects_insufficient_resource_before_running() {
    let obj = CalcFnObj::build(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 100,
            resource: 0,
            value_size: 1280,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::OutOfResource));
    assert_eq!(exec.cycles_used, 0);
}

#[test]
fn precheck_rejects_value_size_smaller_than_output_max() {
    // output_max is 16; a value_size of 8 cannot hold the declared output.
    let obj = CalcFnObj::build(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 100,
            resource: i64::MAX,
            value_size: 8,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::ValueTooLarge));
}

#[test]
fn precheck_rejects_expired_deadline() {
    let obj = CalcFnObj::build(packed()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 100,
            resource: i64::MAX,
            value_size: 1280,
            deadline: Some(Instant::now() - Duration::from_secs(1)),
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::Deadline));
}