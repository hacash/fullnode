//! ISA-level semantics suite for the calc VM, in riscv-tests rv64ui/um/uc
//! style: every whitelisted RV64IMC+Zbb+Zbc instruction form runs through
//! the interpreter and is checked against HOST-side Rust reference
//! semantics (independent of the interpreter implementation). Under
//! `--features calc-aot` the same programs additionally run through the
//! AOT backend with whole-state parity (registers, memory, cycles, abort
//! class) against the interpreter.
//!
//! Case programs and expectations live in `calc_isa_gen.rs` (shared with
//! the git-ignored ckb-vm/softcore differential drivers in `tests.loc/`).

#[path = "calc_isa_gen.rs"]
mod isa_gen;

use isa_gen::{all_cases, record_words, rodata_trap_parts, Case, Outcome};
use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::verify_container;

const BIG: u64 = 10_000_000;

/// Run one case on the interpreter and check everything the case declares.
fn run_interp(case: &Case, container: &CalcContainer, budget: u64) {
    let module = verify_container(container)
        .unwrap_or_else(|e| panic!("{}: container rejected: {e:?}", case.name));
    let (r, regs, mem, _trace) = interp::run_with_state(&module, container, &[], budget);
    let used = match &r {
        Ok(x) => x.cycles_used,
        Err((_, u)) => *u,
    };
    match &case.outcome {
        Outcome::Ok => {
            assert!(r.is_ok(), "{}: expected clean return, got {r:?}", case.name);
            assert_eq!(mem.len(), isa_gen::mem_len() as usize, "{}: state snapshot", case.name);
            let got: Vec<u64> = record_words(&mem)
                .into_iter()
                .take(case.expect.len())
                .collect();
            for (k, (g, e)) in got.iter().zip(case.expect.iter()).enumerate() {
                assert_eq!(g, e, "{}: record slot {} mismatch", case.name, k + 1);
            }
            assert_eq!(regs[2], isa_gen::mem_len(), "{}: sp must be preserved", case.name);
            if !case.ra_free {
                assert_eq!(
                    regs[1],
                    vm::calc::CALC_RETURN_SENTINEL,
                    "{}: ra must be preserved",
                    case.name
                );
            }
        }
        Outcome::Trap => {
            assert!(
                matches!(&r, Err((CalcAbort::Trap(_), _))),
                "{}: expected Trap, got {r:?}",
                case.name
            );
        }
        Outcome::OutOfGas(expect_used) => {
            assert!(
                matches!(&r, Err((CalcAbort::OutOfGas, _))),
                "{}: expected OutOfGas, got {r:?}",
                case.name
            );
            assert_eq!(
                &used,
                expect_used,
                "{}: OOG cycles (failing charge must be excluded)",
                case.name
            );
        }
    }
    if let Some(c) = case.cycles {
        assert_eq!(used, c, "{}: exact cycle total", case.name);
    }
}

#[cfg(feature = "calc-aot")]
fn run_aot_parity(case: &Case, container: &CalcContainer, budget: u64) {
    use vm::calc::aot::{self, AotAbort};
    let module = verify_container(container).unwrap();
    if case.aot_declines {
        assert!(
            aot::aot_compile(&module).is_none(),
            "{}: AOT must decline ecall containers",
            case.name
        );
        return;
    }
    let f = aot::aot_compile(&module)
        .unwrap_or_else(|| panic!("{}: AOT declined a supported case", case.name));
    let (ir, iregs, imem, _) = interp::run_with_state(&module, container, &[], budget);
    let icyc = match &ir {
        Ok(x) => x.cycles_used,
        Err((_, u)) => *u,
    };
    let (out, aregs, amem) = f.run_with_state(&module, container, &[], budget);
    assert_eq!(out.cycles_used, icyc, "{}: AOT cycle total diverges", case.name);
    match &ir {
        Ok(_) => {
            assert_eq!(out.abort, None, "{}: AOT aborted where interp returned", case.name);
            assert_eq!(
                out.output.as_deref().unwrap_or(&[]),
                ir.as_ref().unwrap().output,
                "{}: AOT output diverges",
                case.name
            );
        }
        Err((CalcAbort::OutOfGas, _)) => {
            assert_eq!(
                out.abort,
                Some(AotAbort::OutOfGas),
                "{}: AOT abort class (expected OOG)",
                case.name
            );
        }
        Err((CalcAbort::Trap(_), _)) => {
            assert!(
                matches!(out.abort, Some(AotAbort::Trap) | Some(AotAbort::BadTarget)),
                "{}: AOT abort class (expected trap-like), got {:?}",
                case.name,
                out.abort
            );
        }
        Err((CalcAbort::Deadline, _)) => unreachable!("no deadline set"),
    }
    if !imem.is_empty() {
        assert_eq!(iregs, aregs, "{}: AOT final registers diverge", case.name);
        assert_eq!(imem, amem, "{}: AOT final memory diverges", case.name);
    }
}

fn case_budget(case: &Case) -> u64 {
    match case.outcome {
        Outcome::OutOfGas(_) => 1000,
        _ => BIG,
    }
}

/// The whole ISA matrix through the interpreter against host references,
/// with AOT whole-state parity when the feature is enabled.
#[test]
fn isa_interpreter_matches_host_reference() {
    for case in all_cases() {
        let container = case.container();
        run_interp(&case, &container, case_budget(&case));
        #[cfg(feature = "calc-aot")]
        run_aot_parity(&case, &container, case_budget(&case));
    }
}

/// Store into the read-only rodata region must trap (the code region is
/// covered by `trap/store-code`; this completes the readonly matrix).
#[test]
fn store_into_rodata_traps() {
    let (mut code, rodata, _) = rodata_trap_parts();
    // rodata_base = align16(code_len); rebuild the first instruction
    // (addi x30, x0, imm) with that address as the immediate.
    let rodata_base = (code.len() as u64 + 15) & !15;
    assert!(rodata_base <= 2047, "patch immediate must fit i12");
    let word0 = 0x13u32 | (30 << 7) | ((rodata_base as u32) << 20);
    code[0..4].copy_from_slice(&word0.to_le_bytes());
    let container = CalcContainer {
        abi: vm::calc::container::CALC_ABI_BYTES_TO_BYTES,
        mem_kib: isa_gen::MEM_KIB,
        entry: 0,
        code,
        rodata,
        init: Vec::new(),
        input_max: isa_gen::INPUT_MAX,
        output_max: isa_gen::OUTPUT_MAX,
    };
    let case = Case::trap("trap/store-rodata", container.code.clone());
    run_interp(&case, &container, BIG);
    #[cfg(feature = "calc-aot")]
    run_aot_parity(&case, &container, BIG);
}

/// OOG boundary behavior: budgets at exactly the success cost and one
/// cycle below it. The failing charge is excluded on shortfall.
#[test]
fn oog_boundary_cycles() {
    // minimal program: exactly 2 cycles.
    let code = vec![0x13, 0x05, 0x00, 0x00, 0x67, 0x80, 0x00, 0x00];
    let container = isa_gen::container_for(code);
    let module = verify_container(&container).unwrap();
    let (r, _, _, _) = interp::run_with_state(&module, &container, &[], 2);
    assert!(r.is_ok(), "exact budget must succeed: {r:?}");
    assert_eq!(r.unwrap().cycles_used, 2);
    // one cycle short: the single 2-cycle block charge is excluded -> 0 used
    let (r2, _, _, _) = interp::run_with_state(&module, &container, &[], 1);
    assert!(matches!(r2, Err((CalcAbort::OutOfGas, _))), "{r2:?}");
    assert_eq!(r2.unwrap_err().1, 0, "failing charge excluded");
    let (r3, _, _, _) = interp::run_with_state(&module, &container, &[], 0);
    assert!(matches!(r3, Err((CalcAbort::OutOfGas, _))));
    assert_eq!(r3.unwrap_err().1, 0);
    #[cfg(feature = "calc-aot")]
    {
        let f = vm::calc::aot::aot_compile(&module).unwrap();
        for (budget, ok) in [(2u64, true), (1, false), (0, false)] {
            let (out, _, _) = f.run_with_state(&module, &container, &[], budget);
            assert_eq!(
                out.abort.is_none(),
                ok,
                "AOT at budget {budget}: {:?}",
                out.abort
            );
            assert_eq!(
                out.cycles_used,
                if ok { 2 } else { 0 },
                "AOT cycles at budget {budget}"
            );
        }
    }
    // cost-table program at its exact total (55) and one below.
    let case55 = all_cases()
        .into_iter()
        .find(|c| c.name == "cyc/cost-table")
        .expect("cost-table case");
    let total = case55.cycles.unwrap();
    let c55 = case55.container();
    let m55 = verify_container(&c55).unwrap();
    let (r, _, _, _) = interp::run_with_state(&m55, &c55, &[], total);
    assert!(r.is_ok());
    assert_eq!(r.unwrap().cycles_used, total);
    let (r, _, _, _) = interp::run_with_state(&m55, &c55, &[], total - 1);
    assert!(matches!(r, Err((CalcAbort::OutOfGas, _))));
    assert_eq!(r.unwrap_err().1, 0, "single-block program: charge excluded");
    #[cfg(feature = "calc-aot")]
    {
        let f = vm::calc::aot::aot_compile(&m55).unwrap();
        let (out, _, _) = f.run_with_state(&m55, &c55, &[], total);
        assert!(out.abort.is_none());
        assert_eq!(out.cycles_used, total);
        let (out, _, _) = f.run_with_state(&m55, &c55, &[], total - 1);
        assert_eq!(out.abort, Some(vm::calc::aot::AotAbort::OutOfGas));
        assert_eq!(out.cycles_used, 0);
    }
    // multi-block partial progress: [addi,beq](2) + [li,jalr](2); at
    // budget 3 the first block is charged, the second cannot be -> OOG
    // with exactly 2 cycles kept.
    let bs = all_cases()
        .into_iter()
        .find(|c| c.name == "cyc/branch-split")
        .expect("branch-split case");
    let cbs = bs.container();
    let mbs = verify_container(&cbs).unwrap();
    let (r, _, _, _) = interp::run_with_state(&mbs, &cbs, &[], 3);
    assert!(matches!(r, Err((CalcAbort::OutOfGas, _))), "{r:?}");
    assert_eq!(r.unwrap_err().1, 2, "partial progress kept");
    #[cfg(feature = "calc-aot")]
    {
        let f = vm::calc::aot::aot_compile(&mbs).unwrap();
        let (out, _, _) = f.run_with_state(&mbs, &cbs, &[], 3);
        assert_eq!(out.abort, Some(vm::calc::aot::AotAbort::OutOfGas));
        assert_eq!(out.cycles_used, 2);
    }
}

/// Decode strictness: reserved compressed encodings and malformed tails
/// must be rejected by the verifier (whole-code decode). Halfword values
/// cross-checked against the spec tables / llvm-mc.
#[test]
fn decode_strictness_reserved_forms() {
    let err = |halfwords: &[u16]| {
        let mut code = Vec::new();
        for h in halfwords {
            code.extend_from_slice(&h.to_le_bytes());
        }
        let c = isa_gen::container_for(code);
        verify_container(&c).is_err()
    };
    // c.addi4spn with imm == 0 (reserved).
    assert!(err(&[0x0000]));
    // c.addi16sp imm == 0 (reserved): funct3 011, rd = x2, payload zero.
    assert!(err(&[0x6101]));
    // c.lui imm == 0, rd != x2 (reserved).
    assert!(err(&[0x6281]));
    // c.addiw rd == 0 (reserved in RV64).
    assert!(err(&[0x2001]));
    // c.ebreak: funct3 100, bit12 1, rs2 0, rd 0 (not whitelisted).
    assert!(err(&[0x9002]));
    // c.jr with rs1 == x0 (reserved): funct3 100, bit12 0, rs2 0, rd 0.
    assert!(err(&[0x8002]));
    // quadrant-0 c.fld (funct3 001) / c.fsd (funct3 101).
    assert!(err(&[0x2000]));
    assert!(err(&[0xA000]));
    // quadrant-2 c.fldsp (funct3 001, op 10) / c.fsdsp (funct3 101).
    assert!(err(&[0x2002]));
    assert!(err(&[0xA002]));
    // legal tail reference: these MUST verify.
    assert!(!err(&[0x0001])); // c.nop
    assert!(!err(&[0x6141])); // c.addi16sp 16
    // trailing truncated 32-bit instruction (last 2 bytes look like the
    // start of a 4-byte word with low bits 11).
    assert!(err(&[0x0281, 0x8000]));
    // trailing single byte can never happen (code_len is even), but a
    // lone half 0x0281 = addi with rd... 0x0281: imm=2 rd=0? decodes as
    // addi x0, x0, 2 (legal HINT) - must NOT error:
    assert!(!err(&[0x0281]));
}

/// Input echo kernel: bytes->bytes ABI end to end through CalcFnObj
/// (entry convention a0/a1/a2/a3, output produced by writing the output
/// region and setting a0 before the sentinel return).
#[test]
fn input_echo_end_to_end() {
    // [0] x5 = src (a0) ; [1] x6 = len (a1) ; [2] x7 = dst (a2) ;
    // [3] i = 0 ; head=[4]: x30 = i+src ; lb x29, 0(x30) ; x31 = i+dst ;
    // sb x29, 0(x31) ; i++ ; bltu i, len, head ; a0 = len ; ret.
    let mut a = isa_gen::Asm::new();
    a.i(0, 5, 10, 0x13, 0);
    a.i(0, 6, 11, 0x13, 0);
    a.i(0, 7, 12, 0x13, 0);
    a.i(0, 28, 0, 0x13, 0);
    let head = 4;
    a.r(0, 0, 30, 28, 5, 0b0110011); // add x30, x28(i), x5
    a.i(0, 29, 30, 0x03, 0); // lb x29, 0(x30)
    a.r(0, 0, 31, 28, 7, 0b0110011); // add x31, x28, x7
    a.s(0, 31, 29, 0); // sb x29, 0(x31)
    a.i(0, 28, 28, 0x13, 1); // addi i, i, 1
    a.b(6, 28, 6, head); // bltu i, len, head
    a.i(0, 10, 6, 0x13, 0); // a0 = len
    a.i(0, 0, 1, 0x67, 0); // ret
    let container = isa_gen::container_for(a.finish());
    let obj = vm::calc::CalcFnObj::build(container).unwrap();
    let limits = vm::calc::CalcRunLimits {
        cycles: 1_000_000,
        resource: i64::MAX,
        value_size: usize::MAX,
        deadline: None,
    };
    let input: Vec<u8> = (0..100u32).map(|i| (i * 37 + 11) as u8).collect();
    let exec = obj.run(&input, limits);
    let out = exec.result.expect("echo must succeed");
    assert_eq!(out, input);
    // determinism + sane accounting
    let exec2 = obj.run(&input, limits);
    assert_eq!(exec.cycles_used, exec2.cycles_used, "deterministic cycles");
    assert!(exec.cycles_used > 320, "setup + body cycles");
    // resource = ceil(in/16) + ceil(out/16) + 8 per declared KiB
    let expect_res = ((100 + 15) / 16) * 2 + isa_gen::MEM_KIB as i64 * vm::calc::CALC_MEM_RESOURCE_PER_KIB;
    assert_eq!(exec.resource, expect_res);
}
