//! Multi-engine differential driver over the shared ISA case suite
//! (`vm/tests/calc_isa_gen.rs`): every clean-return case (and every ecall
//! case through the hash syscall shim) runs through
//!
//!   1. the calc interpreter (consensus baseline, full state snapshot),
//!   2. ckb-vm 0.24.13 (ISA_IMC | ISA_B) as a whole-machine oracle,
//!
//! and the engines must agree bit-exactly on the recorded stack slots,
//! the WHOLE final memory image and all 32 registers. Trap/out-of-gas
//! cases are interp↔AOT territory (the committed `calc_isa_suite.rs`);
//! abort taxonomies differ across VM families, so they are skipped here.
//!
//! Business-local test (git-ignored): depends on the ckb-vm dev-dependency
//! pinned in the working-tree Cargo.toml.

#[path = "calc_isa_gen.rs"]
mod isa_gen;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ckb_vm::machine::DefaultMachineBuilder;
use ckb_vm::registers::{A0, A1, A2, A3, SP};
use ckb_vm::{
    CoreMachine, DefaultCoreMachine, Error as CkbError, FlatMemory, Memory, Syscalls, ISA_B,
    ISA_IMC,
};
use isa_gen::{all_cases, Outcome};
use vm::calc::container::CalcContainer;
use vm::calc::interp;
fn verify_container(
    c: &CalcContainer,
) -> Result<vm::calc::verify::CalcModule, String> {
    vm::calc::verify::verify_container(c)
        .map_err(|e| format!("{e:?}"))
}

type Core = DefaultCoreMachine<u64, FlatMemory<u64>>;

/// Hash + HALT ecall shim mirroring the calc builtin contract.
struct HashEcall {
    halted: Arc<AtomicBool>,
}

impl HashEcall {
    fn new() -> Self {
        Self { halted: Arc::new(AtomicBool::new(false)) }
    }
}

impl Syscalls<Core> for HashEcall {
    fn initialize(&mut self, _m: &mut Core) -> Result<(), ckb_vm::Error> {
        Ok(())
    }
    fn ecall(&mut self, m: &mut Core) -> Result<bool, ckb_vm::Error> {
        let a7 = m.registers()[17];
        if a7 == vm::calc::ECALL_HALT {
            self.halted.store(true, Ordering::SeqCst);
            return Ok(true);
        }
        if let Some(kind) = vm::calc::HashKind::from_a7(a7) {
            let src = m.registers()[10];
            let len = m.registers()[11];
            let dst = m.registers()[12];
            let mut buf = vec![0u8; len as usize];
            if m.memory_mut().load_bytes(src, len).map(|b| buf.copy_from_slice(&b)).is_err() {
                return Ok(false);
            }
            let mut out = vec![0u8; kind.digest_len() as usize];
            vm::calc::run_hash(kind, &buf, &mut out);
            m.memory_mut().store_bytes(dst, &out).map(|_| ()).map_err(|e| e)?;
            m.set_register(10, 0);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Run a case's container on ckb-vm with the calc entry convention;
/// returns (registers, full memory) at the sentinel stop.
fn run_ckbvm(
    c: &CalcContainer,
    module: &vm::calc::verify::CalcModule,
    input: &[u8],
) -> Result<([u64; 32], Vec<u8>), String> {
    let lay = module.layout_snapshot();
    let core = Core::new_with_memory(
        ISA_IMC | ISA_B,
        ckb_vm::machine::VERSION2,
        u64::MAX,
        lay.mem_len as usize,
    );
    let sys = HashEcall::new();
    let halted = sys.halted.clone();
    let mut machine = DefaultMachineBuilder::new(core)
        .instruction_cycle_func(Box::new(|_: u64| -> u64 { 1 }))
        .syscall(Box::new(sys))
        .build();
    {
        let m = machine.memory_mut();
        m.store_bytes(
            vm::calc::container::HALT_STUB,
            &vm::calc::container::HALT_STUB_BYTES,
        )
        .unwrap();
        for s in &c.segs {
            m.store_bytes(s.vaddr as u64, &s.data).unwrap();
        }
        m.store_bytes(lay.input_base, input).unwrap();
    }
    machine.set_register(A0, lay.input_base);
    machine.set_register(A1, input.len() as u64);
    machine.set_register(A2, lay.output_base);
    machine.set_register(A3, lay.output_max);
    machine.set_register(SP, lay.stack_top);
    machine.set_register(1, vm::calc::container::HALT_STUB);
    machine.update_pc(c.entry as u64);
    machine.commit_pc();
    let mut decoder = ckb_vm::decoder::build_decoder::<u64>(machine.isa(), machine.version());
    loop {
        if halted.load(Ordering::SeqCst) {
            let mut regs = [0u64; 32];
            for i in 0..32 {
                regs[i] = machine.registers()[i];
            }
            let mut mem = vec![0u8; lay.mem_len as usize];
            machine
                .memory_mut()
                .load_bytes(0, lay.mem_len)
                .map(|b| mem.copy_from_slice(&b))
                .map_err(|e| e.to_string())?;
            return Ok((regs, mem));
        }
        machine
            .step(&mut decoder)
            .map_err(|e: CkbError| format!("{e:?}"))?;
    }
}

#[test]
fn isa_cases_match_ckbvm() {
    let cases = all_cases();
    let mut ran = 0usize;
    for case in cases {
        if case.outcome != Outcome::Ok {
            continue; // trap/OOG classes are pinned by the AOT parity suite
        }
        let container = case.container();
        let module = verify_container(&container)
            .unwrap_or_else(|e| panic!("{}: verify: {e:?}", case.name));
        let (ir, iregs, imem, _) =
            interp::run_with_state(&module, &container, &[], 10_000_000);
        let run = ir.unwrap_or_else(|(a, u)| panic!("{}: interp abort {a:?} after {u}", case.name));
        let (cregs, cmem) = run_ckbvm(&container, &module, &[])
            .unwrap_or_else(|e| panic!("{}: ckb-vm failed: {e}", case.name));
        // whole memory image
        if imem != cmem {
            let off = imem
                .iter()
                .zip(cmem.iter())
                .position(|(x, y)| x != y)
                .unwrap_or(imem.len().min(cmem.len()));
            panic!(
                "{}: memory diverges at byte {off}: interp={:#x} ckb={:#x}",
                case.name,
                imem.get(off).copied().unwrap_or(u8::MAX),
                cmem.get(off).copied().unwrap_or(u8::MAX),
            );
        }
        // register file (ckb-vm keeps its own pc/exit state outside x0..x31)
        for i in 0..32 {
            if iregs[i] != cregs[i] {
                panic!(
                    "{}: register x{i} diverges: interp={:#x} ckb={:#x}",
                    case.name, iregs[i], cregs[i]
                );
            }
        }
        let _ = run;
        ran += 1;
    }
    assert!(ran >= 70, "expected the RV64IM clean-return matrix, ran {ran}");
}

/// Random cross-engine fuzz over the same generator's host-reference
/// semantics is covered by the interpreter suite; this directed probe
/// re-runs a handful of record slots through ckb-vm to pin the
/// little-endian memory conventions under load/store mixing.
#[test]
fn ckbvm_endianness_probe() {
    let case = all_cases()
        .into_iter()
        .find(|c| c.name == "mem/stores")
        .expect("mem/stores case");
    let container = case.container();
    let module = verify_container(&container).unwrap();
    let (_cregs, cmem) = run_ckbvm(&container, &module, &[]).unwrap();
    let got = isa_gen::record_words(&cmem, module.layout_snapshot().stack_top);
    for (k, e) in got.iter().zip(case.expect.iter()).enumerate() {
        assert_eq!(*e.0, *e.1, "mem/stores record slot {}", k + 1);
    }
}
