//! Pre-decoded RV64IMC(+Zbb/Zba/Zbs/Zbc) interpreter.
//!
//! Executes a verified [`CalcModule`] over a freshly zeroed flat memory.
//! Cycles are charged in full at basic-block entry (plus hash cost at each
//! `ecall`); every observable behavior — trap messages excluded — is a pure
//! function of (module, input), so consensus results and gas are bit-exact
//! across runs and builds.

use crate::calc::container::CalcContainer;
use crate::calc::decode::{
    BitImmOp, BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp, UnAluOp,
};
use crate::calc::verify::CalcModule;
use crate::calc::{run_hash, HashKind, CALC_RETURN_SENTINEL};

/// Why a calc run stopped before returning normally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcAbort {
    /// Deterministic kernel fault (bad memory access, unknown ecall kind,
    /// misaligned access, write to code, output length overflow, ...).
    Trap(String),
    /// Cycle budget exhausted at the point of the last block/ecall charge.
    OutOfGas,
    /// Host wall-clock deadline elapsed (never set on the consensus path).
    Deadline,
}

impl CalcAbort {
    fn trap(t: &str) -> Self {
        CalcAbort::Trap(t.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcRun {
    pub output: Vec<u8>,
    /// Cycles consumed up to (and including) the charge that completed
    /// before the stop point; for `Ok` runs this is the exact total.
    pub cycles_used: u64,
}

/// Run a verified module. `input.len()` must be within the container's
/// `input_max` (enforced by `Trap` otherwise); `cycle_budget` is the whole
/// allowance; the caller derives it from remaining compute gas. `deadline`
/// is `None` on the consensus path.
#[doc(hidden)]
pub fn run(
    module: &CalcModule,
    container: &CalcContainer,
    input: &[u8],
    cycle_budget: u64,
    deadline: Option<std::time::Instant>,
) -> Result<CalcRun, (CalcAbort, u64)> {
    run_inner(module, container, input, cycle_budget, None, deadline)
}

/// Test/debug entry: additionally snapshots registers and memory at the stop
/// point. Not used by consensus paths.
#[allow(clippy::type_complexity)]
#[allow(dead_code)]
#[doc(hidden)]
pub fn run_with_state(
    module: &CalcModule,
    container: &CalcContainer,
    input: &[u8],
    cycle_budget: u64,
) -> (Result<CalcRun, (CalcAbort, u64)>, [u64; 32], Vec<u8>, Vec<u64>) {
    let mut regs = [0u64; 32];
    let mut mem: Vec<u8> = Vec::new();
    let mut trace: Vec<u64> = Vec::new();
    let r = run_inner(
        module,
        container,
        input,
        cycle_budget,
        Some((&mut regs, &mut mem, &mut trace)),
        None,
    );
    (r, regs, mem, trace)
}

fn run_inner(
    module: &CalcModule,
    container: &CalcContainer,
    input: &[u8],
    cycle_budget: u64,
    mut state_out: Option<(&mut [u64; 32], &mut Vec<u8>, &mut Vec<u64>)>,
    deadline: Option<std::time::Instant>,
) -> Result<CalcRun, (CalcAbort, u64)> {
    let lay = &module.layout;
    let mut mem: Vec<u8> = vec![0u8; lay.mem_len as usize];
    mem[..container.code.len()].copy_from_slice(&container.code);
    mem[lay.rodata_base as usize..(lay.rodata_base as u64 + lay.rodata_len) as usize]
        .copy_from_slice(&container.rodata);
    mem[lay.data_base as usize..(lay.data_base as u64 + lay.init_len) as usize]
        .copy_from_slice(&container.init);
    if input.len() as u64 > lay.input_max {
        return Err((CalcAbort::trap("calc input exceeds declared input_max"), 0));
    }
    let input_base = lay.input_base as usize;
    mem[input_base..input_base + input.len()].copy_from_slice(input);

    let mut r = [0u64; 32];
    r[10] = lay.input_base;
    r[11] = input.len() as u64;
    r[12] = lay.output_base;
    r[13] = lay.output_max;
    r[2] = lay.stack_top;
    r[1] = CALC_RETURN_SENTINEL;

    let insts = &module.insts;
    let addrs = &module.addrs;
    let blocks = &module.blocks;
    let mut cycles = cycle_budget;
    let mut idx = module.entry as usize;
    let used = |cycles: u64| cycle_budget - cycles;

    macro_rules! snap {
        () => {
            if let Some((ro, mo, _to)) = state_out.as_mut() {
                ro.copy_from_slice(&r);
                mo.extend_from_slice(&mem);
            }
        };
    }
    macro_rules! trap {
        ($t:expr) => {{
            snap!();
            return Err((CalcAbort::trap($t), used(cycles)))
        }};
    }
    macro_rules! out_of_gas {
        () => {{
            snap!();
            return Err((CalcAbort::OutOfGas, used(cycles)))
        }};
    }
    // Bounds- and alignment-checked little-endian loads.
    macro_rules! load {
        ($addr:expr, $op:expr) => {{
            let addr: u64 = $addr;
            let op: LoadOp = $op;
            let size = op.size();
            let end = match addr.checked_add(size) {
                Some(e) => e,
                None => trap!("calc load address overflow"),
            };
            if end > lay.mem_len {
                trap!("calc load out of memory bounds");
            }
            if addr % size != 0 {
                trap!("calc misaligned load");
            }
            let b = &mem[addr as usize..end as usize];
            match op {
                LoadOp::Lb => b[0] as i8 as i64 as u64,
                LoadOp::Lh => i16::from_le_bytes([b[0], b[1]]) as i64 as u64,
                LoadOp::Lw => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64 as u64,
                LoadOp::Ld => u64::from_le_bytes(b.try_into().unwrap()),
                LoadOp::Lbu => b[0] as u64,
                LoadOp::Lhu => u16::from_le_bytes([b[0], b[1]]) as u64,
                LoadOp::Lwu => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64,
            }
        }};
    }
    macro_rules! store {
        ($addr:expr, $op:expr, $val:expr) => {{
            let addr: u64 = $addr;
            let op: StoreOp = $op;
            let size = op.size();
            let end = match addr.checked_add(size) {
                Some(e) => e,
                None => trap!("calc store address overflow"),
            };
            if end > lay.mem_len {
                trap!("calc store out of memory bounds");
            }
            if addr < lay.data_base {
                trap!("calc store into read-only region (code/rodata)");
            }
            // Stack-overflow guard: a store intersecting the band between
            // the output region and the stack traps (stack grows down into
            // it). Stores fully inside the output region (end <= guard_base)
            // and stores at or above the stack bottom pass.
            if addr < lay.stack_bottom && end > lay.guard_base {
                trap!("calc store into the stack-overflow guard band");
            }
            if addr % size != 0 {
                trap!("calc misaligned store");
            }
            let val: u64 = $val;
            let b = &mut mem[addr as usize..end as usize];
            match op {
                StoreOp::Sb => b[0] = val as u8,
                StoreOp::Sh => b[..2].copy_from_slice(&(val as u16).to_le_bytes()),
                StoreOp::Sw => b[..4].copy_from_slice(&(val as u32).to_le_bytes()),
                StoreOp::Sd => b.copy_from_slice(&val.to_le_bytes()),
            }
        }};
    }

    'blocks: loop {
        if let Some(deadline) = deadline {
            if std::time::Instant::now() >= deadline {
                snap!();
                return Err((CalcAbort::Deadline, used(cycles)));
            }
        }
        let blk = &blocks[module.block_of_inst[idx] as usize];
        // Charge the remaining static cost of this block from the entry
        // instruction. A dynamic `jalr` into the middle of a block must not
        // pay for instructions that were skipped.
        let cost = module.suffix_cycles[idx];
        if cycles < cost {
            out_of_gas!();
        }
        cycles -= cost;
        let end = blk.end as usize;
        while idx < end {
            let inst = insts[idx];
            if let Some((_, _, to)) = state_out.as_mut() {
                to.push(addrs[idx] as u64);
            }
            match inst {
                Inst::Lui { rd, val } => {
                    if rd != 0 {
                        r[rd as usize] = val as u64;
                    }
                }
                Inst::Auipc { rd, val } => {
                    if rd != 0 {
                        r[rd as usize] = (addrs[idx] as u64).wrapping_add(val as u64);
                    }
                }
                Inst::Jal { rd, target } => {
                    if rd != 0 {
                        r[rd as usize] = addrs[idx] as u64 + 4;
                    }
                    idx = target as usize;
                    continue 'blocks;
                }
                Inst::Jalr { rd, rs1, imm } => {
                    let dest = r[rs1 as usize].wrapping_add(imm as u64) & !1u64;
                    let ret_addr = module.inst_len(idx) as u64 + addrs[idx] as u64;
                    if dest == CALC_RETURN_SENTINEL {
                        let n = r[10];
                        if n > lay.output_max {
                            return Err((
                                CalcAbort::trap("calc output length exceeds output_max"),
                                used(cycles),
                            ));
                        }
                        let out =
                            mem[lay.output_base as usize..(lay.output_base + n) as usize].to_vec();
                        snap!();
                        return Ok(CalcRun { output: out, cycles_used: used(cycles) });
                    }
                    if rd != 0 {
                        r[rd as usize] = ret_addr;
                    }
                    match module.inst_at_addr(dest) {
                        Some(next) => {
                            idx = next as usize;
                            continue 'blocks;
                        }
                        None => trap!("calc jalr to invalid target"),
                    }
                }
                Inst::Branch { kind, rs1, rs2, target } => {
                    let a = r[rs1 as usize];
                    let b = r[rs2 as usize];
                    let taken = match kind {
                        BrKind::Eq => a == b,
                        BrKind::Ne => a != b,
                        BrKind::Lt => (a as i64) < (b as i64),
                        BrKind::Ge => (a as i64) >= (b as i64),
                        BrKind::Ltu => a < b,
                        BrKind::Geu => a >= b,
                    };
                    if taken {
                        idx = target as usize;
                        continue 'blocks;
                    }
                }
                Inst::Load { op, rd, rs1, imm } => {
                    let v = load!(r[rs1 as usize].wrapping_add(imm as u64), op);
                    if rd != 0 {
                        r[rd as usize] = v;
                    }
                }
                Inst::Store { op, rs1, rs2, imm } => {
                    store!(r[rs1 as usize].wrapping_add(imm as u64), op, r[rs2 as usize]);
                }
                Inst::ImmAlu { op, rd, rs1, imm } => {
                    if rd != 0 {
                        let a = r[rs1 as usize];
                        let v = match op {
                            ImmAluOp::Addi => a.wrapping_add(imm as u64),
                            ImmAluOp::Slti => ((a as i64) < imm) as u64,
                            ImmAluOp::Sltiu => (a < imm as u64) as u64,
                            ImmAluOp::Xori => a ^ imm as u64,
                            ImmAluOp::Ori => a | imm as u64,
                            ImmAluOp::Andi => a & imm as u64,
                            ImmAluOp::Addiw => sext32_(a.wrapping_add(imm as u64)),
                        };
                        r[rd as usize] = v;
                    }
                }
                Inst::ShiftImm { kind, rd, rs1, shamt, word } => {
                    if rd != 0 {
                        let a = r[rs1 as usize];
                        let v = if word {
                            let s = shamt & 31;
                            match kind {
                                ShiftKind::Sll => sext32_(a << s),
                                ShiftKind::Srl => sext32_(((a as u32) >> s) as u64),
                                ShiftKind::Sra => sext32_(((a as i32) >> s) as u64),
                                ShiftKind::Ror => sext32_((a as u32).rotate_right(s as u32) as u64),
                            }
                        } else {
                            match kind {
                                ShiftKind::Sll => a << shamt,
                                ShiftKind::Srl => a >> shamt,
                                ShiftKind::Sra => ((a as i64) >> shamt) as u64,
                                ShiftKind::Ror => a.rotate_right(shamt as u32),
                            }
                        };
                        r[rd as usize] = v;
                    }
                }
                Inst::RegAlu { op, rd, rs1, rs2 } => {
                    if rd != 0 {
                        let a = r[rs1 as usize];
                        let b = r[rs2 as usize];
                        let v = reg_alu(op, a, b);
                        r[rd as usize] = v;
                    }
                }
                Inst::UnAlu { op, rd, rs1 } => {
                    if rd != 0 {
                        let a = r[rs1 as usize];
                        let v = match op {
                            UnAluOp::Clz => a.leading_zeros() as u64,
                            UnAluOp::Ctz => a.trailing_zeros() as u64,
                            UnAluOp::Cpop => a.count_ones() as u64,
                            UnAluOp::Sextb => a as i8 as i64 as u64,
                            UnAluOp::Sexth => a as i16 as i64 as u64,
                            UnAluOp::Zexth => a & 0xFFFF,
                            UnAluOp::Orcb => {
                                let mut v = 0u64;
                                for i in 0..8 {
                                    if (a >> (i * 8)) & 0xFF != 0 {
                                        v |= 0xFFu64 << (i * 8);
                                    }
                                }
                                v
                            }
                            UnAluOp::Rev8 => a.swap_bytes(),
                            UnAluOp::Clzw => (a as u32).leading_zeros() as u64,
                            UnAluOp::Ctzw => (a as u32).trailing_zeros() as u64,
                            UnAluOp::Cpopw => (a as u32).count_ones() as u64,
                        };
                        r[rd as usize] = v;
                    }
                }
                Inst::BitImm { op, rd, rs1, shamt } => {
                    if rd != 0 {
                        let a = r[rs1 as usize];
                        let mask = 1u64 << (shamt & 63);
                        let v = match op {
                            BitImmOp::Set => a | mask,
                            BitImmOp::Clr => a & !mask,
                            BitImmOp::Inv => a ^ mask,
                            BitImmOp::Ext => (a >> (shamt & 63)) & 1,
                        };
                        r[rd as usize] = v;
                    }
                }
                Inst::Slliuw { rd, rs1, shamt } => {
                    if rd != 0 {
                        let a = r[rs1 as usize] as u32 as u64;
                        r[rd as usize] = a << (shamt & 63);
                    }
                }
                // FENCE: ordering is meaningless on this single-hart,
                // in-memory machine — execute as a no-op.
                Inst::Fence => {}
                Inst::Ecall => {
                    // CHARGE-BEFORE-BOUNDS (documented decision): the hash
                    // cost is charged before the src/dst bounds checks, so
                    // the abort CLASS of an out-of-bounds ecall depends on
                    // the remaining budget — a large budget pays the charge
                    // and then traps; a budget below the hash cost aborts
                    // OutOfGas WITHOUT the charge. Both outcomes are pure
                    // functions of (module, input, budget); callers cannot
                    // probe memory through the class difference because
                    // the bounds check traps before any byte is read.
                    let kind = match HashKind::from_a7(r[17]) {
                        Some(k) => k,
                        None => trap!("calc ecall: unknown builtin"),
                    };
                    let cost = kind.cycles(r[11]);
                    if cycles < cost {
                        out_of_gas!();
                    }
                    cycles -= cost;
                    let src = r[10];
                    let len = r[11];
                    let dst = r[12];
                    let dlen = kind.digest_len();
                    let src_end = match src.checked_add(len) {
                        Some(e) => e,
                        None => trap!("calc ecall: input address overflow"),
                    };
                    let dst_end = match dst.checked_add(dlen) {
                        Some(e) => e,
                        None => trap!("calc ecall: output address overflow"),
                    };
                    if src_end > lay.mem_len || dst_end > lay.mem_len {
                        trap!("calc ecall: out of memory bounds");
                    }
                    if dst < lay.data_base {
                        trap!("calc ecall: digest into read-only region");
                    }
                    if dst < lay.stack_bottom && dst_end > lay.guard_base {
                        trap!("calc ecall: digest into the stack-overflow guard band");
                    }
                    // Digest lengths are 20/32; a stack buffer avoids a heap
                    // allocation per ecall.
                    let mut buf = [0u8; 32];
                    let digest = &mut buf[..dlen as usize];
                    run_hash(kind, &mem[src as usize..src_end as usize], digest);
                    mem[dst as usize..dst_end as usize].copy_from_slice(digest);
                    r[10] = 0;
                }
            }
            idx += 1;
        }
        // Fell through to the next block (normal path): charge it at entry.
        // Running off the very end of the code stream without control flow
        // is a trap — returning must go through the sentinel.
        if idx >= insts.len() {
            trap!("calc execution ran past the end of code");
        }
    }
}

/// Test-visible wrapper over the (private) ALU semantics.
#[doc(hidden)]
pub fn reg_alu_test(op: RegAluOp, a: u64, b: u64) -> u64 {
    reg_alu(op, a, b)
}

#[inline(always)]
fn reg_alu(op: RegAluOp, a: u64, b: u64) -> u64 {
    match op {
        RegAluOp::Add => a.wrapping_add(b),
        RegAluOp::Sub => a.wrapping_sub(b),
        RegAluOp::Sll => a << (b & 63),
        RegAluOp::Slt => ((a as i64) < (b as i64)) as u64,
        RegAluOp::Sltu => (a < b) as u64,
        RegAluOp::Xor => a ^ b,
        RegAluOp::Srl => a >> (b & 63),
        RegAluOp::Sra => ((a as i64) >> (b & 63)) as u64,
        RegAluOp::Or => a | b,
        RegAluOp::And => a & b,
        RegAluOp::Mul => a.wrapping_mul(b),
        RegAluOp::Mulh => (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u128 as u64,
        RegAluOp::Mulhsu => {
            // signed(a) * unsigned(b): widen both to i128 (b zero-extends)
            let prod = ((a as i64 as i128) * (b as i128)) >> 64;
            prod as u64
        }
        RegAluOp::Mulhu => (((a as u128) * (b as u128)) >> 64) as u64,
        RegAluOp::Div => {
            if b == 0 {
                u64::MAX
            } else if a == i64::MIN as u64 && b == u64::MAX {
                a
            } else {
                ((a as i64) / (b as i64)) as u64
            }
        }
        RegAluOp::Divu => {
            if b == 0 {
                u64::MAX
            } else {
                a / b
            }
        }
        RegAluOp::Rem => {
            if b == 0 {
                a
            } else if a == i64::MIN as u64 && b == u64::MAX {
                0
            } else {
                ((a as i64) % (b as i64)) as u64
            }
        }
        RegAluOp::Remu => {
            if b == 0 {
                a
            } else {
                a % b
            }
        }
        RegAluOp::Addw => sext32_(a.wrapping_add(b)),
        RegAluOp::Subw => sext32_(a.wrapping_sub(b)),
        RegAluOp::Sllw => sext32_(a << (b & 31)),
        RegAluOp::Srlw => sext32_(((a as u32) >> (b & 31)) as u64),
        RegAluOp::Sraw => sext32_(((a as i32) >> (b & 31)) as u64),
        RegAluOp::Mulw => sext32_(a.wrapping_mul(b)),
        // W-form div by zero yields 32-bit -1 (0xFFFFFFFF), which sign
        // extends to all-ones in the 64-bit register.
        RegAluOp::Divw => {
            if (b as u32) == 0 {
                u64::MAX
            } else {
                // wrapping: MIN32 / -1 overflows i32 division (Rust panics);
                // RISC-V defines the quotient as MIN32.
                sext32_((a as u32 as i32).wrapping_div(b as u32 as i32) as u64)
            }
        }
        RegAluOp::Divuw => {
            if (b as u32) == 0 {
                u64::MAX
            } else {
                sext32_(((a as u32) / (b as u32)) as u64)
            }
        }
        RegAluOp::Remw => {
            if (b as u32) == 0 {
                sext32_(a)
            } else {
                // wrapping: MIN32 % -1 overflows i32 remainder (Rust panics);
                // RISC-V defines the remainder as 0.
                sext32_((a as u32 as i32).wrapping_rem(b as u32 as i32) as u64)
            }
        }
        RegAluOp::Remuw => {
            if (b as u32) == 0 {
                sext32_(a)
            } else {
                sext32_(((a as u32) % (b as u32)) as u64)
            }
        }
        // ---- Zbb / Zbc ----
        RegAluOp::Adduw => (a as u32 as u64).wrapping_add(b),
        RegAluOp::Sh1add => (a << 1).wrapping_add(b),
        RegAluOp::Sh2add => (a << 2).wrapping_add(b),
        RegAluOp::Sh3add => (a << 3).wrapping_add(b),
        RegAluOp::Sh1adduw => ((a as u32 as u64) << 1).wrapping_add(b),
        RegAluOp::Sh2adduw => ((a as u32 as u64) << 2).wrapping_add(b),
        RegAluOp::Sh3adduw => ((a as u32 as u64) << 3).wrapping_add(b),
        RegAluOp::Andn => a & !b,
        RegAluOp::Orn => a | !b,
        RegAluOp::Xnor => !(a ^ b),
        RegAluOp::Rol => a.rotate_left((b & 63) as u32),
        RegAluOp::Ror => a.rotate_right((b & 63) as u32),
        RegAluOp::Rolw => sext32_((a as u32).rotate_left((b & 31) as u32) as u64),
        RegAluOp::Rorw => sext32_((a as u32).rotate_right((b & 31) as u32) as u64),
        RegAluOp::Min => (a as i64).min(b as i64) as u64,
        RegAluOp::Minu => a.min(b),
        RegAluOp::Max => (a as i64).max(b as i64) as u64,
        RegAluOp::Maxu => a.max(b),
        RegAluOp::Bset => a | (1u64 << (b & 63)),
        RegAluOp::Bclr => a & !(1u64 << (b & 63)),
        RegAluOp::Binv => a ^ (1u64 << (b & 63)),
        RegAluOp::Bext => (a >> (b & 63)) & 1,
        RegAluOp::Clmul => clmul128(a, b) as u64,
        RegAluOp::Clmulh => (clmul128(a, b) >> 64) as u64,
        RegAluOp::Clmulr => (clmul128(a, b) >> 63) as u64,
    }
}

/// Carryless multiply: full 127-bit product. clmul/clmulh/clmulr select the
/// low 64 bits, the high 64 bits, and bits [63..127) respectively.
#[inline(always)]
fn clmul128(a: u64, b: u64) -> u128 {
    let mut p: u128 = 0;
    for i in 0..64 {
        if (b >> i) & 1 == 1 {
            p ^= (a as u128) << i;
        }
    }
    p
}

#[inline(always)]
fn sext32_(v: u64) -> u64 {
    (v as u32 as i32) as i64 as u64
}

impl CalcModule {
    /// Encoded byte length of instruction `idx` (2 or 4).
    pub fn inst_len(&self, idx: usize) -> u32 {
        let next = self
            .addrs
            .get(idx + 1)
            .copied()
            .unwrap_or_else(|| self.layout.code_len as u32);
        next - self.addrs[idx]
    }
}

#[cfg(test)]
mod zbb_semantics_tests {
    use super::*;

    #[test]
    fn rotate_ops_match_rust_reference() {
        let a = 0x1000_0000_0000_0008u64;
        assert_eq!(reg_alu_test(RegAluOp::Rol, a, 4), a.rotate_left(4));
        assert_eq!(reg_alu_test(RegAluOp::Ror, a, 4), a.rotate_right(4));
        assert_eq!(reg_alu_test(RegAluOp::Rol, a, 0), a);
        assert_eq!(reg_alu_test(RegAluOp::Ror, a, 64), a); // 64 & 63 == 0
        let w = 0xFFFF_FFF0_8000_0001u64;
        assert_eq!(
            reg_alu_test(RegAluOp::Rolw, w, 4),
            ((w as u32).rotate_left(4)) as u32 as i32 as i64 as u64
        );
        assert_eq!(
            reg_alu_test(RegAluOp::Rorw, w, 37),
            ((w as u32).rotate_right(37 & 31)) as u32 as i32 as i64 as u64
        );
    }

    #[test]
    fn logic_and_minmax_ops() {
        let a = 0xF0F0_AAAA_0000_1234u64;
        let b = 0x0FF0_5555_FFFF_DCBAu64;
        assert_eq!(reg_alu_test(RegAluOp::Andn, a, b), a & !b);
        assert_eq!(reg_alu_test(RegAluOp::Orn, a, b), a | !b);
        assert_eq!(reg_alu_test(RegAluOp::Xnor, a, b), !(a ^ b));
        assert_eq!(reg_alu_test(RegAluOp::Min, u64::MAX, 1), u64::MAX); // -1 < 1
        assert_eq!(reg_alu_test(RegAluOp::Minu, u64::MAX, 1), 1);
        assert_eq!(reg_alu_test(RegAluOp::Max, u64::MAX, 1), 1);
        assert_eq!(reg_alu_test(RegAluOp::Maxu, u64::MAX, 1), u64::MAX);
        assert_eq!(reg_alu_test(RegAluOp::Adduw, 0xFFFF_0000_0000_0005, 3), 8);
        assert_eq!(reg_alu_test(RegAluOp::Sh1add, 3, 5), 11);
        assert_eq!(reg_alu_test(RegAluOp::Sh3adduw, 0xFFFF_FFFF, 1), 34_359_738_361);
    }

    #[test]
    fn single_bit_ops() {
        let a = 0x8000_0000_0000_0000u64;
        assert_eq!(reg_alu_test(RegAluOp::Bset, 0, 63), 1u64 << 63);
        assert_eq!(reg_alu_test(RegAluOp::Bclr, a, 63), 0);
        assert_eq!(reg_alu_test(RegAluOp::Binv, a, 63), 0);
        assert_eq!(reg_alu_test(RegAluOp::Bext, a, 63), 1);
        assert_eq!(reg_alu_test(RegAluOp::Bext, a, 0), 0);
        // Shift amounts mask to 63.
        assert_eq!(reg_alu_test(RegAluOp::Bset, 0, 64), 1); // 64 & 63 == 0
    }

    #[test]
    fn clmul_family_matches_reference() {
        // clmul(0b1011, 0b1101) = 0b1111111: (x^3+x+1)(x^3+x^2+1) over GF(2).
        assert_eq!(reg_alu_test(RegAluOp::Clmul, 0b1011, 0b1101), 0b1111111);
        let (a, b) = (0xDEAD_BEEF_CAFE_F00Du64, 0x1234_5678_9ABC_DEF0u64);
        let p = clmul128(a, b);
        assert_eq!(reg_alu_test(RegAluOp::Clmul, a, b), p as u64);
        assert_eq!(reg_alu_test(RegAluOp::Clmulh, a, b), (p >> 64) as u64);
        assert_eq!(reg_alu_test(RegAluOp::Clmulr, a, b), (p >> 63) as u64);
        // Low bit sanity: clmul(x, 1) == x; the product of x and 1 fits in
        // 64 bits, so the high half is 0 while clmulr keeps bit 63.
        assert_eq!(reg_alu_test(RegAluOp::Clmul, a, 1), a);
        assert_eq!(reg_alu_test(RegAluOp::Clmulh, a, 1), 0);
        assert_eq!(reg_alu_test(RegAluOp::Clmulr, a, 1), a >> 63);
        // Commutativity of the carryless product.
        assert_eq!(clmul128(a, b), clmul128(b, a));
    }
}
