//! Calc module unit tests: decoder vectors (clang 18 `-march=rv64imc`
//! encodings), container malicious samples, and a hand-assembled
//! end-to-end kernel run.

#![cfg(test)]

use crate::calc::container::{
    CalcContainer, CALC_ABI_BYTES_TO_BYTES, CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IM_B, CODE_BASE,
};
use crate::calc::decode::{decode_at, BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
use crate::calc::verify::verify_container_with;
use crate::rt::VmrtRes;

/// Consensus v1 semantics.
fn verify_container(c: &CalcContainer) -> VmrtRes<crate::calc::verify::CalcModule> {
    verify_container_with(c, &crate::calc::CALC_CONFIG_V1)
}

fn make_container(code: Vec<u8>) -> CalcContainer {
    make_container_profile(CALC_PROFILE_RV64IM_B, code)
}

fn make_container_profile(profile: u8, code: Vec<u8>) -> CalcContainer {
    CalcContainer {
        profile,
        abi: CALC_ABI_BYTES_TO_BYTES,
        mem_kib: 8,
        entry: CODE_BASE as u32,
        code,
        rodata: vec![],
        init: vec![],
        bss_len: 0,
        input_max: 64,
        output_max: 64,
    }
}

fn d(bytes: &[u8]) -> Inst {
    decode_at(bytes, 0, CALC_PROFILE_RV64IM_B).unwrap().0
}

fn halt_empty() -> Vec<u8> {
    vec![
        0x13, 0x05, 0x00, 0x00, // addi a0, x0, 0
        0x93, 0x05, 0x00, 0x00, // addi a1, x0, 0
        0x93, 0x08, 0x00, 0x00, // addi a7, x0, 0
        0x73, 0x00, 0x00, 0x00, // ecall
    ]
}

#[test]
fn decode_32bit_vectors() {
    // All encodings generated with: clang 18 --target=riscv64-unknown-none-elf
    // -march=rv64imc (see research report §4).
    assert_eq!(
        d(&hexw("123457b7")),
        Inst::Lui { rd: 15, val: 0x12345 << 12 }
    );
    assert_eq!(d(&hexw("00001717")), Inst::Auipc { rd: 14, val: 0x1000 });
    assert_eq!(d(&hexw("008000ef")), Inst::Jal { rd: 1, target: 8 });
    assert_eq!(d(&hexw("ffc582e7")), Inst::Jalr { rd: 5, rs1: 11, imm: -4 });
    assert_eq!(
        d(&hexw("00b50863")),
        Inst::Branch { kind: BrKind::Eq, rs1: 10, rs2: 11, target: 16 }
    );
    assert_eq!(
        d(&hexw("00b57863")),
        Inst::Branch { kind: BrKind::Geu, rs1: 10, rs2: 11, target: 16 }
    );
    assert_eq!(
        d(&hexw("ffd58603")),
        Inst::Load { op: LoadOp::Lb, rd: 12, rs1: 11, imm: -3 }
    );
    assert_eq!(
        d(&hexw("1005a603")),
        Inst::Load { op: LoadOp::Lw, rd: 12, rs1: 11, imm: 0x100 }
    );
    assert_eq!(
        d(&hexw("ff85b603")),
        Inst::Load { op: LoadOp::Ld, rd: 12, rs1: 11, imm: -8 }
    );
    assert_eq!(
        d(&hexw("00c580a3")),
        Inst::Store { op: StoreOp::Sb, rs1: 11, rs2: 12, imm: 1 }
    );
    assert_eq!(
        d(&hexw("04c5a023")),
        Inst::Store { op: StoreOp::Sw, rs1: 11, rs2: 12, imm: 64 }
    );
    assert_eq!(
        d(&hexw("00c5b823")),
        Inst::Store { op: StoreOp::Sd, rs1: 11, rs2: 12, imm: 16 }
    );
    assert_eq!(
        d(&hexw("ffb58613")),
        Inst::ImmAlu { op: ImmAluOp::Addi, rd: 12, rs1: 11, imm: -5 }
    );
    assert_eq!(
        d(&hexw("02459613")),
        Inst::ShiftImm { kind: ShiftKind::Sll, rd: 12, rs1: 11, shamt: 36, word: false }
    );
    assert_eq!(
        d(&hexw("0145d613")),
        Inst::ShiftImm { kind: ShiftKind::Srl, rd: 12, rs1: 11, shamt: 20, word: false }
    );
    assert_eq!(
        d(&hexw("43f5d613")),
        Inst::ShiftImm { kind: ShiftKind::Sra, rd: 12, rs1: 11, shamt: 63, word: false }
    );
    assert_eq!(
        d(&hexw("0075861b")),
        Inst::ImmAlu { op: ImmAluOp::Addiw, rd: 12, rs1: 11, imm: 7 }
    );
    assert_eq!(
        d(&hexw("41f5d61b")),
        Inst::ShiftImm { kind: ShiftKind::Sra, rd: 12, rs1: 11, shamt: 31, word: true }
    );
    assert_eq!(
        d(&hexw("00a58633")),
        Inst::RegAlu { op: RegAluOp::Add, rd: 12, rs1: 11, rs2: 10 }
    );
    assert_eq!(
        d(&hexw("40a5d633")),
        Inst::RegAlu { op: RegAluOp::Sra, rd: 12, rs1: 11, rs2: 10 }
    );
    assert_eq!(
        d(&hexw("02a59633")),
        Inst::RegAlu { op: RegAluOp::Mulh, rd: 12, rs1: 11, rs2: 10 }
    );
    assert_eq!(
        d(&hexw("02a5f633")),
        Inst::RegAlu { op: RegAluOp::Remu, rd: 12, rs1: 11, rs2: 10 }
    );
    assert_eq!(
        d(&hexw("02a5c63b")),
        Inst::RegAlu { op: RegAluOp::Divw, rd: 12, rs1: 11, rs2: 10 }
    );
    assert_eq!(d(&hexw("00000073")), Inst::Ecall);
}

#[test]
fn decode_rejects_compressed() {
    // RVC is rejected when profile bit 1 is clear, and accepted on RV64IMC.
    for half in ["0808", "0001", "8502", "c1b0", "e990", "9002", "0000"] {
        assert!(
            decode_at(&hexw(half), 0, CALC_PROFILE_RV64IM).is_err(),
            "compressed {half} must be rejected"
        );
        assert!(decode_at(&hexw(half), 0, CALC_PROFILE_RV64IM_B).is_err());
    }
    let nop = decode_at(&hexw("0001"), 0, crate::calc::CALC_PROFILE_RV64IMC).unwrap();
    assert_eq!(nop.1, 2);
    assert!(decode_at(&hexw("0001"), 0, crate::calc::CALC_PROFILE_RV64IMC_B).is_ok());
}

#[test]
fn profile0_rejects_bitmanip() {
    // clz x3, x4 — profile 0 must reject, profile 1 must accept.
    let clz = hexw("60f21293"); // may not be right encoding; use r-type builder below
    let w = (0b0110000u32 << 25) | (0 << 20) | (4 << 15) | (1 << 12) | (3 << 7) | 0b0010011;
    let bytes = w.to_le_bytes();
    assert!(decode_at(&bytes, 0, CALC_PROFILE_RV64IM).is_err());
    assert!(decode_at(&bytes, 0, CALC_PROFILE_RV64IM_B).is_ok());
    let _ = clz;
}


fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Objdump/clang print instruction words in big-endian hex; memory (and our
/// decoder) is little-endian: reverse the byte order.
fn hexw(s: &str) -> Vec<u8> {
    let mut v = hex(s);
    v.reverse();
    v
}

#[test]
fn decode_rejects_non_whitelist() {
    let p = CALC_PROFILE_RV64IM_B;
    // amoadd.w (A extension)
    assert!(decode_at(&hexw("0025a52f"), 0, p).is_err());
    // ebreak and canonical unimp decode to a 1-cycle trap.
    assert_eq!(decode_at(&hexw("00100073"), 0, p).unwrap().0, Inst::Trap);
    assert_eq!(decode_at(&hexw("c0001073"), 0, p).unwrap().0, Inst::Trap);
    assert_eq!(crate::calc::decode::inst_cycles(&Inst::Trap), 1);
    // csr read
    assert!(decode_at(&hexw("c0102573"), 0, p).is_err());
    // fence: accepted as a no-op (single-hart)
    assert!(decode_at(&hexw("0ff0000f"), 0, p).is_ok());
    // fp add (D extension)
    assert!(decode_at(&hexw("002585d3"), 0, p).is_err());
    // compressed encodings
    assert!(decode_at(&hexw("9002"), 0, p).is_err());
    assert!(decode_at(&hex("0000"), 0, p).is_err());
    assert!(decode_at(&hexw("2000"), 0, p).is_err());
    // unknown profile
    assert!(decode_at(&hexw("00000013"), 0, 9).is_err());
}

#[test]
fn container_roundtrip_and_malicious() {
    let c = make_container(hex("1305800067800000")); // li a0,8-ish placeholder
    let enc = c.encode().expect("encode");
    let (c2, used) = CalcContainer::decode(&enc).unwrap();
    assert_eq!(used, enc.len());
    assert_eq!(c, c2);

    // magic / version / isa / abi / fncnf
    let mut bad = enc.clone();
    bad[0] = b'X';
    assert!(CalcContainer::decode(&bad).is_err());
    let mut bad = enc.clone();
    bad[4] = 2;
    assert!(CalcContainer::decode(&bad).is_err());
    let mut bad = enc.clone();
    bad[5] = 9; // unknown profile
    assert!(CalcContainer::decode(&bad).is_err());
    let mut bad = enc.clone();
    bad[7] = 0b1000_0001; // reserved fncnf
    assert!(CalcContainer::decode(&bad).is_err());
    let mut bad = enc.clone();
    bad[5] = 1; // profile 1 is legal
    assert!(CalcContainer::decode(&bad).is_ok());
    // corrupting rodata_len (bytes 28..32) must not decode
    let mut bad = enc.clone();
    bad[28] = 1;
    assert!(CalcContainer::decode(&bad).is_err());
    // entry outside code
    let mut c3 = c.clone();
    c3.entry = 999;
    assert!(CalcContainer::decode(&c3.encode().unwrap()).is_err());
    // non-multiple-of-4 code_len
    let mut c4 = c.clone();
    c4.code = hex("13");
    assert!(CalcContainer::decode(&c4.encode().unwrap()).is_err());
}

#[test]
fn verify_rejects_malicious_code() {
    // (a) branch into the middle of its own 32-bit encoding:
    //     0: beq a0,a1,+2 (00b50163) -> target 2 is mid-instruction
    let code = concat_hex(&["6301b500"]);
    let c = make_container(code);
    assert!(verify_container(&c).is_err());

    // (b) branch target beyond the code stream:
    //     beq a0,a1,+16 ; addi x0,x0,0
    let code = concat_hex(&["6308b500", "13000000"]);
    let c = make_container(code);
    assert!(verify_container(&c).is_err());

    // (c) unreachable trailing nop is accepted (LLVM padding).
    let code = concat_hex(&["13000000", "13000000"]);
    let c = make_container(code);
    assert!(verify_container(&c).is_ok());

    // (d) bare self-loop (jal x0, 0) verifies; one block, 1 cycle.
    let code = concat_hex(&["6f000000"]);
    let c = make_container(code);
    let module = verify_container(&c).unwrap();
    assert_eq!(module.blocks.len(), 1);
    assert_eq!(module.blocks[0].cycles, 1);

    // (e) valid tiny kernel: lbu t0,0(a0); sd t0,0(a2); halt empty
    let mut code = concat_hex(&["83420500", "23305600"]);
    code.extend_from_slice(&halt_empty());
    let c = make_container(code);
    let module = verify_container(&c).unwrap();
    assert_eq!(module.insts.len(), 6);
    // lbu(2)+sd(2)+3*addi(1)+ecall(1) split: ecall terminates a block.
    assert!(module.blocks.iter().map(|b| b.cycles).sum::<u64>() >= 5);
}

fn concat_hex(parts: &[&str]) -> Vec<u8> {
    parts.iter().flat_map(|p| hex(p)).collect()
}

#[test]
fn interp_tiny_kernel() {
    use crate::calc::interp::{run, CalcAbort};
    // lbu t0,0(a0) ; sd t0,0(a2) ; halt with a1=8
    let mut code = concat_hex(&["83420500", "23305600"]);
    // addi a0,x0,0 ; addi a1,x0,8 ; addi a7,x0,0 ; ecall
    code.extend_from_slice(&hex("13050000")); // addi a0, x0, 0
    code.extend_from_slice(&hex("93058000")); // addi a1, x0, 8
    code.extend_from_slice(&hex("93080000")); // addi a7, x0, 0
    code.extend_from_slice(&hex("73000000")); // ecall
    let c = make_container(code);
    let module = verify_container(&c).unwrap();
    let res = run(&module, &c, b"A", 1000, None).unwrap();
    assert_eq!(res.output, vec![b'A', 0, 0, 0, 0, 0, 0, 0]);

    // budget exhaustion: bare jal x0,0 self-loop with a tiny budget
    let loop_code = concat_hex(&["6f000000"]);
    let c = make_container(loop_code);
    let module = verify_container(&c).unwrap();
    match run(&module, &c, b"", 10, None) {
        Err((CalcAbort::OutOfGas, used)) => assert_eq!(used, 10),
        other => panic!("expected OutOfGas, got {:?}", other.map(|r| r.cycles_used)),
    }

    // store to address 0 traps (unmapped low page):
    //   addi t0,x0,0 ; sd t0,0(t0) ; halt
    let mut code = concat_hex(&["93280200", "23b05200"]);
    code.extend_from_slice(&halt_empty());
    let c = make_container(code);
    let module = verify_container(&c).unwrap();
    match run(&module, &c, b"", 100, None) {
        Err((CalcAbort::Trap(t), _)) => {
            assert!(t.contains("unmapped") || t.contains("read-only"), "{t}");
        }
        other => panic!("expected trap, got {:?}", other),
    }

    // misaligned load is allowed (Zicclsm): lh a0,1(a0)
    let mut code = concat_hex(&["03151500"]);
    code.extend_from_slice(&halt_empty());
    let c = make_container(code);
    let module = verify_container(&c).unwrap();
    let res = run(&module, &c, b"AB", 100, None);
    assert!(res.is_ok(), "misaligned load must execute, got {res:?}");
}


#[test]
fn interp_div_rem_by_zero_matches_qemu() {
    // qemu-riscv64 8.2.2 oracle (remtest.s): rem(7,0)=7 remu(7,0)=7
    // div(7,0)=-1 divw(7,0)=all-ones remw(7,0)=7
    use crate::calc::container::CalcContainer;
    use crate::calc::interp::run;
    fn enc_i(op: u32, f3: u32, rd: u8, rs1: u8, imm: i32) -> u32 {
        op | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (f3 << 12) | (((imm as u32) & 0xFFF) << 20)
    }
    fn enc_r(op: u32, f7: u32, f3: u32, rd: u8, rs1: u8, rs2: u8) -> u32 {
        op | ((rd as u32) << 7) | (f3 << 12) | ((rs1 as u32) << 15)
            | ((rs2 as u32) << 20) | (f7 << 25)
    }
    fn enc_s(f3: u32, rs1: u8, rs2: u8, imm: i32) -> u32 {
        let imm = (imm as u32) & 0xFFF;
        0x23 | (f3 << 12) | ((rs1 as u32) << 15) | ((rs2 as u32) << 20)
            | ((imm & 0x1F) << 7) | (imm >> 5 << 25)
    }
    // x19 = a2 (entry-provided output base)
    let w: Vec<u32> = vec![
        enc_r(0x33, 0, 0, 19, 12, 0),
        enc_i(0x13, 0, 6, 0, 7),          // x6 = 7
        enc_r(0x33, 1, 6, 7, 6, 0),        // rem  x7
        enc_r(0x33, 1, 7, 28, 6, 0),       // remu x28
        enc_r(0x33, 1, 4, 29, 6, 0),       // div  x29
        enc_r(0x3B, 1, 4, 30, 6, 0),       // divw x30
        enc_r(0x3B, 1, 6, 31, 6, 0),       // remw x31
        enc_s(3, 19, 7, 0),
        enc_s(3, 19, 28, 8),
        enc_s(3, 19, 29, 16),
        enc_s(3, 19, 30, 24),
        enc_s(3, 19, 31, 32),
        enc_i(0x13, 0, 10, 0, 0),
        enc_i(0x13, 0, 11, 0, 40),
        enc_i(0x13, 0, 17, 0, 0),
        0x0000_0073,
    ];
    let code: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
    let c = CalcContainer {
        profile: CALC_PROFILE_RV64IM,
        abi: 1,
        mem_kib: 16,
        entry: CODE_BASE as u32,
        code,
        rodata: vec![],
        init: vec![],
        bss_len: 0,
        input_max: 512,
        output_max: 512,
    };
    let module = verify_container(&c).expect("verify");
    for (i, inst) in module.insts.iter().enumerate() {
        println!("inst[{i}] @ {:#x}: {:?}", module.addrs[i], inst);
    }
    let res = run(&module, &c, b"", 100_000, None).expect("run");
    assert_eq!(res.output.len(), 40);
    let v: Vec<u64> = res
        .output
        .chunks(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    assert_eq!(v[0], 7, "rem(7,0) = dividend (qemu oracle)");
    assert_eq!(v[1], 7, "remu(7,0) = dividend");
    assert_eq!(v[2], u64::MAX, "div(7,0) = -1");
    assert_eq!(v[3], u64::MAX, "divw(7,0) = sign-extended -1");
    assert_eq!(v[4], 7, "remw(7,0) = dividend");
}

#[test]
fn reg_alu_div_rem_units() {
    use crate::calc::decode::RegAluOp::*;
    use crate::calc::interp::reg_alu_test;
    assert_eq!(reg_alu_test(Rem, 7, 0), 7);
    assert_eq!(reg_alu_test(Remu, 7, 0), 7);
    assert_eq!(reg_alu_test(Div, 7, 0), u64::MAX);
    assert_eq!(reg_alu_test(Divw, 7, 0), u64::MAX);
    assert_eq!(reg_alu_test(Divuw, 7, 0), u64::MAX);
    assert_eq!(reg_alu_test(Remw, 7, 0), 7);
    assert_eq!(reg_alu_test(Remuw, 7, 0), 7);
    assert_eq!(reg_alu_test(Div, i64::MIN as u64, u64::MAX), i64::MIN as u64);
    assert_eq!(reg_alu_test(Rem, i64::MIN as u64, u64::MAX), 0);
    assert_eq!(reg_alu_test(Mulh, i64::MIN as u64, i64::MIN as u64), 1u64 << 62); // (2^63)^2 >> 64 = 2^62
    assert_eq!(reg_alu_test(Mulh, -3i64 as u64, 5), (((-3i64 as i128) * 5i128) >> 64) as u64);
    assert_eq!(reg_alu_test(Mulhu, u64::MAX, u64::MAX), (((u64::MAX as u128) * (u64::MAX as u128)) >> 64) as u64);
    assert_eq!(reg_alu_test(Mulhsu, -1i64 as u64, u64::MAX), (((-1i64 as i128) * (u64::MAX as i128)) >> 64) as u64);
}

#[test]
fn pack_container_roundtrip() {
    // clang-assembled echo kernel (same as the gas tests)
    let mut code: Vec<u8> = vec![0x83, 0x42, 0x05, 0x00, 0x23, 0x30, 0x56, 0x00];
    code.extend_from_slice(&halt_empty());
    let c = crate::calc::pack_container(8, CODE_BASE as u32, 128, 128, code.clone(), vec![], vec![]).unwrap();
    let enc = c.encode().expect("encode");
    let (c2, used) = crate::calc::container::CalcContainer::decode(&enc).unwrap();
    assert_eq!(used, enc.len());
    assert_eq!(c, c2);
    // non-fitting layout must be rejected
    assert!(crate::calc::pack_container(4, CODE_BASE as u32, 4096, 4096, code.clone(), vec![], vec![]).is_err());
}

// --------------------------------------------------------------------
// Semantics regressions (from the 2026-10 full audit; these pin the
// POST-FIX behavior decided in the review: zext.h in OP-32, slliw
// imm[11:5]==0 enforced, stack-overflow guard band, jalr >u32 rejected
// by both engines, ecall charge-before-bounds documented).

/// slliw with imm[11:5] != 0 (bit 25 set in the 6-bit shamt view) is
/// RESERVED per the RV64 spec and must be rejected.
#[test]
fn slliw_reserved_bit25_rejected() {
    let legal: u32 = (8u32 << 20) | (5 << 15) | (1 << 12) | (28 << 7) | 0x1B;
    assert!(matches!(
        decode_at(&legal.to_le_bytes(), 0, CALC_PROFILE_RV64IM),
        Ok((Inst::ShiftImm { word: true, shamt: 8, .. }, 4))
    ));
    assert!(decode_at(&(legal | (1 << 25)).to_le_bytes(), 0, CALC_PROFILE_RV64IM).is_err());
}

/// zext.h: only the RV64 OP-32 form decodes; the RV32-only OP form is
/// illegal on RV64 (llvm-mc 18 / spike / ckb-vm agree).
#[test]
fn zexth_requires_op32_encoding() {
    let rv64 = (0b0000100u32 << 25) | (5 << 15) | (0b100 << 12) | (28 << 7) | 0b0111011;
    assert!(matches!(
        decode_at(&rv64.to_le_bytes(), 0, CALC_PROFILE_RV64IM_B),
        Ok((Inst::UnAlu { op: crate::calc::decode::UnAluOp::Zexth, rd: 28, rs1: 5 }, 4))
    ));
    let rv32 = (0b0000100u32 << 25) | (5 << 15) | (0b100 << 12) | (28 << 7) | 0b0110011;
    assert!(decode_at(&rv32.to_le_bytes(), 0, CALC_PROFILE_RV64IM_B).is_err());
}

/// Stack-overflow guard: a store below the stack bottom lands in the guard
/// band and traps (it used to silently corrupt the output region); a store
/// into the output region proper still succeeds.
#[test]
fn stack_guard_traps_on_overflow() {
    use crate::calc::interp;
    let addi = |rd: u8, rs1: u8, imm: i32| -> u32 {
        0x13u32 | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (((imm as u32) & 0xFFF) << 20)
    };
    // Probe is the first byte of the guard, which now sits just above .bss
    // rather than a fixed 2 KiB under sp. Code is 16 bytes, so data/guard
    // start at align16(CODE_BASE+16) = 0x1010.
    let guard = 0x1010u64;
    let mut prog: Vec<u8> = Vec::new();
    li_addr(&mut prog, 28, guard);
    let sd = 0x23u32 | (28 << 15) | (5 << 20); // sd x5, 0(x28)
    prog.extend_from_slice(&sd.to_le_bytes());
    let ret = 0x67u32 | (1 << 15);
    prog.extend_from_slice(&ret.to_le_bytes());
    assert_eq!(prog.len(), 16);
    let c = make_container(prog);
    let module = verify_container(&c).unwrap();
    let lay = module.layout_snapshot();
    assert_eq!(lay.guard_base, guard);
    let addr = guard;
    assert!(
        addr >= lay.guard_base && addr < lay.stack_bottom,
        "probe address must sit inside the guard band"
    );
    let (r, _, _, _) = interp::run_with_state(&module, &c, &[], 100_000);
    assert!(
        matches!(&r, Err((interp::CalcAbort::Trap(t), _)) if t.contains("guard")),
        "guard-band store must trap, got {r:?}"
    );
    // Control: a store into the OUTPUT region (a2-relative, below the
    // guard) still succeeds and the bytes are readable at return.
    let mut prog: Vec<u8> = Vec::new();
    prog.extend_from_slice(&addi(28, 12, 0).to_le_bytes()); // x28 = output base
    prog.extend_from_slice(&sd.to_le_bytes()); // sd x5, 0(x28)
    prog.extend_from_slice(&addi(10, 0, 0).to_le_bytes()); // a0 = 0
    prog.extend_from_slice(&addi(11, 0, 8).to_le_bytes()); // a1 = 8
    prog.extend_from_slice(&addi(17, 0, 0).to_le_bytes()); // a7 = HALT
    prog.extend_from_slice(&0x0000_0073u32.to_le_bytes()); // ecall
    let c = make_container(prog);
    let module = verify_container(&c).unwrap();
    let (r, _, _, _) = interp::run_with_state(&module, &c, &[], 100_000);
    let run = r.expect("output-region store must stay legal");
    assert_eq!(run.output.len(), 8);
}

/// A `jalr` to an address above u32::MAX is an invalid target on BOTH
/// engines (the AOT resolver used to truncate to u32 and jump; fixed).
#[cfg(feature = "calc-aot")]
#[test]
fn jalr_above_u32_traps_both_engines() {
    use crate::calc::aot;
    use crate::calc::interp;
    let mut prog: Vec<u8> = Vec::new();
    let lui = 0x37u32 | (28 << 7) | (1 << 12);
    prog.extend_from_slice(&lui.to_le_bytes());
    let slli = 0x13u32 | (28 << 7) | (28 << 15) | (1 << 12) | (20 << 20);
    prog.extend_from_slice(&slli.to_le_bytes());
    let jalr = 0x67u32 | (28 << 15);
    prog.extend_from_slice(&jalr.to_le_bytes());
    let ret = 0x67u32 | (1 << 15);
    prog.extend_from_slice(&ret.to_le_bytes());
    let c = make_container(prog);
    let module = verify_container(&c).unwrap();
    let (r, icyc) = {
        let (r, regs, mem, _) = interp::run_with_state(&module, &c, &[], 10_000);
        let cyc = match &r {
            Ok(x) => x.cycles_used,
            Err((_, u)) => *u,
        };
        let _ = (regs, mem);
        (r, cyc)
    };
    assert!(
        matches!(&r, Err((interp::CalcAbort::Trap(t), _)) if t.contains("invalid target")),
        "interp must reject a >u32 target, got {r:?}"
    );
    let f = aot::aot_compile(&module).unwrap();
    let (out, _, _) = f.run_with_state(&module, &c, &[], 10_000);
    assert_eq!(out.abort, Some(aot::AotAbort::BadTarget));
    assert_eq!(out.cycles_used, icyc, "cycle totals must agree");
}

/// Store-fault ORDER differs between the engines (interp: bounds ->
/// readonly -> guard -> alignment; AOT: readonly -> bounds -> guard ->
/// alignment) but the abort CLASS and the cycle total are identical —
/// only the message text differs, and messages are not consensus data.
#[cfg(feature = "calc-aot")]
#[test]
fn store_fault_classes_agree_across_engines() {
    use crate::calc::aot;
    use crate::calc::interp;
    // sh x5, 1(x0): unmapped (and would be read-only) — whichever check
    // fires first, both engines must stop with (Trap, same cycles).
    let mut prog: Vec<u8> = Vec::new();
    let sh = 0x23u32 | (1 << 12) | (5 << 20);
    prog.extend_from_slice(&sh.to_le_bytes());
    let ret = 0x67u32 | (1 << 15);
    prog.extend_from_slice(&ret.to_le_bytes());
    let c = make_container(prog);
    let module = verify_container(&c).unwrap();
    let (r, icyc) = {
        let (r, regs, mem, _) = interp::run_with_state(&module, &c, &[], 10_000);
        let cyc = match &r {
            Ok(x) => x.cycles_used,
            Err((_, u)) => *u,
        };
        let _ = (regs, mem);
        (r, cyc)
    };
    assert!(matches!(r, Err((interp::CalcAbort::Trap(_), _))));
    let f = aot::aot_compile(&module).unwrap();
    let (out, _, _) = f.run_with_state(&module, &c, &[], 10_000);
    assert_eq!(out.abort, Some(aot::AotAbort::Trap));
    assert_eq!(out.cycles_used, icyc);
}

/// ecall charges the hash cost BEFORE the src/dst bounds checks
/// (documented decision): a large budget pays and then traps on the
/// out-of-bounds length; a budget below the hash cost aborts OutOfGas
/// without the charge. Both classes are deterministic.
#[test]
fn ecall_oob_length_abort_class_is_budget_dependent() {
    use crate::calc::interp;
    let mut prog: Vec<u8> = Vec::new();
    let li = |prog: &mut Vec<u8>, rd: u8, imm: i32| {
        let w = 0x13u32 | ((rd as u32) << 7) | (((imm as u32) & 0xFFF) << 20);
        prog.extend_from_slice(&w.to_le_bytes());
    };
    // src = CODE_BASE (mapped) so the unmapped check does not fire; len
    // = 2^20 overruns mem_len. dst stays the ABI output_base (writable).
    let lui_src = 0x37u32 | (10 << 7) | (1 << 12); // a0 = 0x1000
    prog.extend_from_slice(&lui_src.to_le_bytes());
    let lui = 0x37u32 | (11 << 7) | (0x100 << 12); // a1 = 2^20
    prog.extend_from_slice(&lui.to_le_bytes());
    li(&mut prog, 17, 1);
    prog.extend_from_slice(&0x0000_0073u32.to_le_bytes());
    let ret = 0x67u32 | (1 << 15);
    prog.extend_from_slice(&ret.to_le_bytes());
    let c = make_container(prog);
    let module = verify_container(&c).unwrap();
    let (r, _, _, _) = interp::run_with_state(&module, &c, &[], 5_000_000);
    assert!(
        matches!(&r, Err((interp::CalcAbort::Trap(t), _)) if t.contains("out of memory bounds")),
        "big budget -> Trap, got {r:?}"
    );
    let (r2, _, _, _) = interp::run_with_state(&module, &c, &[], 200);
    assert!(matches!(r2, Err((interp::CalcAbort::OutOfGas, _))));
}

#[test]
fn image_id_and_receipt_are_deterministic() {
    use crate::calc::{sha256_bytes, CalcFnObj, CalcRunLimits, CALC_IMAGE_ID_DOMAIN};
    let c = make_container(halt_empty());
    let id = c.image_id();
    let mut buf = Vec::from(CALC_IMAGE_ID_DOMAIN);
    buf.extend_from_slice(&c.encode().unwrap());
    assert_eq!(id, sha256_bytes(&buf));
    assert_eq!(id, c.image_id());
    let obj = CalcFnObj::build_with_host(c, crate::calc::CalcHostConfig::new()).unwrap();
    assert_eq!(obj.image_id(), id);
    let exec = obj.run(
        b"xy",
        CalcRunLimits {
            cycles: 10_000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert!(exec.result.is_ok());
    assert_eq!(exec.exit_code, 0);
    let rec = obj.receipt(b"xy", &exec);
    assert_eq!(rec.image_id, id);
    assert_eq!(rec.input_hash, sha256_bytes(b"xy"));
    assert_eq!(rec.output_hash, sha256_bytes(&[]));
    assert_eq!(rec.exit_code, 0);
    assert_eq!(rec.abort, None);
    let exec2 = obj.run(
        b"xy",
        CalcRunLimits {
            cycles: 10_000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert_eq!(obj.receipt(b"xy", &exec), obj.receipt(b"xy", &exec2));
}

fn w_addi(rd: u8, rs1: u8, imm: i32) -> u32 {
    0x13u32 | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (((imm as u32) & 0xFFF) << 20)
}
fn w_lui(rd: u8, imm20: u32) -> u32 {
    0x37u32 | ((rd as u32) << 7) | ((imm20 & 0xFFFFF) << 12)
}
fn w_load(f3: u32, rd: u8, rs1: u8, imm: i32) -> u32 {
    0x03u32 | ((rd as u32) << 7) | (f3 << 12) | ((rs1 as u32) << 15) | (((imm as u32) & 0xFFF) << 20)
}
fn w_store(f3: u32, rs1: u8, rs2: u8, imm: i32) -> u32 {
    let imm = (imm as u32) & 0xFFF;
    0x23u32 | (f3 << 12) | ((rs1 as u32) << 15) | ((rs2 as u32) << 20) | ((imm & 0x1F) << 7) | ((imm >> 5) << 25)
}
fn w_jalr(rd: u8, rs1: u8, imm: i32) -> u32 {
    0x67u32 | ((rd as u32) << 7) | ((rs1 as u32) << 15) | (((imm as u32) & 0xFFF) << 20)
}
fn push_w(p: &mut Vec<u8>, w: u32) {
    p.extend_from_slice(&w.to_le_bytes());
}
fn li_addr(p: &mut Vec<u8>, rd: u8, addr: u64) {
    let mut hi = (addr >> 12) as i64;
    let mut lo = (addr & 0xFFF) as i64;
    if lo >= 0x800 {
        hi += 1;
        lo -= 0x1000;
    }
    push_w(p, w_lui(rd, hi as u32));
    push_w(p, w_addi(rd, rd, lo as i32));
}
fn halt_exit(p: &mut Vec<u8>, exit: i32, out_len: i32) {
    push_w(p, w_addi(10, 0, exit));
    push_w(p, w_addi(11, 0, out_len));
    push_w(p, w_addi(17, 0, 0));
    push_w(p, 0x0000_0073);
}

fn run_interp(c: &CalcContainer, input: &[u8], budget: u64) -> Result<crate::calc::interp::CalcRun, (crate::calc::interp::CalcAbort, u64)> {
    let module = verify_container(c).unwrap();
    crate::calc::interp::run(&module, c, input, budget, None)
}

#[cfg(feature = "calc-aot")]
fn assert_aot_parity(c: &CalcContainer, input: &[u8], budget: u64) {
    use crate::calc::aot;
    use crate::calc::interp;
    let module = verify_container(c).unwrap();
    let (ir, iregs, imem, _) = interp::run_with_state(&module, c, input, budget);
    let f = aot::aot_compile(&module).expect("aot compiles");
    let (out, aregs, amem) = f.run_with_state(&module, c, input, budget);
    let icyc = match &ir {
        Ok(r) => r.cycles_used,
        Err((_, u)) => *u,
    };
    assert_eq!(out.cycles_used, icyc, "cycle totals diverge");
    match &ir {
        Ok(r) => {
            assert_eq!(out.abort, None);
            assert_eq!(out.output.as_deref().unwrap_or(&[]), r.output.as_slice());
        }
        Err((interp::CalcAbort::OutOfGas, _)) => assert_eq!(out.abort, Some(aot::AotAbort::OutOfGas)),
        Err((interp::CalcAbort::GuestExit(code), _)) => {
            assert_eq!(out.abort, Some(aot::AotAbort::GuestExit));
            assert_eq!(out.exit_code, *code);
        }
        Err((interp::CalcAbort::Trap(_), _)) => {
            assert!(matches!(out.abort, Some(aot::AotAbort::Trap) | Some(aot::AotAbort::BadTarget)));
        }
        Err((interp::CalcAbort::Deadline, _)) => panic!("deadline"),
    }
    if !imem.is_empty() {
        assert_eq!(iregs, aregs);
        assert_eq!(imem, amem);
    }
}

/// `data_base = align16(rodata_end)`: init is copied there even when
/// `rodata_len` is not a multiple of 8; a store into the alignment hole traps.
#[test]
fn data_base_align16_init_and_hole_is_readonly() {
    use crate::calc::interp::CalcAbort;
    let word: u64 = 0x0102_0304_0506_0708;
    // lui+addi (2) + ld + sd + 4-word halt = 8 ins, 32 bytes.
    let code_len = 32u64;
    let rodata = vec![0u8; 13];
    assert_ne!(rodata.len() % 8, 0);
    let rodata_base = (CODE_BASE + code_len + 15) & !15;
    let data_base = (rodata_base + 13 + 15) & !15;
    assert!(data_base > rodata_base + 13);
    let mut code = Vec::new();
    li_addr(&mut code, 5, data_base);
    push_w(&mut code, w_load(3, 6, 5, 0));
    push_w(&mut code, w_store(3, 12, 6, 0));
    halt_exit(&mut code, 0, 8);
    assert_eq!(code.len() as u64, code_len);
    let c = CalcContainer {
        profile: CALC_PROFILE_RV64IM,
        abi: CALC_ABI_BYTES_TO_BYTES,
        mem_kib: 8,
        entry: CODE_BASE as u32,
        code,
        rodata,
        init: word.to_le_bytes().to_vec(),
        bss_len: 0,
        input_max: 64,
        output_max: 64,
    };
    assert_eq!(c.layout().data_base, data_base);
    let res = run_interp(&c, &[], 10_000).expect("init load");
    assert_eq!(u64::from_le_bytes(res.output[..8].try_into().unwrap()), word);

    // 2 (li) + 1 (sb) + 4 (halt) = 28 bytes
    let hole_code_len = 28u64;
    let hole_ro_base = (CODE_BASE + hole_code_len + 15) & !15;
    let hole_data_base = (hole_ro_base + 13 + 15) & !15;
    assert!(hole_data_base > hole_ro_base + 13);
    let mut store_hole = Vec::new();
    li_addr(&mut store_hole, 5, hole_ro_base + 13);
    push_w(&mut store_hole, w_store(0, 5, 0, 0)); // sb x0, 0(t0) at rodata_end
    halt_exit(&mut store_hole, 0, 0);
    assert_eq!(store_hole.len() as u64, hole_code_len);
    let c2 = CalcContainer {
        profile: CALC_PROFILE_RV64IM,
        abi: CALC_ABI_BYTES_TO_BYTES,
        mem_kib: 8,
        entry: CODE_BASE as u32,
        code: store_hole,
        rodata: vec![0u8; 13],
        init: word.to_le_bytes().to_vec(),
        bss_len: 0,
        input_max: 64,
        output_max: 64,
    };
    match run_interp(&c2, &[], 10_000) {
        Err((CalcAbort::Trap(t), _)) => {
            assert!(t.contains("read-only"), "{t}");
        }
        other => panic!("store into alignment hole must trap, got {other:?}"),
    }
}

/// Misaligned lw/ld/sw/sd, including accesses that cross an 8-byte boundary.
#[test]
fn misaligned_lw_ld_sw_sd_crossing_8byte() {
    use crate::calc::interp::CalcAbort;
    let mut input = [0u8; 16];
    for (i, b) in input.iter_mut().enumerate() {
        *b = 0x10 + i as u8;
    }
    // lw at +5 crosses the 8-byte lane (bytes 5..8); ld at +7 crosses (bytes 7..14).
    // sw at output+1 and sd at output+7 are misaligned stores (no overlap).
    let mut code = Vec::new();
    push_w(&mut code, w_load(2, 5, 10, 5)); // lw t0, 5(a0)
    push_w(&mut code, w_load(3, 6, 10, 7)); // ld t1, 7(a0)
    push_w(&mut code, w_store(2, 12, 5, 1)); // sw t0, 1(a2)
    push_w(&mut code, w_store(3, 12, 6, 7)); // sd t1, 7(a2)
    halt_exit(&mut code, 0, 16);
    let c = make_container(code);
    let res = match run_interp(&c, &input, 10_000) {
        Ok(r) => r,
        Err((CalcAbort::Trap(t), _)) => panic!("misaligned access trapped: {t}"),
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(res.output.len(), 16);
    let lw = u32::from_le_bytes(input[5..9].try_into().unwrap());
    let ld = u64::from_le_bytes(input[7..15].try_into().unwrap());
    assert_eq!(&res.output[1..5], &lw.to_le_bytes());
    assert_eq!(&res.output[7..15], &ld.to_le_bytes());
    #[cfg(feature = "calc-aot")]
    assert_aot_parity(&c, &input, 10_000);
}

#[test]
fn unmapped_low_page_traps_load_store_ecall_jalr() {
    use crate::calc::interp::CalcAbort;
    let trap = |code: Vec<u8>, why: &str| {
        let c = make_container(code);
        match run_interp(&c, &[], 10_000) {
            Err((CalcAbort::Trap(t), _)) => {
                assert!(
                    t.contains("unmapped") || t.contains("invalid target") || t.contains("read-only"),
                    "{why}: {t}"
                );
            }
            other => panic!("{why}: expected trap, got {other:?}"),
        }
        #[cfg(feature = "calc-aot")]
        assert_aot_parity(&c, &[], 10_000);
        let _ = why;
    };
    // lw t0, 0(x0) at address 0
    let mut load0 = Vec::new();
    push_w(&mut load0, w_load(2, 5, 0, 0));
    halt_exit(&mut load0, 0, 0);
    trap(load0, "load 0");
    // lw at 0x800
    let mut load_mid = Vec::new();
    li_addr(&mut load_mid, 5, 0x800);
    push_w(&mut load_mid, w_load(2, 6, 5, 0));
    halt_exit(&mut load_mid, 0, 0);
    trap(load_mid, "load 0x800");
    // sd at 0
    let mut store0 = Vec::new();
    push_w(&mut store0, w_store(3, 0, 5, 0));
    halt_exit(&mut store0, 0, 0);
    trap(store0, "store 0");
    // jalr to 0
    let mut j0 = Vec::new();
    push_w(&mut j0, w_jalr(0, 0, 0));
    halt_exit(&mut j0, 0, 0);
    trap(j0, "jalr 0");
    // jalr to 0x800
    let mut j800 = Vec::new();
    li_addr(&mut j800, 5, 0x800);
    push_w(&mut j800, w_jalr(0, 5, 0));
    halt_exit(&mut j800, 0, 0);
    trap(j800, "jalr 0x800");
    // hash ecall buffer at src=0
    let mut ecall0 = Vec::new();
    push_w(&mut ecall0, w_addi(10, 0, 0)); // a0 = 0
    push_w(&mut ecall0, w_addi(11, 0, 1)); // a1 = 1
    push_w(&mut ecall0, w_addi(17, 0, 1)); // a7 = SHA-256
    push_w(&mut ecall0, 0x0000_0073);
    halt_exit(&mut ecall0, 0, 0);
    trap(ecall0, "ecall src 0");
}

#[test]
fn halt_nonzero_exit_is_guest_exit_and_charges_cycles() {
    use crate::calc::interp::CalcAbort;
    use crate::calc::{CalcFnObj, CalcRunLimits, CalcAbort2};
    let mut code = Vec::new();
    halt_exit(&mut code, 7, 0);
    let c = make_container(code);
    match run_interp(&c, &[], 10_000) {
        Err((CalcAbort::GuestExit(7), used)) => {
            assert!(used > 0, "nonzero HALT must charge consumed cycles, used={used}");
        }
        other => panic!("expected GuestExit(7), got {other:?}"),
    }
    #[cfg(feature = "calc-aot")]
    assert_aot_parity(&c, &[], 10_000);
    let obj = CalcFnObj::build_with_host(c, crate::calc::CalcHostConfig::new()).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 10_000,
            resource: i64::MAX,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Err(CalcAbort2::Trap));
    assert_eq!(exec.exit_code, 7);
    assert!(exec.cycles_used > 0);
    let rec = obj.receipt(&[], &exec);
    assert_eq!(rec.exit_code, 7);
    assert_eq!(rec.abort, Some(CalcAbort2::Trap));
    assert_eq!(rec.output_hash, crate::calc::sha256_bytes(&[]));
}

#[test]
fn ecall_hint_range_traps() {
    use crate::calc::interp::CalcAbort;
    for a7 in [16i32, 20, 31] {
        let mut code = Vec::new();
        push_w(&mut code, w_addi(17, 0, a7));
        push_w(&mut code, 0x0000_0073);
        halt_exit(&mut code, 0, 0);
        let c = make_container(code);
        match run_interp(&c, &[], 10_000) {
            Err((CalcAbort::Trap(t), _)) => {
                assert!(t.contains("hint") || t.contains("ecall"), "a7={a7}: {t}");
            }
            other => panic!("a7={a7}: expected hint trap, got {other:?}"),
        }
        #[cfg(feature = "calc-aot")]
        assert_aot_parity(&c, &[], 10_000);
    }
}

#[test]
fn halt_output_len_over_max_traps() {
    use crate::calc::interp::CalcAbort;
    let mut code = Vec::new();
    halt_exit(&mut code, 0, 65); // make_container output_max = 64
    let c = make_container(code);
    match run_interp(&c, &[], 10_000) {
        Err((CalcAbort::Trap(t), _)) => {
            assert!(t.contains("output"), "{t}");
        }
        other => panic!("expected output_max trap, got {other:?}"),
    }
    #[cfg(feature = "calc-aot")]
    assert_aot_parity(&c, &[], 10_000);
}

/// V1 rejects `mem_kib > 64` at verify. Wire decode accepts any
/// `mem_kib >= 4`. A 64 KiB container keeps the pinned layout, interpreter
/// cycles, and `CalcFnObj` setup/resource charges.
#[test]
fn mem_cap_is_config_and_v1_layout_stays_pinned() {
    use crate::calc::config::CALC_TESTKIT_MEM_KIB_CEILING;
    use crate::calc::container::{compute_layout, CALC_MEM_KIB_MAX};
    use crate::calc::interp::{self, CalcRun};
    use crate::calc::{
        CalcConfig, CalcFnObj, CalcRunLimits, CALC_CONFIG_V1, CALC_MEM_RESOURCE_PER_KIB,
        CALC_SETUP_BASE_CYCLES, CALC_SETUP_PER_KIB_CYCLES,
    };

    let mut over = make_container_profile(CALC_PROFILE_RV64IM, halt_empty());
    over.mem_kib = CALC_MEM_KIB_MAX + 1;
    assert!(over.validate_header(&CALC_CONFIG_V1).is_err());
    assert!(verify_container(&over).is_err());
    assert!(CalcFnObj::build(over.clone()).is_err());
    assert!(crate::calc::pack_container(
        CALC_MEM_KIB_MAX + 1,
        CODE_BASE as u32,
        16,
        16,
        halt_empty(),
        vec![],
        vec![],
    )
    .is_err());
    let mut bytes = make_container_profile(CALC_PROFILE_RV64IM, halt_empty())
        .encode()
        .unwrap();
    bytes[8..12].copy_from_slice(&(CALC_MEM_KIB_MAX + 1).to_be_bytes());
    let (decoded, _) = CalcContainer::decode(&bytes).expect("decode accepts mem_kib=65");
    assert_eq!(decoded.mem_kib, CALC_MEM_KIB_MAX + 1);
    assert!(verify_container(&decoded).is_err());
    let mut low = bytes.clone();
    low[8..12].copy_from_slice(&3u32.to_be_bytes());
    assert!(CalcContainer::decode(&low).is_err());

    let cap = CalcConfig::testkit(128).unwrap();
    let mut wide = decoded.clone();
    wide.mem_kib = 128;
    let wide_m = crate::calc::verify::verify_container_with(&wide, &cap).unwrap();
    assert_eq!(wide_m.layout_snapshot().usable_len, 128 * 1024);
    assert_eq!(wide_m.layout_snapshot().mem_len, CODE_BASE + 128 * 1024);
    wide.mem_kib = 129;
    assert!(crate::calc::verify::verify_container_with(&wide, &cap).is_err());
    assert!(CalcConfig::testkit(CALC_TESTKIT_MEM_KIB_CEILING).is_ok());
    assert!(CalcConfig::testkit(CALC_TESTKIT_MEM_KIB_CEILING + 1).is_err());

    let mut c = make_container_profile(CALC_PROFILE_RV64IM, halt_empty());
    c.mem_kib = CALC_MEM_KIB_MAX;
    c.input_max = 16;
    c.output_max = 16;
    let lay = c.layout();
    assert_eq!(
        lay,
        compute_layout(
            c.code.len() as u64,
            c.rodata.len() as u64,
            c.init.len() as u64,
            c.bss_len as u64,
            c.input_max as u64,
            c.output_max as u64,
            c.mem_kib as u64 * 1024,
        )
    );
    assert_eq!(lay.usable_len, 64 * 1024);
    assert_eq!(lay.mem_len, CODE_BASE + 64 * 1024);
    let consensus = verify_container(&c).unwrap();
    assert_eq!(consensus.layout_snapshot(), lay);
    let (r1, _, mem1, _) = interp::run_with_state(&consensus, &c, &[], 10_000);
    assert_eq!(mem1.len() as u64, lay.mem_len);
    assert_eq!(r1, Ok(CalcRun { output: vec![], cycles_used: 4 }));

    let obj = CalcFnObj::build(c).unwrap();
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 1_000_000,
            resource: 1_000_000,
            deadline: None,
        },
    );
    assert_eq!(exec.result, Ok(vec![]));
    let setup = CALC_SETUP_BASE_CYCLES + 64 * CALC_SETUP_PER_KIB_CYCLES;
    assert_eq!(setup, 64 + 64 * 16);
    assert_eq!(exec.cycles_used, setup + 4);
    assert_eq!(exec.cycles_used, 1092);
    assert_eq!(exec.resource, 64 * CALC_MEM_RESOURCE_PER_KIB);
    assert_eq!(exec.resource, 512);
}

