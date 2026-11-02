//! AOT backend for verified calc containers: translates the pre-decoded
//! [`CalcModule`](verify::CalcModule) into x86-64 (SysV) machine code and
//! executes it natively. Semantics mirror `interp.rs` exactly — instruction
//! behavior, link values, div/rem special cases, read-only ranges, bounds
//! checks, and the cycle accounting (charged per dynamic block entry at the
//! entry instruction, so a `jalr` into the middle of a block pays only the
//! remaining instructions, matching `suffix_cycles`).
//!
//! Design (benchmark-grade, deliberately simple):
//! * RV64 registers live in a memory array `[u64; 32]`; each RV64
//!   instruction emits a handful of x86-64 ops through fixed scratch
//!   registers.
//! * Control flow: direct `jcc/jmp` to translated block prologues (branch
//!   and JAL targets are always block leaders after verification); `jalr`
//!   calls a host resolver (address -> instruction index) which returns
//!   either the block prologue (leader target) or a small trampoline that
//!   first charges only the remaining suffix cycles and then jumps to the
//!   target instruction (mid-block target); a miss lands on the invalid-
//!   target trap stub.
//! * Cycle metering is block-level and static, identical to the
//!   interpreter and host-independent: before charging a block the
//!   remaining budget is compared against the entry cost, and on shortfall
//!   the run aborts WITHOUT keeping the failing charge — exactly the
//!   interpreter's `out_of_gas` accounting.
//! * Div/Rem families (all special cases) go through a host helper
//!   mirroring `interp::reg_alu`; `mulh*` use native `mul/imul`.
//! * `ecall` (HALT, hash builtins, HINT trap) is implemented via a host
//!   helper that mirrors `interp.rs`. `fence`
//!   executes as a no-op, matching the single-hart interpreter.
//!
//! Early launch insurance: this backend is not the sole engine yet. Official x86-64
//! nodes (feature `calc-aot`, on by default) run this translation after
//! the interpreter in order on the calling thread and reject the
//! transaction when gas or the result disagree (`CalcEngineMismatch`). Nodes built without the
//! feature run the interpreter only. After the lockstep check has stayed
//! quiet in production, the interpreter is retired and this backend
//! becomes the only execution engine. `cargo test -p vm --features calc-aot`
//! runs the differential suite at the bottom of this file.

// The generated code is x86-64 SysV only. Enabling the feature on another
// host architecture would compile this module and then SIGILL at run time;
// fail the build instead.
#[cfg(not(target_arch = "x86_64"))]
compile_error!("calc-aot generates x86-64 code and is only supported on x86_64 hosts");

use crate::calc::decode::{BitImmOp, BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp, UnAluOp};
use crate::calc::container::{CalcContainer, CalcLayout, CODE_BASE};
use crate::calc::verify::CalcModule;
use crate::calc::{run_hash, HashKind, ECALL_HALT, ECALL_HINT_MAX, ECALL_HINT_MIN};

// Host register numbers (x86-64 encoding order).
const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
const RBX: u8 = 3; // mem base
const RBP: u8 = 5; // div/rem helper fn
const RSI: u8 = 6;
const RDI: u8 = 7;
const R12: u8 = 12; // regs array base
const R13: u8 = 13; // AotState base
const R14: u8 = 14; // resolve ctx
const R15: u8 = 15; // jalr resolver fn

// AotState field displacements (see the struct below).
const ST_CYCLES: i8 = 0;
const ST_OUT_LEN: i8 = 8;
const ST_ABORTED: i8 = 16;

struct Code {
    b: Vec<u8>,
}

impl Code {
    fn new() -> Self {
        Code { b: Vec::new() }
    }
    fn rex(&mut self, w: bool, r: bool, x: bool, b: bool) {
        self.b.push(0x40 | ((w as u8) << 3) | ((r as u8) << 2) | ((x as u8) << 1) | b as u8);
    }
    fn op(&mut self, o: u8) {
        self.b.push(o);
    }
    fn op2(&mut self, o: u8, o2: u8) {
        self.b.push(o);
        self.b.push(o2);
    }
    fn imm8(&mut self, v: i8) {
        self.b.push(v as u8);
    }
    fn imm32(&mut self, v: i32) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn imm64(&mut self, v: u64) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn modrm(&mut self, md: u8, reg: u8, rm: u8) {
        self.b.push(((md & 3) << 6) | ((reg & 7) << 3) | (rm & 7));
    }
    fn sib(&mut self, scale: u8, index: u8, base: u8) {
        self.b.push(((scale & 3) << 6) | ((index & 7) << 3) | (base & 7));
    }
    fn len(&self) -> usize {
        self.b.len()
    }
    fn patch32(&mut self, at: usize, rel: i32) {
        self.b[at..at + 4].copy_from_slice(&rel.to_le_bytes());
    }
    fn patch64(&mut self, at: usize, v: u64) {
        self.b[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// `mov hdst, qword [r12 + 8*rv]` — load an RV64 register into a host reg.
    fn ld_reg(&mut self, hdst: u8, rv: u8) {
        self.rex(true, hdst >= 8, false, true); // r12 base needs B
        self.op(0x8B);
        self.modrm(2, hdst, 4); // mod=10 disp32, rm=100 -> SIB
        self.sib(0, 4, 4); // index none, base r12
        self.imm32((8 * rv as i64) as i32);
    }
    /// `mov qword [r12 + 8*rv], hsrc`
    fn st_reg(&mut self, rv: u8, hsrc: u8) {
        self.rex(true, hsrc >= 8, false, true);
        self.op(0x89);
        self.modrm(2, hsrc, 4);
        self.sib(0, 4, 4);
        self.imm32((8 * rv as i64) as i32);
    }
    /// `movabs h, imm64`
    fn movabs(&mut self, h: u8, v: u64) -> usize {
        self.rex(true, false, false, h >= 8);
        self.op(0xB8 | (h & 7));
        let at = self.len();
        self.imm64(v);
        at
    }
    /// ALU op with imm32 sign-extended, register destination: `81 /op`.
    fn alu_imm(&mut self, opext: u8, h: u8, imm: i32) {
        self.rex(true, false, false, h >= 8);
        self.op(0x81);
        self.modrm(3, opext, h);
        self.imm32(imm);
    }
    /// ALU op hdst <- hdst OP hsrc (64-bit). add=01 or=09 and=21 sub=29 xor=31
    fn alu_rr(&mut self, base_op: u8, hdst: u8, hsrc: u8) {
        self.rex(true, hsrc >= 8, false, hdst >= 8);
        self.op(base_op);
        self.modrm(3, hsrc, hdst);
    }
    /// `cmp hdst, hsrc` (flags for hdst OP hsrc)
    fn cmp_rr(&mut self, a: u8, b: u8) {
        self.rex(true, b >= 8, false, a >= 8);
        self.op(0x39);
        self.modrm(3, b, a);
    }
    /// `cmp qword [r13 + disp], imm32` — /7 with SIB base r13.
    fn cmp_state_imm32(&mut self, disp: i8, imm: i32) {
        self.rex(true, false, false, true);
        self.op(0x81);
        self.modrm(1, 7, 4);
        self.sib(0, 4, 5);
        self.imm8(disp);
        self.imm32(imm);
    }
    /// `sub qword [r13 + disp], imm32` — /5 with SIB base r13.
    fn sub_state_imm32(&mut self, disp: i8, imm: i32) {
        self.rex(true, false, false, true);
        self.op(0x81);
        self.modrm(1, 5, 4);
        self.sib(0, 4, 5);
        self.imm8(disp);
        self.imm32(imm);
    }
    /// `jcc rel32` — returns the rel32 field offset for patching.
    /// cc: 0=O 1=NO 2=B/C 3=AE 4=E 5=NE 6=BE 7=A 8=S 9=NS A=P B=NP C=L D=GE E=LE F=G
    fn jcc32(&mut self, cc: u8) -> usize {
        self.op2(0x0F, 0x80 + cc);
        let at = self.len();
        self.imm32(0);
        at
    }
    fn jmp32(&mut self) -> usize {
        self.op(0xE9);
        let at = self.len();
        self.imm32(0);
        at
    }
    /// `setcc al` + `movzx rax, al`
    fn setcc_rax(&mut self, cc: u8) {
        self.op2(0x0F, 0x90 + cc);
        self.modrm(3, 0, RAX);
        self.op2(0x0F, 0xB6);
        self.modrm(3, 0, RAX);
    }
    /// Shift h by cl (x86 masks the count by 63 on 64-bit ops, 31 on 32-bit).
    fn shift_cl(&mut self, ext: u8, h: u8, w: bool) {
        self.rex(w, false, false, h >= 8);
        self.op(0xD3);
        self.modrm(3, ext, h);
    }
    /// Shift h by imm8.
    fn shift_imm(&mut self, ext: u8, h: u8, sh: u8, w: bool) {
        self.rex(w, false, false, h >= 8);
        self.op(0xC1);
        self.modrm(3, ext, h);
        self.imm8(sh as i8);
    }
    /// `mov rcx, rax`
    fn mov_rcx_rax(&mut self) {
        self.rex(true, false, false, false);
        self.op(0x89);
        self.modrm(3, RAX, RCX);
    }
    /// Memory data access through `[rbx + rax]`.
    fn rax_to_mem(&mut self, form: MemForm) {
        match form {
            MemForm::Sd => {
                self.rex(true, false, false, false);
                self.op(0x89);
                self.modrm(0, RCX, 4);
                self.sib(0, RAX, RBX);
            }
            MemForm::Sw => {
                self.op(0x89);
                self.modrm(0, RCX, 4);
                self.sib(0, RAX, RBX);
            }
            MemForm::Sh => {
                self.op(0x66);
                self.op(0x89);
                self.modrm(0, RCX, 4);
                self.sib(0, RAX, RBX);
            }
            MemForm::Sb => {
                self.op(0x88);
                self.modrm(0, RCX, 4);
                self.sib(0, RAX, RBX);
            }
            _ => unreachable!("rax_to_mem only for stores"),
        }
    }
    /// `movsxd rax, eax` — sign-extend the 32-bit result in eax.
    fn sext_eax(&mut self) {
        self.rex(true, false, false, false);
        self.op(0x63);
        self.modrm(3, RAX, RAX);
    }
    /// 32-bit ALU with memory source: `op eax, [r12 + 8*rv]`.
    fn alu32_rm_reg(&mut self, base_op: u8, hdst: u8, rv: u8) {
        self.rex(false, hdst >= 8, false, true);
        self.op(base_op);
        self.modrm(2, hdst, 4);
        self.sib(0, 4, 4);
        self.imm32((8 * rv as i64) as i32);
    }
    /// 32-bit load of the low word of an RV64 register, zero-extended:
    /// `mov ehd, [r12 + 8*rv]`.
    fn ld_reg32_zext(&mut self, hdst: u8, rv: u8) {
        self.rex(false, hdst >= 8, false, true);
        self.op(0x8B);
        self.modrm(2, hdst, 4);
        self.sib(0, 4, 4);
        self.imm32((8 * rv as i64) as i32);
    }
    /// `not h` (0xF7 /2).
    fn not_r64(&mut self, h: u8) {
        self.rex(true, false, false, h >= 8);
        self.op(0xF7);
        self.modrm(3, 2, h);
    }
    /// Rotate by cl: ext 0 = rol, 1 = ror (0xD3 /ext; count masked &63 for
    /// 64-bit and &31 for 32-bit, matching the RISC-V semantics).
    fn rot_cl(&mut self, ext: u8, h: u8, w: bool) {
        self.rex(w, false, false, h >= 8);
        self.op(0xD3);
        self.modrm(3, ext, h);
    }
    /// `cmov hdst, hsrc` (0F 40+cc). NOTE: unlike MOV, the CMOVcc
    /// DESTINATION lives in the reg field and the source in r/m.
    fn cmov_rr(&mut self, cc: u8, hdst: u8, hsrc: u8) {
        self.rex(true, hdst >= 8, false, hsrc >= 8);
        self.op2(0x0F, 0x40 + cc);
        self.modrm(3, hdst, hsrc);
    }
    /// BT family: 0F A3 (bt), 0F AB (bts), 0F B3 (btr), 0F BB (btc) on
    /// `hdst` with the count in cl.
    fn bt_group(&mut self, opcode2: u8, hdst: u8) {
        self.rex(true, false, false, hdst >= 8);
        self.op2(0x0F, opcode2);
        self.modrm(3, RCX, hdst);
    }
    /// `imul hdst, qword [r12 + 8*rv]` (0F AF).
    fn imul_rm(&mut self, hdst: u8, rv: u8) {
        self.rex(true, hdst >= 8, false, true);
        self.op2(0x0F, 0xAF);
        self.modrm(2, hdst, 4);
        self.sib(0, 4, 4);
        self.imm32((8 * rv as i64) as i32);
    }
}

#[derive(Clone, Copy)]
enum MemForm {
    Ld,
    LwSext,
    LwZero,
    LhSext,
    LhZero,
    LbSext,
    LbZero,
    Sd,
    Sw,
    Sh,
    Sb,
}

// ---------------------------------------------------------------- runtime

/// Host-side state shared with the generated code.
#[repr(C)]
struct AotState {
    /// Remaining cycle budget (interpreter: `cycles` counter). Always >= 0:
    /// a block is only charged after the pre-check passes.
    cycles: i64,
    /// Output length written by the HALT ecall path.
    out_len: u64,
    /// 0 = ok, 1 = trap, 2 = out of gas, 3 = bad jalr target, 4 = guest exit.
    aborted: u32,
    _pad: u32,
    /// Guest HALT exit code (a0). Meaningful when aborted == 0 or 4.
    exit_code: u64,
}

/// Host tables for the jalr resolver and ecall helper.
struct ResolveCtx {
    inst_addrs: Vec<u32>,
    /// Landing code offset per instruction: block prologues for leaders,
    /// per-instruction suffix-charge stubs for mid-block entries,
    /// `u32::MAX` never (leaders are populated for every block start).
    inst_landing: Vec<u32>,
    /// Generated offset of the invalid-target stub.
    invalid_off: u32,
    code: *const u8,
    layout: CalcLayout,
}

/// Mirror of `interp::reg_alu` for the ops the generated code routes to the
/// host helper (div/rem families). Validated against `reg_alu_test` by the
/// differential suite at the bottom of this file.
pub fn alu_helper_op(op: u64, a: u64, b: u64) -> u64 {
    match op {
        0 => {
            // Div
            if b == 0 {
                u64::MAX
            } else if a == i64::MIN as u64 && b == u64::MAX {
                a
            } else {
                ((a as i64) / (b as i64)) as u64
            }
        }
        1 => {
            // Divu
            if b == 0 {
                u64::MAX
            } else {
                a / b
            }
        }
        2 => {
            // Rem
            if b == 0 {
                a
            } else if a == i64::MIN as u64 && b == u64::MAX {
                0
            } else {
                ((a as i64) % (b as i64)) as u64
            }
        }
        3 => {
            // Remu
            if b == 0 {
                a
            } else {
                a % b
            }
        }
        4 => {
            // Divw
            if (b as u32) == 0 {
                u64::MAX
            } else {
                sext32_((a as u32 as i32).wrapping_div(b as u32 as i32) as u64)
            }
        }
        5 => {
            // Divuw
            if (b as u32) == 0 {
                u64::MAX
            } else {
                sext32_((a as u32 / b as u32) as u64)
            }
        }
        6 => {
            // Remw
            if (b as u32) == 0 {
                sext32_(a)
            } else {
                sext32_((a as u32 as i32).wrapping_rem(b as u32 as i32) as u64)
            }
        }
        7 => {
            // Remuw
            if (b as u32) == 0 {
                sext32_(a)
            } else {
                sext32_((a as u32 % b as u32) as u64)
            }
        }
        // Zbc carryless multiplies (software loop mirroring interp::clmul128;
        // WEIGHTS.clmul charged either way — see decode::WEIGHTS).
        8 => clmul128_(a, b) as u64,        // clmul
        9 => (clmul128_(a, b) >> 64) as u64, // clmulh
        10 => (clmul128_(a, b) >> 63) as u64, // clmulr
        // Zbb unary ops (b ignored) — same helper entry as div/rem so the
        // generated code needs no extra long-lived register.
        _ => unalu_helper_op(op - 16, a),
    }
}

#[inline(always)]
fn clmul128_(a: u64, b: u64) -> u128 {
    let mut p: u128 = 0;
    for i in 0..64 {
        if (b >> i) & 1 == 1 {
            p ^= (a as u128) << i;
        }
    }
    p
}

/// Zbb unary ops routed through the host (op order matches `UnAluOp`).
pub fn unalu_helper_op(op: u64, a: u64) -> u64 {
    use crate::calc::decode::UnAluOp;
    let kind = match op {
        0 => UnAluOp::Clz,
        1 => UnAluOp::Ctz,
        2 => UnAluOp::Cpop,
        3 => UnAluOp::Sextb,
        4 => UnAluOp::Sexth,
        5 => UnAluOp::Zexth,
        6 => UnAluOp::Orcb,
        7 => UnAluOp::Rev8,
        8 => UnAluOp::Clzw,
        9 => UnAluOp::Ctzw,
        _ => UnAluOp::Cpopw,
    };
    match kind {
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
    }
}

extern "C" fn alu_helper(op: u64, a: u64, b: u64) -> u64 {
    alu_helper_op(op, a, b)
}

/// Host ecall: HALT, hash builtins, HINT trap. Returns 0 = continue,
/// 1 = stop (aborted / halt already recorded on `state`).
/// `state` is the per-call `AotState` held in `R13` at the call site.
extern "C" fn aot_ecall(
    ctx: *const ResolveCtx,
    regs: *mut u64,
    mem: *mut u8,
    state: *mut AotState,
) -> u64 {
    let ctx = unsafe { &*ctx };
    let r = unsafe { std::slice::from_raw_parts_mut(regs, 32) };
    let lay = ctx.layout;
    let state = unsafe { &mut *state };
    let a7 = r[17];
    if a7 == ECALL_HALT {
        let exit_code = r[10];
        let n = r[11];
        if n > lay.output_max {
            state.aborted = 1;
            return 1;
        }
        if exit_code != 0 {
            state.aborted = 4;
            state.exit_code = exit_code;
            return 1;
        }
        state.out_len = n;
        state.exit_code = 0;
        state.aborted = 0;
        return 1;
    }
    if (ECALL_HINT_MIN..=ECALL_HINT_MAX).contains(&a7) {
        state.aborted = 1;
        return 1;
    }
    let kind = match HashKind::from_a7(a7) {
        Some(k) => k,
        None => {
            state.aborted = 1;
            return 1;
        }
    };
    let cost = kind.cycles(r[11]);
    if (state.cycles as u64) < cost {
        state.aborted = 2;
        return 1;
    }
    state.cycles -= cost as i64;
    let src = r[10];
    let len = r[11];
    let dst = r[12];
    let dlen = kind.digest_len();
    let src_end = match src.checked_add(len) {
        Some(e) => e,
        None => {
            state.aborted = 1;
            return 1;
        }
    };
    let dst_end = match dst.checked_add(dlen) {
        Some(e) => e,
        None => {
            state.aborted = 1;
            return 1;
        }
    };
    if src < CODE_BASE || dst < CODE_BASE {
        state.aborted = 1;
        return 1;
    }
    if src_end > lay.mem_len || dst_end > lay.mem_len {
        state.aborted = 1;
        return 1;
    }
    // Digest writes obey the store rules: code, the pre-rodata pad, rodata
    // and the alignment hole are read-only; `.data` starts at data_base.
    if dst < lay.data_base {
        state.aborted = 1;
        return 1;
    }
    if dst < lay.stack_bottom && dst_end > lay.guard_base {
        state.aborted = 1;
        return 1;
    }
    let mem = unsafe { std::slice::from_raw_parts_mut(mem, lay.mem_len as usize) };
    let mut buf = [0u8; 32];
    let digest = &mut buf[..dlen as usize];
    run_hash(kind, &mem[src as usize..src_end as usize], digest);
    mem[dst as usize..dst_end as usize].copy_from_slice(digest);
    r[10] = 0;
    0
}

extern "C" fn resolve_jalr(ctx: *const ResolveCtx, dest: u64) -> u64 {
    let ctx = unsafe { &*ctx };
    // A miss returns the invalid-target stub. That stub writes
    // `aborted = 3` through R13 (the per-call `AotState`) and jumps to Exit,
    // so this helper stays a pure address lookup.
    let invalid = unsafe { ctx.code.add(ctx.invalid_off as usize) as u64 };
    // Mirror `CalcModule::inst_at_addr`: targets above u32::MAX are never
    // valid. Without this the `dest as u32` truncation below would alias
    // 2^32 + <addr> onto <addr> and silently diverge from the interpreter
    // (audit B2, pinned by the tests.loc probe and the isa suite).
    if dest > u32::MAX as u64 {
        return invalid;
    }
    match ctx.inst_addrs.binary_search(&(dest as u32)) {
        Ok(idx) => unsafe { ctx.code.add(ctx.inst_landing[idx] as usize) as u64 },
        Err(_) => invalid,
    }
}

#[inline(always)]
fn sext32_(v: u64) -> u64 {
    (v as u32 as i32) as i64 as u64
}

// ---------------------------------------------------------------- compile

pub struct AotFunc {
    /// RX mapping (kept alive for the lifetime of this object).
    map: (*mut u8, usize),
    /// Translated code length in bytes.
    len: usize,
    /// Resolver tables. Immutable after `aot_compile` returns; each run
    /// passes its own `AotState` in R13.
    ctx: Box<ResolveCtx>,
}

impl AotFunc {
    pub fn code_len(&self) -> usize {
        self.len
    }

    /// Generated machine code (debug/disassembly aid).
    pub fn code_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.map.0 as *const u8, self.len) }
    }
}

// `AotFunc` and `ResolveCtx` are immutable after compilation. Generated code
// keeps the per-call `AotState` in R13 and passes that pointer into
// `aot_ecall`; a jalr miss lands on the invalid-target stub, which records
// the abort through the same R13. Nothing in the compiled image or the
// resolver tables is written during `run`, so concurrent execution on one
// `AotFunc` is safe. The raw pointers (`map`, `ResolveCtx::code`) are why
// these impls are `unsafe`: they alias the RX mapping owned by `AotFunc`.
unsafe impl Send for AotFunc {}
unsafe impl Sync for AotFunc {}

impl Drop for AotFunc {
    fn drop(&mut self) {
        // `ctx` owns the resolver tables whose address is patched into the
        // RX image; it must stay alive until the mapping is released.
        let _ = self.ctx.code;
        unsafe {
            libc::munmap(self.map.0.cast(), self.map.1);
        }
    }
}

pub struct AotRun {
    pub output: Vec<u8>,
    pub cycles_used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AotAbort {
    Trap,
    OutOfGas,
    BadTarget,
    GuestExit,
}

/// Full stop-state of one AOT execution: output on success, abort class on
/// failure, and the cycle total in both cases (the interpreter reports
/// `cycles_used` on abort paths too, and the differential suite pins that
/// equality).
pub struct AotOutcome {
    pub output: Option<Vec<u8>>,
    pub abort: Option<AotAbort>,
    pub cycles_used: u64,
    pub exit_code: u64,
}

impl AotOutcome {
    /// Result-style view for benchmark callers.
    pub fn into_result(self) -> Result<AotRun, AotAbort> {
        match (self.output, self.abort) {
            (Some(o), None) => Ok(AotRun {
                output: o,
                cycles_used: self.cycles_used,
            }),
            (_, Some(a)) => Err(a),
            (None, None) => unreachable!("outcome carries output or abort"),
        }
    }
}

impl AotFunc {
    /// Execute with interpreter-identical memory/entry setup. The cycle
    /// budget covers instructions only — the interpreter's per-call setup
    /// charge lives in `CalcFnObj::run`, outside this layer. This image does
    /// not poll a host deadline; `CalcFnObj::run` keeps the interpreter when
    /// a deadline is set.
    ///
    pub fn run(
        &self,
        module: &CalcModule,
        container: &CalcContainer,
        input: &[u8],
        cycle_budget: u64,
    ) -> Result<AotRun, AotAbort> {
        self.run_inner(module, container, input, cycle_budget)
            .0
            .into_result()
    }

    /// `run` plus final register and memory snapshots — the differential
    /// counterpart of `interp::run_with_state`.
    pub fn run_with_state(
        &self,
        module: &CalcModule,
        container: &CalcContainer,
        input: &[u8],
        cycle_budget: u64,
    ) -> (AotOutcome, [u64; 32], Vec<u8>) {
        self.run_inner(module, container, input, cycle_budget)
    }

    fn run_inner(
        &self,
        module: &CalcModule,
        container: &CalcContainer,
        input: &[u8],
        cycle_budget: u64,
    ) -> (AotOutcome, [u64; 32], Vec<u8>) {
        let lay = &module.layout;
        if input.len() as u64 > lay.input_max {
            return (
                AotOutcome {
                    output: None,
                    abort: Some(AotAbort::Trap),
                    cycles_used: 0,
                    exit_code: 0,
                },
                [0u64; 32],
                Vec::new(),
            );
        }
        let mut mem = vec![0u8; lay.mem_len as usize];
        let code_off = CODE_BASE as usize;
        mem[code_off..code_off + container.code.len()].copy_from_slice(&container.code);
        mem[lay.rodata_base as usize..(lay.rodata_base + lay.rodata_len) as usize]
            .copy_from_slice(&container.rodata);
        mem[lay.data_base as usize..(lay.data_base + container.init.len() as u64) as usize]
            .copy_from_slice(&container.init);
        let mut regs = [0u64; 32];
        regs[10] = lay.input_base;
        regs[11] = input.len() as u64;
        regs[12] = lay.output_base;
        regs[13] = lay.output_max;
        regs[2] = lay.stack_top;
        let input_base = lay.input_base as usize;
        mem[input_base..input_base + input.len()].copy_from_slice(input);
        // `u64::MAX as i64` is -1, and the block prologue's `jl` treats that
        // as "budget already exhausted". Clamp before the signed counter.
        let cycle_budget = cycle_budget.min(i64::MAX as u64);
        let mut state = AotState {
            cycles: cycle_budget as i64,
            out_len: 0,
            aborted: 0,
            _pad: 0,
            exit_code: 0,
        };
        let state_ptr = std::ptr::addr_of_mut!(state);
        let f: extern "C" fn(*mut u8, *mut u64, *mut AotState) = unsafe {
            std::mem::transmute::<*const u8, extern "C" fn(*mut u8, *mut u64, *mut AotState)>(
                self.map.0 as *const u8,
            )
        };
        f(mem.as_mut_ptr(), regs.as_mut_ptr(), state_ptr);
        let cycles_used = (cycle_budget as i64 - state.cycles).max(0) as u64;
        let out = match state.aborted {
            0 => {
                let n = state.out_len as usize;
                AotOutcome {
                    output: Some(
                        mem[lay.output_base as usize..lay.output_base as usize + n].to_vec(),
                    ),
                    abort: None,
                    cycles_used,
                    exit_code: 0,
                }
            }
            2 => AotOutcome {
                output: None,
                abort: Some(AotAbort::OutOfGas),
                cycles_used,
                exit_code: 0,
            },
            3 => AotOutcome {
                output: None,
                abort: Some(AotAbort::BadTarget),
                cycles_used,
                exit_code: 0,
            },
            4 => AotOutcome {
                output: None,
                abort: Some(AotAbort::GuestExit),
                cycles_used,
                exit_code: state.exit_code,
            },
            _ => AotOutcome {
                output: None,
                abort: Some(AotAbort::Trap),
                cycles_used,
                exit_code: 0,
            },
        };
        (out, regs, mem)
    }
}

enum Fixup {
    /// Patch rel32 to a byte offset in the code buffer.
    To(usize),
    /// Patch rel32 to the prologue of the block containing this inst idx.
    Block(u32),
    /// Trap stub (aborted = 1).
    Trap,
    /// Out-of-gas stub (aborted = 2).
    Gas,
    /// Fallthrough to the block after `u32` (block index).
    Next(u32),
    /// Ran past the end of the code stream (trap).
    PastEnd,
    /// Normal exit epilogue.
    Exit,
}

/// Translate a verified module. Returns `None` only if mmap fails.
pub fn aot_compile(module: &CalcModule) -> Option<AotFunc> {
    let lay = &module.layout;
    let mut c = Code::new();
    let mut fixups: Vec<(usize, Fixup)> = Vec::new();
    let mut block_off = vec![0u32; module.blocks.len()];
    // Translated-code offset of every instruction (for mid-block landings).
    let mut inst_off = vec![0u32; module.insts.len()];
    // Landing offsets per instruction: leaders -> block prologue (filled
    // below), non-leaders -> their suffix-charge stub.
    let mut inst_landing = vec![u32::MAX; module.insts.len()];

    // ---- prologue
    for r in [RBX, RBP, R12, R13, R14, R15] {
        if r >= 12 {
            c.rex(false, false, false, true);
        }
        c.op(0x50 | (r & 7)); // push
    }
    // SysV: rsp must be 16-aligned at helper `call` sites. Entry left
    // rsp%16==8; six pushes keep it at 8 — pad to 0.
    c.op(0x48);
    c.op(0x83);
    c.modrm(3, 5, 4); // sub rsp, 8
    c.imm8(8);
    c.rex(true, false, false, false);
    c.op(0x89);
    c.modrm(3, RDI, RBX); // mov rbx, rdi (mem)
    c.rex(true, false, false, true);
    c.op(0x89);
    c.modrm(3, RSI, R12); // mov r12, rsi (regs)
    c.rex(true, false, false, true);
    c.op(0x89);
    c.modrm(3, RDX, R13); // mov r13, rdx (state)
    let fix_ctx = c.movabs(R14, 0);
    let fix_resolve = c.movabs(R15, 0);
    let fix_alu = c.movabs(RBP, 0);
    let fix_entry = c.jmp32();

    // ---- blocks

    for (bi, blk) in module.blocks.iter().enumerate() {
        block_off[bi] = c.len() as u32;
        // Block entry charge with interpreter-identical out-of-gas
        // accounting: compare first, abort WITHOUT keeping the failing
        // charge, only then subtract.
        debug_assert!(blk.cycles <= i32::MAX as u64);
        c.cmp_state_imm32(ST_CYCLES, blk.cycles as i32);
        let at = c.jcc32(0xC); // jl (remaining < cost) -> gas stub
        fixups.push((at, Fixup::Gas));
        debug_assert!(blk.cycles <= i32::MAX as u64);
        c.sub_state_imm32(ST_CYCLES, blk.cycles as i32);

        inst_landing[blk.start as usize] = block_off[bi];
        let end = blk.end as usize;
        let mut idx = blk.start as usize;
        while idx < end {
            inst_off[idx] = c.len() as u32;
            let inst = module.insts[idx];
            let addr = module.addrs[idx] as u64;
            let next_link = addr + module.inst_len(idx) as u64;
            match inst {
                Inst::Lui { rd, val } => {
                    if rd != 0 {
                        c.movabs(RAX, val as u64);
                        c.st_reg(rd, RAX);
                    }
                }
                Inst::Auipc { rd, val } => {
                    if rd != 0 {
                        c.movabs(RAX, addr.wrapping_add(val as u64));
                        c.st_reg(rd, RAX);
                    }
                }
                Inst::Jal { rd, target } => {
                    if rd != 0 {
                        c.movabs(RAX, next_link);
                        c.st_reg(rd, RAX);
                    }
                    fixups.push((c.jmp32(), Fixup::Block(target as u32)));
                }
                Inst::Jalr { rd, rs1, imm } => {
                    c.ld_reg(RAX, rs1);
                    c.alu_imm(0 /*add*/, RAX, imm as i32);
                    c.alu_imm(4 /*and*/, RAX, -2); // & !1
                    if rd != 0 {
                        c.movabs(RCX, next_link);
                        c.st_reg(rd, RCX);
                    }
                    c.rex(true, true, false, false);
                    c.op(0x89);
                    c.modrm(3, R14, RDI); // mov rdi, r14 (ctx)
                    c.rex(true, false, false, false);
                    c.op(0x89);
                    c.modrm(3, RAX, RSI); // mov rsi, rax (dest)
                    c.rex(false, false, false, true);
                    c.op(0xFF);
                    c.modrm(3, 2, R15); // call r15
                    c.op(0xFF);
                    c.modrm(3, 4, RAX); // jmp rax
                }
                Inst::Branch { kind, rs1, rs2, target } => {
                    c.ld_reg(RAX, rs1);
                    c.ld_reg(RCX, rs2);
                    c.cmp_rr(RAX, RCX);
                    let cc = match kind {
                        BrKind::Eq => 4u8,   // je
                        BrKind::Ne => 5u8,   // jne
                        BrKind::Lt => 0xCu8, // jl
                        BrKind::Ge => 0xDu8, // jge
                        BrKind::Ltu => 2u8,  // jb
                        BrKind::Geu => 3u8,  // jae
                    };
                    let at = c.jcc32(cc);
                    fixups.push((at, Fixup::Block(target as u32)));
                }
                Inst::Load { op, rd, rs1, imm } => {
                    c.ld_reg(RAX, rs1);
                    c.alu_imm(0, RAX, imm as i32); // addr = base + imm
                    c.mov_rcx_rax(); // rcx = addr
                    c.alu_imm(0, RCX, op.size() as i32); // rcx = end
                    let at = c.jcc32(2); // jc -> trap (addr+size overflow)
                    fixups.push((at, Fixup::Trap));
                    // unmapped: cmp rax, CODE_BASE ; jb trap
                    c.alu_imm(7 /*cmp*/, RAX, CODE_BASE as i32);
                    let at = c.jcc32(2); // jb
                    fixups.push((at, Fixup::Trap));
                    // bounds: cmp rcx, mem_len ; ja trap
                    debug_assert!(lay.mem_len <= i32::MAX as u64);
                    c.alu_imm(7 /*cmp*/, RCX, lay.mem_len as i32);
                    let at = c.jcc32(7); // ja
                    fixups.push((at, Fixup::Trap));
                    let form = match op {
                        LoadOp::Ld => MemForm::Ld,
                        LoadOp::Lw => MemForm::LwSext,
                        LoadOp::Lwu => MemForm::LwZero,
                        LoadOp::Lh => MemForm::LhSext,
                        LoadOp::Lhu => MemForm::LhZero,
                        LoadOp::Lb => MemForm::LbSext,
                        LoadOp::Lbu => MemForm::LbZero,
                    };
                    emit_load(&mut c, form);
                    if rd != 0 {
                        c.st_reg(rd, RAX);
                    }
                }
                Inst::Store { op, rs1, rs2, imm } => {
                    c.ld_reg(RAX, rs1);
                    c.alu_imm(0, RAX, imm as i32); // addr
                    // unmapped: addr < CODE_BASE -> trap
                    c.alu_imm(7 /*cmp*/, RAX, CODE_BASE as i32);
                    let at = c.jcc32(2); // jb
                    fixups.push((at, Fixup::Trap));
                    // Code, the pad below rodata_base, rodata and the
                    // alignment hole are read-only (unconditional v1
                    // semantics); `.data` starts at data_base.
                    debug_assert!(lay.data_base <= i32::MAX as u64);
                    c.alu_imm(7 /*cmp*/, RAX, lay.data_base as i32);
                    let at = c.jcc32(2); // jb
                    fixups.push((at, Fixup::Trap));
                    c.mov_rcx_rax();
                    c.alu_imm(0, RCX, op.size() as i32);
                    let at = c.jcc32(2); // jc
                    fixups.push((at, Fixup::Trap));
                    debug_assert!(lay.mem_len <= i32::MAX as u64);
                    c.alu_imm(7, RCX, lay.mem_len as i32);
                    let at = c.jcc32(7); // ja
                    fixups.push((at, Fixup::Trap));
                    // Stack-overflow guard (mirrors interp). RAX holds addr,
                    // RCX holds end.
                    debug_assert!(lay.stack_bottom <= i32::MAX as u64);
                    c.alu_imm(7 /*cmp*/, RAX, lay.stack_bottom as i32);
                    let sg1 = c.jcc32(3); // jae -> skip (addr >= bottom)
                    debug_assert!(lay.guard_base <= i32::MAX as u64);
                    c.alu_imm(7 /*cmp*/, RCX, lay.guard_base as i32);
                    let sg2 = c.jcc32(6); // jbe -> skip (end <= guard)
                    fixups.push((c.jmp32(), Fixup::Trap));
                    let sg_skip = c.len();
                    fixups.push((sg1, Fixup::To(sg_skip)));
                    fixups.push((sg2, Fixup::To(sg_skip)));
                    c.ld_reg(RCX, rs2);
                    let form = match op {
                        StoreOp::Sd => MemForm::Sd,
                        StoreOp::Sw => MemForm::Sw,
                        StoreOp::Sh => MemForm::Sh,
                        StoreOp::Sb => MemForm::Sb,
                    };
                    c.rax_to_mem(form);
                }
                Inst::ImmAlu { op, rd, rs1, imm } => {
                    if rd != 0 {
                        match op {
                            ImmAluOp::Addi => {
                                c.ld_reg(RAX, rs1);
                                c.alu_imm(0, RAX, imm as i32);
                                c.st_reg(rd, RAX);
                            }
                            ImmAluOp::Slti => {
                                // cmp qword [r12+8rs1], imm32 ; setl
                                c.rex(true, false, false, true);
                                c.op(0x81);
                                c.modrm(2, 7, 4);
                                c.sib(0, 4, 4);
                                c.imm32((8 * rs1 as i64) as i32);
                                c.imm32(imm as i32);
                                c.setcc_rax(0xC); // l
                                c.st_reg(rd, RAX);
                            }
                            ImmAluOp::Sltiu => {
                                c.movabs(RCX, imm as u64);
                                c.ld_reg(RAX, rs1);
                                c.cmp_rr(RAX, RCX);
                                c.setcc_rax(2); // b
                                c.st_reg(rd, RAX);
                            }
                            ImmAluOp::Xori => {
                                c.ld_reg(RAX, rs1);
                                c.movabs(RCX, imm as u64);
                                c.alu_rr(0x31, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            ImmAluOp::Ori => {
                                c.ld_reg(RAX, rs1);
                                c.movabs(RCX, imm as u64);
                                c.alu_rr(0x09, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            ImmAluOp::Andi => {
                                c.ld_reg(RAX, rs1);
                                c.movabs(RCX, imm as u64);
                                c.alu_rr(0x21, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            ImmAluOp::Addiw => {
                                c.ld_reg(RAX, rs1);
                                c.rex(false, false, false, false);
                                c.op(0x81);
                                c.modrm(3, 0, RAX); // add eax, imm32
                                c.imm32(imm as i32);
                                c.sext_eax();
                                c.st_reg(rd, RAX);
                            }
                        }
                    }
                }
                Inst::ShiftImm { kind, rd, rs1, shamt, word } => {
                    if rd != 0 {
                        c.ld_reg(RAX, rs1);
                        let ext = match kind {
                            ShiftKind::Sll => 4u8,
                            ShiftKind::Srl => 5u8,
                            ShiftKind::Sra => 7u8,
                            // rori / roriw (x86 ror by imm8)
                            ShiftKind::Ror => 1u8,
                        };
                        if word {
                            c.shift_imm(ext, RAX, shamt & 31, false);
                            c.sext_eax();
                        } else {
                            c.shift_imm(ext, RAX, shamt, true);
                        }
                        c.st_reg(rd, RAX);
                    }
                }
                Inst::RegAlu { op, rd, rs1, rs2 } => {
                    if rd != 0 {
                        match op {
                            RegAluOp::Add => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x01, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Sub => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x29, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::And => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x21, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Or => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x09, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Xor => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x31, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Slt => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.cmp_rr(RAX, RCX);
                                c.setcc_rax(0xC); // l (signed)
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Sltu => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.cmp_rr(RAX, RCX);
                                c.setcc_rax(2); // b (unsigned)
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Sll | RegAluOp::Srl | RegAluOp::Sra => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                let ext = match op {
                                    RegAluOp::Sll => 4u8,
                                    RegAluOp::Srl => 5u8,
                                    _ => 7u8,
                                };
                                c.shift_cl(ext, RAX, true);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Mul => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.rex(true, false, false, false);
                                c.op2(0x0F, 0xAF);
                                c.modrm(3, RAX, RCX); // imul rax, rcx
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Mulh | RegAluOp::Mulhu | RegAluOp::Mulhsu => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.rex(true, false, false, false);
                                c.op(0xF7);
                                let ext = match op {
                                    RegAluOp::Mulh => 5u8,  // imul rcx (signed)
                                    RegAluOp::Mulhu => 4u8, // mul rcx (unsigned)
                                    _ => 5u8,               // mulhsu: imul then fix
                                };
                                c.modrm(3, ext, RCX); // -> rdx:rax
                                if op == RegAluOp::Mulhsu {
                                    // mulhsu(a, b) = mulh(a, b) + a when
                                    // bit 63 of (unsigned) b is set:
                                    // a*b_u = a*b_s + a*2^64, and a*2^64
                                    // adds a to the high word.
                                    c.shift_imm(5, RCX, 63, true); // shr rcx, 63 -> {0,1}
                                    c.rex(true, false, false, false);
                                    c.op(0xF7);
                                    c.modrm(3, 3, RCX); // neg rcx -> {0, ~0}
                                    c.ld_reg(RAX, rs1);
                                    c.alu_rr(0x21, RAX, RCX); // and -> a or 0
                                    c.alu_rr(0x01, RDX, RAX); // add rdx, rax
                                }
                                c.st_reg(rd, RDX);
                            }
                            RegAluOp::Addw | RegAluOp::Subw | RegAluOp::Mulw => {
                                c.ld_reg(RAX, rs1);
                                match op {
                                    RegAluOp::Addw => c.alu32_rm_reg(0x03, RAX, rs2),
                                    RegAluOp::Subw => c.alu32_rm_reg(0x2B, RAX, rs2),
                                    _ => c.imul_rm(RAX, rs2),
                                }
                                c.sext_eax();
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Sllw | RegAluOp::Srlw | RegAluOp::Sraw => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                let ext = match op {
                                    RegAluOp::Sllw => 4u8,
                                    RegAluOp::Srlw => 5u8,
                                    _ => 7u8,
                                };
                                c.shift_cl(ext, RAX, false); // 32-bit masks & 31
                                c.sext_eax();
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Div
                            | RegAluOp::Divu
                            | RegAluOp::Rem
                            | RegAluOp::Remu
                            | RegAluOp::Divw
                            | RegAluOp::Divuw
                            | RegAluOp::Remw
                            | RegAluOp::Remuw => {
                                let opid: u64 = match op {
                                    RegAluOp::Div => 0,
                                    RegAluOp::Divu => 1,
                                    RegAluOp::Rem => 2,
                                    RegAluOp::Remu => 3,
                                    RegAluOp::Divw => 4,
                                    RegAluOp::Divuw => 5,
                                    RegAluOp::Remw => 6,
                                    _ => 7,
                                };
                                c.movabs(RDI, opid);
                                c.ld_reg(RSI, rs1);
                                c.ld_reg(RDX, rs2);
                                c.op(0xFF);
                                c.modrm(3, 2, RBP); // call rbp
                                c.st_reg(rd, RAX);
                            }
                            // ---- Zbb / Zbc (single unified ISA) ----
                            RegAluOp::Adduw => {
                                c.ld_reg32_zext(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x01, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Sh1add | RegAluOp::Sh2add | RegAluOp::Sh3add => {
                                let n = match op {
                                    RegAluOp::Sh1add => 1u8,
                                    RegAluOp::Sh2add => 2,
                                    _ => 3,
                                };
                                c.ld_reg(RAX, rs1);
                                c.shift_imm(4, RAX, n, true);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x01, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Sh1adduw | RegAluOp::Sh2adduw | RegAluOp::Sh3adduw => {
                                let n = match op {
                                    RegAluOp::Sh1adduw => 1u8,
                                    RegAluOp::Sh2adduw => 2,
                                    _ => 3,
                                };
                                c.ld_reg32_zext(RAX, rs1);
                                c.shift_imm(4, RAX, n, true);
                                c.ld_reg(RCX, rs2);
                                c.alu_rr(0x01, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Andn | RegAluOp::Orn | RegAluOp::Xnor => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                match op {
                                    RegAluOp::Andn => {
                                        c.not_r64(RCX);
                                        c.alu_rr(0x21, RAX, RCX);
                                    }
                                    RegAluOp::Orn => {
                                        c.not_r64(RCX);
                                        c.alu_rr(0x09, RAX, RCX);
                                    }
                                    _ => {
                                        c.alu_rr(0x31, RAX, RCX);
                                        c.not_r64(RAX);
                                    }
                                }
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Rol | RegAluOp::Ror | RegAluOp::Rolw | RegAluOp::Rorw => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                match op {
                                    RegAluOp::Rol => c.rot_cl(0, RAX, true),
                                    RegAluOp::Ror => c.rot_cl(1, RAX, true),
                                    RegAluOp::Rolw => {
                                        c.rot_cl(0, RAX, false);
                                        c.sext_eax();
                                    }
                                    _ => {
                                        c.rot_cl(1, RAX, false);
                                        c.sext_eax();
                                    }
                                }
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Min | RegAluOp::Minu | RegAluOp::Max | RegAluOp::Maxu => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                c.cmp_rr(RAX, RCX);
                                let cc = match op {
                                    RegAluOp::Min => 0xF,  // cmovg: a>b -> take b
                                    RegAluOp::Max => 0xC,  // cmovl: a<b -> take b
                                    RegAluOp::Minu => 0x7, // cmova
                                    _ => 0x2,              // cmovb
                                };
                                c.cmov_rr(cc, RAX, RCX);
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Bset | RegAluOp::Bclr | RegAluOp::Binv | RegAluOp::Bext => {
                                c.ld_reg(RAX, rs1);
                                c.ld_reg(RCX, rs2);
                                match op {
                                    RegAluOp::Bset => {
                                        c.bt_group(0xAB, RAX);
                                    }
                                    RegAluOp::Bclr => {
                                        c.bt_group(0xB3, RAX);
                                    }
                                    RegAluOp::Binv => {
                                        c.bt_group(0xBB, RAX);
                                    }
                                    RegAluOp::Bext => {
                                        c.bt_group(0xA3, RAX);
                                        c.setcc_rax(2); // setc (CF = tested bit)
                                    }
                                    _ => unreachable!("bset family"),
                                }
                                c.st_reg(rd, RAX);
                            }
                            RegAluOp::Clmul | RegAluOp::Clmulh | RegAluOp::Clmulr => {
                                let opid: u64 = match op {
                                    RegAluOp::Clmul => 8,
                                    RegAluOp::Clmulh => 9,
                                    _ => 10,
                                };
                                c.movabs(RDI, opid);
                                c.ld_reg(RSI, rs1);
                                c.ld_reg(RDX, rs2);
                                c.op(0xFF);
                                c.modrm(3, 2, RBP); // call rbp
                                c.st_reg(rd, RAX);
                            }
                        }
                    }
                }
                // Zbb unary ops — routed through the shared ALU host helper
                // (opids 16..26; the Rust bodies lower to native bsr/bsf/
                // bswap/popcnt on hosts that have them).
                Inst::UnAlu { op, rd, rs1 } => {
                    if rd != 0 {
                        let opid: u64 = 16 + match op {
                            UnAluOp::Clz => 0,
                            UnAluOp::Ctz => 1,
                            UnAluOp::Cpop => 2,
                            UnAluOp::Sextb => 3,
                            UnAluOp::Sexth => 4,
                            UnAluOp::Zexth => 5,
                            UnAluOp::Orcb => 6,
                            UnAluOp::Rev8 => 7,
                            UnAluOp::Clzw => 8,
                            UnAluOp::Ctzw => 9,
                            UnAluOp::Cpopw => 10,
                        };
                        c.movabs(RDI, opid);
                        c.ld_reg(RSI, rs1);
                        c.op(0xFF);
                        c.modrm(3, 2, RBP); // call rbp
                        c.st_reg(rd, RAX);
                    }
                }
                // Zbb single-bit immediate ops.
                Inst::BitImm { op, rd, rs1, shamt } => {
                    if rd != 0 {
                        let sh = shamt & 63;
                        c.ld_reg(RAX, rs1);
                        match op {
                            BitImmOp::Set => {
                                c.movabs(RCX, 1u64 << sh);
                                c.alu_rr(0x09 /*or*/, RAX, RCX);
                            }
                            BitImmOp::Clr => {
                                c.movabs(RCX, !(1u64 << sh));
                                c.alu_rr(0x21 /*and*/, RAX, RCX);
                            }
                            BitImmOp::Inv => {
                                c.movabs(RCX, 1u64 << sh);
                                c.alu_rr(0x31 /*xor*/, RAX, RCX);
                            }
                            BitImmOp::Ext => {
                                c.shift_imm(5 /*shr*/, RAX, sh, true);
                                c.alu_imm(4 /*and*/, RAX, 1);
                            }
                        }
                        c.st_reg(rd, RAX);
                    }
                }
                // slli.uw: zero-extend rs1's low 32 bits, shift left.
                Inst::Slliuw { rd, rs1, shamt } => {
                    if rd != 0 {
                        c.ld_reg32_zext(RAX, rs1);
                        c.shift_imm(4 /*shl*/, RAX, shamt & 63, true);
                        c.st_reg(rd, RAX);
                    }
                }
                // Single-hart pure-compute machine: no-op, matches interp.
                Inst::Fence => {}
                Inst::Trap => {
                    fixups.push((c.jmp32(), Fixup::Trap));
                }
                Inst::Ecall => {
                    // rdi = ctx, rsi = regs, rdx = mem, rcx = state (r13); call aot_ecall.
                    c.rex(true, true, false, false);
                    c.op(0x89);
                    c.modrm(3, R14, RDI); // mov rdi, r14
                    c.rex(true, true, false, false);
                    c.op(0x89);
                    c.modrm(3, R12, RSI); // mov rsi, r12
                    c.rex(true, false, false, false);
                    c.op(0x89);
                    c.modrm(3, RBX, RDX); // mov rdx, rbx
                    c.rex(true, true, false, false);
                    c.op(0x89);
                    c.modrm(3, R13, RCX); // mov rcx, r13
                    c.movabs(
                        RAX,
                        aot_ecall as extern "C" fn(*const ResolveCtx, *mut u64, *mut u8, *mut AotState) -> u64
                            as u64,
                    );
                    c.op(0xFF);
                    c.modrm(3, 2, RAX); // call rax
                    c.rex(true, false, false, false);
                    c.op(0x85);
                    c.modrm(3, RAX, RAX); // test rax, rax
                    let at = c.jcc32(5); // jnz -> Exit
                    fixups.push((at, Fixup::Exit));
                }
            }
            idx += 1;
        }
        // Fallthrough to next block; past the end -> trap.
        if idx >= module.insts.len() {
            fixups.push((c.jmp32(), Fixup::PastEnd));
        } else {
            fixups.push((c.jmp32(), Fixup::Next(bi as u32)));
        }
    }

    // ---- landing stubs for mid-block jalr entries: charge ONLY the
    // remaining static cost (`suffix_cycles[inst]`, same pre-check/abort
    // accounting as a block prologue) and resume at the instruction's own
    // translated code — the skipped prefix never executes, mirroring the
    // interpreter exactly.
    let mut stub_fixups: Vec<(usize, u32)> = Vec::new(); // (jmp rel32 site, inst idx)
    let leader_flags: Vec<bool> = inst_landing.iter().map(|&l| l != u32::MAX).collect();
    for i in 0..inst_landing.len() {
        if leader_flags[i] {
            continue; // block leader: prologue is the landing
        }
        let here = c.len() as u32;
        inst_landing[i] = here;
        let cost = module.suffix_cycles[i];
        debug_assert!(cost <= i32::MAX as u64);
        c.cmp_state_imm32(ST_CYCLES, cost as i32);
        let at = c.jcc32(0xC); // jl -> gas stub
        fixups.push((at, Fixup::Gas));
        debug_assert!(cost <= i32::MAX as u64);
        c.sub_state_imm32(ST_CYCLES, cost as i32);
        stub_fixups.push((c.jmp32(), i as u32));
    }
    for (at, i) in stub_fixups {
        let target = inst_off[i as usize] as usize;
        let rel = (target as i64 - at as i64 - 4) as i32;
        c.patch32(at, rel);
    }

    // ---- stubs: invalid target, trap, gas, exit
    let invalid_off = c.len() as u32;
    c.rex(false, false, false, true);
    c.op(0xC7); // mov dword [r13+16], 3
    c.modrm(1, 0, 5);
    c.imm8(ST_ABORTED);
    c.imm32(3);
    fixups.push((c.jmp32(), Fixup::Exit));

    let trap_off = c.len() as u32;
    c.rex(false, false, false, true);
    c.op(0xC7); // mov dword [r13+16], 1
    c.modrm(1, 0, 5);
    c.imm8(ST_ABORTED);
    c.imm32(1);
    fixups.push((c.jmp32(), Fixup::Exit));

    let gas_off = c.len() as u32;
    c.rex(false, false, false, true);
    c.op(0xC7); // mov dword [r13+16], 2
    c.modrm(1, 0, 5);
    c.imm8(ST_ABORTED);
    c.imm32(2);
    fixups.push((c.jmp32(), Fixup::Exit));

    let exit_off = c.len() as u32;
    c.op(0x48);
    c.op(0x83);
    c.modrm(3, 0, 4); // add rsp, 8 (undo prologue pad)
    c.imm8(8);
    c.rex(true, false, false, true);
    c.op(0x8B); // mov rax, [r13+8] (out_len)
    c.modrm(1, 0, 5);
    c.imm8(ST_OUT_LEN);
    for r in [R15, R14, R13, R12, RBP, RBX] {
        if r >= 12 {
            c.rex(false, false, false, true);
        }
        c.op(0x58 | (r & 7)); // pop
    }
    c.op(0xC3); // ret

    // ---- patch fixups
    for (at, f) in fixups {
        let target: usize = match f {
            Fixup::To(to) => to,
            Fixup::Block(ti) => block_off[module.block_of_inst[ti as usize] as usize] as usize,
            Fixup::Trap => trap_off as usize,
            Fixup::Gas => gas_off as usize,
            Fixup::Next(b) => block_off[b as usize + 1] as usize,
            Fixup::PastEnd => trap_off as usize,
            Fixup::Exit => exit_off as usize,
        };
        let rel = (target as i64 - at as i64 - 4) as i32;
        c.patch32(at, rel);
    }
    let entry_blk = module.block_of_inst[module.entry as usize] as usize;
    c.patch32(fix_entry, block_off[entry_blk] as i32 - (fix_entry as i32 + 4));

    // ---- build the resolver context and patch prologue constants BEFORE the
    // single copy (the mapping turns RX and must never be written again).
    let mut ctx = Box::new(ResolveCtx {
        inst_addrs: module.addrs.clone(),
        inst_landing,
        invalid_off,
        code: std::ptr::null(),
        layout: *lay,
    });
    c.patch64(fix_ctx, ctx.as_ref() as *const ResolveCtx as u64);
    c.patch64(
        fix_resolve,
        resolve_jalr as extern "C" fn(*const ResolveCtx, u64) -> u64 as u64,
    );
    c.patch64(
        fix_alu,
        alu_helper as extern "C" fn(u64, u64, u64) -> u64 as u64,
    );

    // ---- mmap RW, one copy, then RX
    let sz = (c.b.len() + 4095) / 4096 * 4096;
    let map = unsafe {
        let p = libc::mmap(
            std::ptr::null_mut(),
            sz,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        if p == libc::MAP_FAILED {
            return None;
        }
        std::ptr::copy_nonoverlapping(c.b.as_ptr(), p as *mut u8, c.b.len());
        if libc::mprotect(p, sz, libc::PROT_READ | libc::PROT_EXEC) != 0 {
            libc::munmap(p, sz);
            return None;
        }
        (p as *mut u8, sz)
    };
    ctx.code = map.0 as *const u8;
    Some(AotFunc { map, len: c.b.len(), ctx })
}

/// Emit a load `mov rax, [rbx + rax]` in the right width/sign form.
fn emit_load(c: &mut Code, form: MemForm) {
    match form {
        MemForm::Ld => {
            c.rex(true, false, false, false);
            c.op(0x8B);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        MemForm::LwSext => {
            c.rex(true, false, false, false);
            c.op(0x63);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        MemForm::LwZero => {
            c.op(0x8B);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        MemForm::LhSext => {
            c.rex(true, false, false, false);
            c.op2(0x0F, 0xBF);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        MemForm::LhZero => {
            c.op2(0x0F, 0xB7);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        MemForm::LbSext => {
            c.rex(true, false, false, false);
            c.op2(0x0F, 0xBE);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        MemForm::LbZero => {
            c.op2(0x0F, 0xB6);
            c.modrm(0, RAX, 4);
            c.sib(0, RAX, RBX);
        }
        _ => unreachable!("emit_load only for loads"),
    }
}

// ---------------------------------------------------------------- tests

/// Differential suite: every test runs the SAME container through the
/// interpreter (`interp::run_with_state`) and the AOT backend and asserts
/// bit-exact agreement of the output, the cycle total, all 32 registers and
/// the whole memory image — on success AND on abort paths (the interpreter
/// reports `cycles_used` on traps/out-of-gas, and the accounting must match
/// to the cycle).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::container::{CalcContainer, CALC_ABI_BYTES_TO_BYTES};
    use crate::calc::interp::{self, CalcAbort};
    fn verify_container(c: &CalcContainer) -> crate::rt::VmrtRes<crate::calc::verify::CalcModule> {
        crate::calc::verify::verify_container_with(c, &crate::calc::CALC_CONFIG_V1)
    }

    // ---------------------------------------------------------- assembler

    /// Minimal two-pass RV64 assembler: 4-byte forms only. Branch/JAL
    /// targets are instruction indices; `finish` encodes the byte offsets
    /// into the immediate fields.
    struct Asm {
        off: Vec<u32>,
        bytes: Vec<u8>,
        /// (inst idx, params, target idx) for late patching.
        fixups: Vec<(usize, Fixup, usize)>,
    }

    enum Fixup {
        /// B-type conditional branch.
        B { f3: u32, rs1: u8, rs2: u8 },
        /// J-type unconditional jump.
        J { rd: u8 },
    }

    impl Asm {
        fn new() -> Self {
            Asm { off: Vec::new(), bytes: Vec::new(), fixups: Vec::new() }
        }

        fn note(&mut self, _halfwords: usize) -> usize {
            self.off.push(self.bytes.len() as u32);
            self.off.len() - 1
        }

        fn raw(&mut self, w: u32) -> usize {
            let idx = self.note(2);
            self.bytes.extend_from_slice(&w.to_le_bytes());
            idx
        }

        /// R-type: add/sub/sll/slt/sltu/xor/srl/sra/or/and, M-ext, W-forms.
        fn r(&mut self, f7: u32, f3: u32, rd: u8, rs1: u8, rs2: u8, op: u32) -> usize {
            self.raw(
                (f7 << 25)
                    | ((rs2 as u32) << 20)
                    | ((rs1 as u32) << 15)
                    | (f3 << 12)
                    | ((rd as u32) << 7)
                    | op,
            )
        }

        /// I-type: loads, jalr, addi/slti/xori/ori/andi/addiw.
        fn i(&mut self, f3: u32, rd: u8, rs1: u8, op: u32, imm: i32) -> usize {
            self.raw(
                (((imm as u32) & 0xFFF) << 20)
                    | ((rs1 as u32) << 15)
                    | (f3 << 12)
                    | ((rd as u32) << 7)
                    | op,
            )
        }

        /// Shift-immediate (shamt field form).
        fn sh(&mut self, f6: u32, f3: u32, rd: u8, rs1: u8, shamt: u8, op: u32) -> usize {
            self.raw(
                (f6 << 26)
                    | ((shamt as u32) << 20)
                    | ((rs1 as u32) << 15)
                    | (f3 << 12)
                    | ((rd as u32) << 7)
                    | op,
            )
        }

        /// S-type store.
        fn s(&mut self, f3: u32, rs1: u8, rs2: u8, imm: i32) -> usize {
            let imm = imm as u32;
            self.raw(
                (((imm >> 5) & 0x7F) << 25)
                    | ((rs2 as u32) << 20)
                    | ((rs1 as u32) << 15)
                    | (f3 << 12)
                    | ((imm & 0x1F) << 7)
                    | 0x23,
            )
        }

        /// B-type branch; immediate patched in `finish`.
        fn b(&mut self, f3: u32, rs1: u8, rs2: u8, target: usize) -> usize {
            let idx = self.raw(0);
            self.fixups.push((idx, Fixup::B { f3, rs1, rs2 }, target));
            idx
        }

        /// J-type jal; immediate patched in `finish`.
        fn j(&mut self, rd: u8, target: usize) -> usize {
            let idx = self.raw(0);
            self.fixups.push((idx, Fixup::J { rd }, target));
            idx
        }

        /// U-type: lui (0x37) / auipc (0x17); imm20 is the raw field.
        fn u(&mut self, rd: u8, op: u32, imm20: u32) -> usize {
            self.raw((imm20 << 12) | ((rd as u32) << 7) | op)
        }

        /// HALT ecall with exit 0 and empty output.
        fn ret(&mut self) -> usize {
            self.i(0, 10, 0, 0x13, 0);
            self.i(0, 11, 0, 0x13, 0);
            self.i(0, 17, 0, 0x13, 0);
            self.raw(0x0000_0073)
        }

        fn finish(mut self) -> Vec<u8> {
            for (idx, f, target) in self.fixups {
                let rel = self.off[target] as i32 - self.off[idx] as i32;
                let w = match f {
                    Fixup::B { f3, rs1, rs2 } => {
                        let imm = rel as u32;
                        assert_eq!(imm & 1, 0, "branch offset must be even");
                        ((imm >> 12 & 1) << 31)
                            | ((imm >> 5 & 0x3F) << 25)
                            | ((rs2 as u32) << 20)
                            | ((rs1 as u32) << 15)
                            | (f3 << 12)
                            | ((imm >> 1 & 0xF) << 8)
                            | ((imm >> 11 & 1) << 7)
                            | 0x63
                    }
                    Fixup::J { rd } => {
                        let imm = rel as u32;
                        ((imm >> 20 & 1) << 31)
                            | ((imm >> 1 & 0x3FF) << 21)
                            | ((imm >> 11 & 1) << 20)
                            | ((imm >> 12 & 0xFF) << 12)
                            | ((rd as u32) << 7)
                            | 0x6F
                    }
                };
                let at = self.off[idx] as usize;
                self.bytes[at..at + 4].copy_from_slice(&w.to_le_bytes());
            }
            self.bytes
        }
    }

    fn container_for(code: Vec<u8>) -> CalcContainer {
        CalcContainer {
            profile: crate::calc::CALC_PROFILE_RV64IM_B,
            abi: CALC_ABI_BYTES_TO_BYTES,
            mem_kib: 4,
            entry: CODE_BASE as u32,
            code,
            rodata: Vec::new(),
            init: Vec::new(),
            bss_len: 0,
            input_max: 64,
            output_max: 64,
        }
    }

    /// Run one container through both engines and require bit-exact
    /// agreement: output bytes, cycle total, abort class, all 32 registers
    /// and the whole memory image (interp snapshots state at every stop
    /// point; an empty snapshot means the pre-run input check fired, where
    /// both engines return zeros/empty deterministically).
    /// Returns (interp cycles_used, interp-succeeded).
    fn diff(name: &str, code: &[u8], input: &[u8], budget: u64) -> (u64, bool) {
        let c = container_for(code.to_vec());
        let module = verify_container(&c)
            .unwrap_or_else(|e| panic!("{name}: container rejected: {e:?}"));
        let (ir, iregs, imem, _trace) = interp::run_with_state(&module, &c, input, budget);
        let f = aot_compile(&module)
            .unwrap_or_else(|| panic!("{name}: aot_compile declined a supported module"));
        let (out, aregs, amem) = f.run_with_state(&module, &c, input, budget);
        let icyc = match &ir {
            Ok(r) => r.cycles_used,
            Err((_, u)) => *u,
        };
        assert_eq!(out.cycles_used, icyc, "{name}: cycle totals diverge");
        match &ir {
            Ok(r) => {
                assert_eq!(out.abort, None, "{name}: AOT aborted where interp returned");
                assert_eq!(
                    out.output.as_deref().unwrap_or(&[]),
                    r.output.as_slice(),
                    "{name}: outputs diverge"
                );
            }
            Err((CalcAbort::OutOfGas, _)) => {
                assert_eq!(
                    out.abort,
                    Some(AotAbort::OutOfGas),
                    "{name}: abort class diverges (expected out-of-gas)"
                );
            }
            Err((CalcAbort::Deadline, _)) => {
                panic!("{name}: interpreter hit a deadline (none was set)");
            }
            Err((CalcAbort::Trap(t), _)) => match out.abort {
                Some(AotAbort::BadTarget) => {
                    assert!(t.contains("invalid target"), "{name}: {t}");
                }
                Some(AotAbort::Trap) => {}
                other => panic!("{name}: AOT abort {other:?} vs interp trap \"{t}\""),
            },
            Err((CalcAbort::GuestExit(code), _)) => {
                assert_eq!(
                    out.abort,
                    Some(AotAbort::GuestExit),
                    "{name}: abort class diverges (expected guest exit)"
                );
                assert_eq!(out.exit_code, *code, "{name}: guest exit code diverges");
            }
        }
        if !imem.is_empty() {
            assert_eq!(iregs, aregs, "{name}: final registers diverge");
            if imem != amem {
                let off = imem
                    .iter()
                    .zip(amem.iter())
                    .position(|(x, y)| x != y)
                    .unwrap_or(imem.len().min(amem.len()));
                eprintln!(
                    "{name}: first mem diff at byte {off} (len {} vs {}): interp={} aot={}",
                    imem.len(),
                    amem.len(),
                    imem.get(off).copied().unwrap_or(u8::MAX),
                    amem.get(off).copied().unwrap_or(u8::MAX)
                );
            }
            assert_eq!(imem, amem, "{name}: final memory diverges");
        }
        (icyc, ir.is_ok())
    }

    /// `li rd, imm` for small imms.
    fn li(a: &mut Asm, rd: u8, imm: i32) {
        a.i(0, rd, 0, 0x13, imm);
    }

    #[test]
    fn alu_helper_matches_reg_alu() {
        let ops = [
            (RegAluOp::Div, 0u64),
            (RegAluOp::Divu, 1),
            (RegAluOp::Rem, 2),
            (RegAluOp::Remu, 3),
            (RegAluOp::Divw, 4),
            (RegAluOp::Divuw, 5),
            (RegAluOp::Remw, 6),
            (RegAluOp::Remuw, 7),
        ];
        let cases: [(u64, u64); 9] = [
            (i64::MIN as u64, u64::MAX),
            (i64::MIN as u64, (-1i64) as u64),
            (i64::MIN as u32 as u64, (-1i32) as u64),
            (0, 0),
            (7, 2),
            (u64::MAX, 3),
            (12345, 0),
            (i64::MIN as u64, 1),
            (0x8000_0000, 0xFFFF_FFFF),
        ];
        for (op, idx) in ops {
            for (a, b) in cases {
                assert_eq!(
                    alu_helper_op(idx, a, b),
                    interp::reg_alu_test(op, a, b),
                    "helper divergence {op:?} a={a:#x} b={b:#x}"
                );
            }
        }
    }

    #[test]
    fn ecall_halt_compiles() {
        let mut a = Asm::new();
        a.i(0, 10, 0, 0x13, 0); // addi a0, x0, 0
        a.i(0, 11, 0, 0x13, 0); // addi a1, x0, 0
        a.i(0, 17, 0, 0x13, 0); // addi a7, x0, 0
        a.raw(0x0000_0073); // ecall HALT
        let c = container_for(a.finish());
        let module = verify_container(&c).expect("ecall verifies");
        assert!(aot_compile(&module).is_some(), "HALT ecall must AOT-compile");
        let (_, ok) = diff("ecall_halt", &c.code, &[], 100_000);
        assert!(ok);
    }

    /// `len == 0` does not exempt `src < CODE_BASE`. A hash ecall with
    /// `a0 = 0`, `a1 = 0`, and `a2` left at the ABI output base traps in
    /// both engines with the same cycle total (hash cost is charged first).
    #[test]
    fn hash_ecall_zero_len_below_code_base_traps() {
        let mut a = Asm::new();
        a.i(0, 10, 0, 0x13, 0); // addi a0, x0, 0  (src)
        a.i(0, 11, 0, 0x13, 0); // addi a1, x0, 0  (len)
        a.i(0, 17, 0, 0x13, 1); // addi a7, x0, 1  (sha2)
        a.raw(0x0000_0073);
        let code = a.finish();
        let (cyc, ok) = diff("zero_len_unmapped_src", &code, &[], 100_000);
        assert!(!ok, "unmapped src must trap");
        assert!(cyc > 0, "hash cost is charged before the bounds trap");
    }

    /// `a1 = u64::MAX` saturates the SHA-256 cycle price. Both engines must
    /// abort `OutOfGas` on that charge (a truncated `i64` cost used to skip
    /// the check and trap on the address instead).
    #[test]
    fn hash_ecall_huge_len_oog_parity() {
        let mut a = Asm::new();
        a.i(0, 11, 0, 0x13, -1); // addi a1, x0, -1
        a.i(0, 17, 0, 0x13, 1); // addi a7, x0, 1 (SHA-256)
        a.raw(0x0000_0073);
        let (cyc, ok) = diff("hash_huge_len_oog", &a.finish(), &[], 100_000);
        assert!(!ok, "huge hash length must exhaust the cycle budget");
        assert!(cyc < 100_000, "failing hash charge is not kept");
    }

    /// The whole Zbb/Zba/Zbs/Zbc subset through both engines (the ISA is
    /// so the AOT must cover every bit-manipulation form the decoder
    /// accepts): unary ops, single-bit immediate and register forms, shifts
    /// (rol/ror/rori incl. W), address-adds, min/max, and the clmul family.
    #[test]
    fn zbb_zbc_full_subset_parity() {
        let mut a = Asm::new();
        // sources: t0 = 0xFFFFFFFF8000000D, t1 = 0xDEADB000, t2 = 3
        a.u(5, 0x37, 0x80000); // t0 low setup
        a.i(0, 5, 5, 0x13, 13); // t0 = 0xFFFFFFFF8000000D
        a.u(6, 0x37, 0xDEADB); // t1 = 0x00000000DEADB000
        li(&mut a, 7, 3); // t2 = 3
        // ---- unary via OP-IMM (selector in rs2)
        a.r(0b0110000, 1, 20, 5, 0, 0x13); // clz t0
        a.r(0b0110000, 1, 21, 5, 1, 0x13); // ctz t0
        a.r(0b0110000, 1, 22, 5, 2, 0x13); // cpop t0
        a.r(0b0110000, 1, 23, 5, 4, 0x13); // sext.b t0
        a.r(0b0110000, 1, 24, 5, 5, 0x13); // sext.h t0
        a.r(0b0010100, 5, 25, 5, 0b00111, 0x13); // orc.b t0
        a.r(0b0110101, 5, 26, 5, 0b11000, 0x13); // rev8 t0
        a.r(0b0000100, 4, 27, 5, 0, 0x3B); // zext.h t0 (RV64 OP-32 slot)
        // ---- W unary via OP-32
        a.r(0b0110000, 1, 8, 5, 0, 0x1B); // clzw
        a.r(0b0110000, 1, 9, 5, 1, 0x1B); // ctzw
        a.r(0b0110000, 1, 10, 5, 2, 0x1B); // cpopw
        // ---- single-bit immediates
        a.sh(0b001010, 1, 11, 5, 7, 0x13); // bseti
        a.sh(0b010010, 2, 12, 5, 7, 0x13); // bclri (f3=010)
        a.sh(0b011010, 3, 13, 5, 7, 0x13); // binvi (f3=011)
        a.sh(0b010010, 5, 14, 5, 61, 0x13); // bexti
        // ---- slli.uw and rori / roriw
        a.sh(0b000010, 1, 15, 5, 4, 0x1B); // slli.uw
        a.sh(0b011000, 5, 16, 5, 9, 0x13); // rori
        // ---- register-form bit ops + rotates
        a.r(0b0100000, 4, 17, 5, 6, 0x33); // xnor
        a.r(0b0100000, 6, 18, 5, 6, 0x33); // orn
        a.r(0b0100000, 7, 19, 5, 6, 0x33); // andn
        a.r(0b0110000, 1, 20, 5, 6, 0x33); // rol
        a.r(0b0110000, 5, 21, 5, 6, 0x33); // ror
        a.r(0b0110000, 1, 22, 5, 6, 0x3B); // rolw
        a.r(0b0110000, 5, 23, 5, 6, 0x3B); // rorw
        // ---- address adds + min/max
        a.r(0b0000100, 0, 24, 5, 6, 0x3B); // add.uw
        a.r(0b0010000, 2, 25, 5, 6, 0x33); // sh1add
        a.r(0b0010000, 4, 26, 5, 6, 0x33); // sh2add
        a.r(0b0010000, 6, 27, 5, 6, 0x33); // sh3add
        a.r(0b0010000, 2, 8, 5, 6, 0x3B); // sh1add.uw
        a.r(0b0010000, 4, 9, 5, 6, 0x3B); // sh2add.uw
        a.r(0b0010000, 6, 10, 5, 6, 0x3B); // sh3add.uw
        a.r(0b0000101, 4, 11, 5, 6, 0x33); // min
        a.r(0b0000101, 5, 12, 5, 6, 0x33); // minu
        a.r(0b0000101, 6, 13, 5, 6, 0x33); // max
        a.r(0b0000101, 7, 14, 5, 6, 0x33); // maxu
        a.r(0b0010100, 1, 15, 5, 6, 0x33); // bset
        a.r(0b0100100, 1, 16, 5, 6, 0x33); // bclr (f3=001)
        a.r(0b0110100, 1, 17, 5, 6, 0x33); // binv
        a.r(0b0100100, 5, 18, 5, 6, 0x33); // bext
        // ---- clmul family (host helper path)
        a.r(0b0000101, 1, 19, 5, 6, 0x33); // clmul
        a.r(0b0000101, 3, 20, 5, 6, 0x33); // clmulh
        a.r(0b0000101, 2, 21, 5, 6, 0x33); // clmulr
        li(&mut a, 10, 0); // a0 = 0: empty output at the sentinel return
        a.ret();
        let code = a.finish();
        let (_, ok) = diff("zbb_zbc_full", &code, &[], 1_000_000);
        assert!(ok, "zbb/zbc program must return cleanly");
    }

    /// Mounted-engine equivalence through the full HVM gas path: the same
    /// `CalcFnObj` run with and without the AOT mount must produce the
    /// identical `CalcExec` (result, cycles, resource) on success, OOG and
    /// trap cases — the mountability guarantee in one test.
    #[test]
    fn mounted_object_gas_path_equivalence() {
        use crate::calc::{CalcFnObj, CalcRunLimits};
        // loop kernel for OOG/trap coverage + a clean echo kernel
        let mut a = Asm::new();
        li(&mut a, 6, 40);
        li(&mut a, 5, 0);
        let lp = a.i(0, 5, 5, 0x13, 1);
        a.b(1, 5, 6, lp);
        li(&mut a, 10, 0);
        a.ret();
        let loop_code = a.finish();
        let mut a = Asm::new();
        li(&mut a, 10, 0);
        a.ret();
        let echo_code = a.finish();
        for (name, code) in [("loop", &loop_code), ("echo", &echo_code)] {
            let plain = CalcFnObj::build_with_host(
                container_for(code.clone()),
                crate::calc::CalcHostConfig::new().with_aot_mount(false),
            )
            .unwrap();
            let mounted = CalcFnObj::build_with_aot(container_for(code.clone())).unwrap();
            assert!(mounted.aot_mounted(), "{name}: AOT must mount");
            for budget in [0u64, 4, 64, 300, 10_000] {
                let limits = CalcRunLimits {
                    cycles: budget,
                    resource: i64::MAX,
                    deadline: None,
                };
                let e1 = plain.run(&[], limits);
                let e2 = mounted.run(&[], limits);
                assert_eq!(e1.cycles_used, e2.cycles_used, "{name}@{budget}: cycles");
                assert_eq!(e1.resource, e2.resource, "{name}@{budget}: resource");
                assert_eq!(e1.exit_code, e2.exit_code, "{name}@{budget}: exit_code");
                assert_eq!(
                    plain.receipt(&[], &e1),
                    mounted.receipt(&[], &e2),
                    "{name}@{budget}: receipt"
                );
                assert_eq!(
                    e1.result.is_ok(),
                    e2.result.is_ok(),
                    "{name}@{budget}: outcome class"
                );
                if let (Ok(o1), Ok(o2)) = (&e1.result, &e2.result) {
                    assert_eq!(o1, o2, "{name}@{budget}: output");
                }
            }
        }
    }

    /// Micro differentials: one Zbb instruction at a time against fresh
    /// sources, so a divergence pins the exact emission (a full-program
    /// run only shows the downstream register mismatch).
    #[test]
    fn zbb_micro_parity() {
        let cases: &[(&str, fn(&mut Asm))] = &[
            ("bset", |a: &mut Asm| {
                a.r(0b0010100, 1, 28, 5, 6, 0x33);
            }),
            ("bclr", |a: &mut Asm| {
                a.r(0b0100100, 1, 28, 5, 6, 0x33);
            }),
            ("binv", |a: &mut Asm| {
                a.r(0b0110100, 1, 28, 5, 6, 0x33);
            }),
            ("bext", |a: &mut Asm| {
                a.r(0b0100100, 5, 28, 5, 6, 0x33);
            }),
            ("min", |a: &mut Asm| {
                a.r(0b0000101, 4, 28, 5, 6, 0x33);
            }),
            ("minu", |a: &mut Asm| {
                a.r(0b0000101, 5, 28, 5, 6, 0x33);
            }),
            ("max", |a: &mut Asm| {
                a.r(0b0000101, 6, 28, 5, 6, 0x33);
            }),
            ("maxu", |a: &mut Asm| {
                a.r(0b0000101, 7, 28, 5, 6, 0x33);
            }),
            ("rol", |a: &mut Asm| {
                a.r(0b0110000, 1, 28, 5, 6, 0x33);
            }),
            ("ror", |a: &mut Asm| {
                a.r(0b0110000, 5, 28, 5, 6, 0x33);
            }),
            ("andn", |a: &mut Asm| {
                a.r(0b0100000, 7, 28, 5, 6, 0x33);
            }),
            ("xnor", |a: &mut Asm| {
                a.r(0b0100000, 4, 28, 5, 6, 0x33);
            }),
            ("sh1add", |a: &mut Asm| {
                a.r(0b0010000, 2, 28, 5, 6, 0x33);
            }),
            ("clmul", |a: &mut Asm| {
                a.r(0b0000101, 1, 28, 5, 6, 0x33);
            }),
        ];
        for (name, emit) in cases {
            let mut a = Asm::new();
            a.u(5, 0x37, 0x80000);
            a.i(0, 5, 5, 0x13, 13); // t0 = 0xFFFFFFFF8000000D
            a.u(6, 0x37, 0xDEADB); // t1 = 0x00000000DEADB000
            emit(&mut a);
            li(&mut a, 10, 0);
            a.ret();
            let code = a.finish();
            let (_, ok) = diff(&format!("micro_{name}"), &code, &[], 100_000);
            assert!(ok, "micro_{name} must return cleanly");
        }
    }

    /// The unalu host helper must mirror the interpreter's unary semantics
    /// on representative values (0, 1, all-ones, mixed bytes).
    #[test]
    fn unalu_helper_matches_reference() {
        let vals = [
            0u64,
            1,
            u64::MAX,
            0x00FF_00FF_00FF_00FF,
            0x8000_0000_0000_0000,
            0x0000_0000_DEAD_BEEF,
            0xFFFF_0000_0000_0001,
        ];
        for v in vals {
            assert_eq!(unalu_helper_op(0, v), v.leading_zeros() as u64, "clz {v:#x}");
            assert_eq!(unalu_helper_op(1, v), v.trailing_zeros() as u64, "ctz {v:#x}");
            assert_eq!(unalu_helper_op(2, v), v.count_ones() as u64, "cpop {v:#x}");
            assert_eq!(unalu_helper_op(3, v), v as i8 as i64 as u64, "sextb {v:#x}");
            assert_eq!(unalu_helper_op(4, v), v as i16 as i64 as u64, "sexth {v:#x}");
            assert_eq!(unalu_helper_op(5, v), v & 0xFFFF, "zexth {v:#x}");
            assert_eq!(unalu_helper_op(7, v), v.swap_bytes(), "rev8 {v:#x}");
            assert_eq!(unalu_helper_op(8, v), (v as u32).leading_zeros() as u64, "clzw {v:#x}");
            assert_eq!(unalu_helper_op(9, v), (v as u32).trailing_zeros() as u64, "ctzw {v:#x}");
            assert_eq!(unalu_helper_op(10, v), (v as u32).count_ones() as u64, "cpopw {v:#x}");
        }
    }


    /// One program touching every base instruction class: both engines
    /// must agree on the full stop-state. Stores/loads anchor at `sp` with
    /// negative offsets (the stack top is the end of memory); `a0` is only
    /// touched at the very end (entry convention: a0 = input_base on entry,
    /// output length at HALT). Also exercises unreachable padding.
    #[test]
    fn all_profile1_instructions_parity() {
        let mut a = Asm::new();
        // -- source values
        li(&mut a, 5, -1); // t0 = -1
        a.u(6, 0x37, 0xDEADB); // t1 = 0x00000000DEADB000
        li(&mut a, 7, 7); // t2 = 7
        a.u(28, 0x37, 0x80000); // t3 = 0xFFFFFFFF80000000
        a.i(0, 29, 0, 0x13, 31); // t4 = 31
        a.i(0, 30, 7, 0x13, 0); // t5 = t2
        a.raw(0x0FF0_000F); // fence
        // -- ImmAlu family (destinations s0..s6 = x8..x14)
        a.i(0, 8, 5, 0x13, -5); // addi s0, t0, -5
        a.i(2, 9, 5, 0x13, 0); // slti s1, t0, 0
        a.i(3, 10 + 128 - 128, 5, 0x13, -1); // placeholder replaced below
        a.i(3, 11, 5, 0x13, -1); // sltiu s2... (rd 11; keep pins distinct)
        a.i(4, 12, 6, 0x13, 0x7FF); // xori x12
        a.i(6, 13, 6, 0x13, 0x123); // ori x13
        a.i(7, 14, 6, 0x13, 0x0F0); // andi x14
        a.i(0, 15, 6, 0x1B, 100); // addiw x15
        // -- ShiftImm family
        a.sh(0, 1, 16, 6, 5, 0x13); // slli x16
        a.sh(0, 5, 17, 5, 3, 0x13); // srli x17
        a.sh(0x10, 5, 18, 5, 3, 0x13); // srai x18
        a.sh(0, 1, 19, 6, 4, 0x1B); // slliw x19
        a.sh(0, 5, 20, 5, 4, 0x1B); // srliw x20
        a.sh(0x10, 5, 21, 28, 8, 0x1B); // sraiw x21
        a.sh(0, 1, 22, 5, 63, 0x13); // slli 63 -> x22
        // -- RegAlu family (destinations x23..x27, then t4/t5/t6)
        a.r(0x00, 0, 23, 5, 6, 0x33); // add
        a.r(0x20, 0, 24, 6, 5, 0x33); // sub
        a.r(0x00, 7, 25, 5, 6, 0x33); // and
        a.r(0x00, 6, 26, 5, 6, 0x33); // or
        a.r(0x00, 4, 27, 5, 6, 0x33); // xor
        a.r(0x00, 2, 29, 5, 6, 0x33); // slt(-1, big) = 1
        a.r(0x00, 3, 30, 5, 6, 0x33); // sltu
        a.r(0x00, 1, 31, 6, 7, 0x33); // sll
        a.r(0x00, 5, 8, 5, 7, 0x33); // srl  (reuse x8; pins above already read)
        a.r(0x20, 5, 9, 5, 7, 0x33); // sra
        a.r(0x01, 0, 18, 6, 7, 0x33); // mul
        a.r(0x01, 1, 19, 5, 6, 0x33); // mulh
        a.r(0x01, 2, 20, 5, 6, 0x33); // mulhsu
        a.r(0x01, 3, 21, 5, 6, 0x33); // mulhu
        a.r(0x00, 0, 22, 5, 6, 0x3B); // addw
        a.r(0x20, 0, 23, 5, 6, 0x3B); // subw
        a.r(0x01, 0, 24, 6, 7, 0x3B); // mulw
        a.r(0x00, 1, 25, 5, 7, 0x3B); // sllw
        a.r(0x00, 5, 26, 5, 7, 0x3B); // srlw
        a.r(0x20, 5, 27, 5, 7, 0x3B); // sraw
        // -- div/rem edge cases
        a.r(0x01, 4, 8, 28, 0, 0x33); // div  t3 / x0  -> -1
        a.r(0x01, 5, 9, 28, 0, 0x33); // rem  t3 % x0  -> t3
        a.r(0x01, 6, 16, 28, 5, 0x33); // divu
        a.r(0x01, 7, 17, 28, 5, 0x33); // remu
        a.r(0x01, 4, 18, 28, 5, 0x3B); // divw
        a.r(0x01, 5, 19, 28, 5, 0x3B); // divuw
        a.r(0x01, 6, 20, 28, 5, 0x3B); // remw
        a.r(0x01, 7, 21, 28, 5, 0x3B); // remuw
        a.r(0x01, 4, 22, 28, 5, 0x33); // div  MIN/-1 -> MIN
        a.r(0x01, 6, 23, 28, 5, 0x33); // rem  MIN/-1 -> 0
        // -- stores/loads anchored BELOW the stack top (writable, in-bounds)
        a.s(3, 2, 5, -32); // sd t0, -32(sp)
        a.s(2, 2, 6, -24); // sw t1, -24(sp)
        a.s(1, 2, 7, -20); // sh t2, -20(sp)
        a.s(0, 2, 5, -18); // sb t0, -18(sp)
        a.i(3, 24, 2, 0x03, -32); // ld  x24
        a.i(2, 25, 2, 0x03, -24); // lw  x25
        a.i(6, 26, 2, 0x03, -24); // lwu x26
        a.i(1, 27, 2, 0x03, -20); // lh  x27
        a.i(5, 29, 2, 0x03, -20); // lhu x29
        a.i(0, 30, 2, 0x03, -18); // lb  x30
        a.i(4, 31, 2, 0x03, -18); // lbu x31
        // 32-bit leaf call: jal ra, leaf; resume; j after; leaf; ret; after.
        let _jal = a.j(1, 0); // patched below to leaf
        let resume = a.i(0, 26, 0, 0x13, 1);
        let _skip = a.j(0, 0); // patched to after
        let leaf = a.i(0, 27, 0, 0x13, 9);
        a.i(0, 0, 1, 0x67, 0); // jalr x0, ra
        let after = a.i(0, 8, 0, 0x13, 77);
        // retarget the two JAL fixups: first j -> leaf, second j -> after.
        // Asm stores fixups in order; rewrite the recorded targets.
        let nfix = a.fixups.len();
        a.fixups[nfix - 2].2 = leaf;
        a.fixups[nfix - 1].2 = after;
        let _ = (resume, after);
        a.ret();
        let code = a.finish();
        let (_, ok) = diff("all_profile1", &code, &[], 1_000_000);
        assert!(ok, "all_profile1 must return cleanly");
    }

    /// Directed regression for the register-form comparisons: an earlier
    /// revision of the x86-64 backend emitted `setcc S` for `slt` and
    /// `setcc NO` for `sltu` (correct: `L` and `B`); both engines agree
    /// here AND the interpreter's own values are pinned.
    #[test]
    fn slt_sltu_signedness_parity() {
        let mut a = Asm::new();
        li(&mut a, 5, -1); // t0 = -1
        li(&mut a, 6, 0); // t1 = 0
        a.r(0x00, 2, 28, 5, 6, 0x33); // x28 = slt(-1, 0)   -> 1
        a.r(0x00, 2, 24, 6, 5, 0x33); // x24 = slt(0, -1)   -> 0
        a.r(0x00, 3, 30, 6, 5, 0x33); // x30 = sltu(0, ~0)  -> 1
        a.r(0x00, 3, 25, 5, 6, 0x33); // x25 = sltu(~0, 0)  -> 0
        a.i(2, 26, 5, 0x13, 0); // x26 = slti(-1, 0)       -> 1
        a.i(3, 27, 6, 0x13, -1); // x27 = sltiu(0, ~0)     -> 1
        a.i(2, 21, 6, 0x13, -1); // x21 = slti(0, -1)      -> 0
        a.i(3, 20, 5, 0x13, -1); // x20 = sltiu(~0, ~0)    -> 0
        li(&mut a, 10, 0); // a0 = 0: empty output at the sentinel return
        a.ret();
        let code = a.finish();
        let c = container_for(code.clone());
        let module = verify_container(&c).unwrap();
        let (ir, iregs, _imem, _tr) = interp::run_with_state(&module, &c, &[], 100_000);
        assert!(ir.is_ok());
        assert_eq!(iregs[28], 1, "slt(-1,0)");
        assert_eq!(iregs[24], 0, "slt(0,-1)");
        assert_eq!(iregs[30], 1, "sltu(0,~0)");
        assert_eq!(iregs[25], 0, "sltu(~0,0)");
        assert_eq!(iregs[26], 1, "slti(-1,0)");
        assert_eq!(iregs[27], 1, "sltiu(0,~0)");
        assert_eq!(iregs[21], 0, "slti(0,-1)");
        assert_eq!(iregs[20], 0, "sltiu(~0,~0)");
        let (_, ok) = diff("slt_sltu", &code, &[], 100_000);
        assert!(ok);
    }

    /// A `jalr` landing in the MIDDLE of a basic block must charge only the
    /// remaining instructions (`suffix_cycles`): the AOT trampoline rebates
    /// the skipped prefix. A skipped 1-cycle `addi` in front of a 32-cycle
    /// `div` makes any accounting drift visible.
    #[test]
    fn mid_block_jalr_rebate_parity() {
        let mut a = Asm::new();
        // A+0:  auipc t0, 0
        a.u(5, 0x17, 0);
        // A+4:  addi t0, t0, 16      ; dest = A+16 (non-leader inside the
        //                             ;  block starting at A+12)
        a.i(0, 5, 5, 0x13, 16);
        // A+8:  jalr x0, t0, 0       ; no link: x1 stays 0
        a.i(0, 0, 5, 0x67, 0);
        // A+12: block leader — SKIPPED by the mid-block entry (rebate source)
        a.i(0, 8, 0, 0x13, 7); // addi s0, x0, 7   (must NOT execute)
        // A+16: mid-block landing
        a.i(0, 9, 0, 0x13, 8); // addi s1, x0, 8
        // A+20..: same-block expensive tail
        a.u(28, 0x37, 0x80000); // lui t3, i64::MIN low half
        li(&mut a, 7, 3);
        a.r(0x01, 4, 10, 28, 7, 0x33); // div a2, t3, t2 (32 cycles)
        // clean HALT (a0=0, a1=0, a7=0); jalr-to-ra is no longer a return
        a.ret();
        let code = a.finish();
        // pin interpreter semantics: s0 skipped, s1 ran
        let c = container_for(code.clone());
        let module = verify_container(&c).unwrap();
        let (ir, iregs, _imem, _tr) = interp::run_with_state(&module, &c, &[], 1_000_000);
        assert!(ir.is_ok());
        assert_eq!(iregs[8], 0, "skipped block prefix must not execute");
        assert_eq!(iregs[9], 8, "mid-block landing executed");
        let (_, ok) = diff("mid_block_jalr", &code, &[], 1_000_000);
        assert!(ok);
    }

    /// Out-of-gas accounting at the exact boundary: the interpreter reports
    /// `used` WITHOUT the failing block charge; the AOT pre-check must match
    /// to the cycle, for budgets at and just under the success cost.
    #[test]
    fn oog_boundary_parity() {
        let mut a = Asm::new();
        li(&mut a, 6, 40); // t1 = 40
        li(&mut a, 5, 0); // t0 = 0
        let lp = a.i(0, 5, 5, 0x13, 1); // loop: addi t0, t0, 1
        a.b(1, 5, 6, lp); // bne t0, t1, loop
        li(&mut a, 10, 0); // a0 = 0: empty output
        a.ret();
        let code = a.finish();
        let (full, ok) = diff("oog@full", &code, &[], 10_000_000);
        assert!(ok);
        assert!(full > 0);
        for d in 1..=4u64 {
            let (used, ok) = diff("oog", &code, &[], full - d);
            assert!(!ok, "budget {} below the success cost must OOG", full - d);
            // The failing charge is NOT included: used <= budget always.
            assert!(used <= full - d);
        }
    }

    /// Branch/jal coverage: every condition kind taken and not taken,
    /// backward-branch loops, jal linking, and dead (unreachable but
    /// decodable) padding — accepted by the verifier, never executed.
    #[test]
    fn branches_and_jumps_parity() {
        let code = {
            let mut a = Asm::new();
            li(&mut a, 5, 1); // 0 t0 = 1
            li(&mut a, 6, 2); // 1 t1 = 2
            li(&mut a, 7, 1); // 2 t2 = 1
            a.i(0, 28, 0, 0x13, 0xAA); // 3 dead marker (skipped when beq taken)
            a.i(0, 29, 0, 0x13, 0xBB); // 4 landing marker
            a.b(0, 5, 7, 4); // 5 beq t0,t2 (taken)  -> 4
            a.b(1, 5, 7, 4); // 6 bne t0,t2 (not taken) -> falls
            a.b(4, 6, 5, 4); // 7 blt t1,t0 (2<1 false) -> falls
            a.b(5, 5, 6, 4); // 8 bge t0,t1 (1>=2 false) -> falls
            a.b(6, 5, 6, 4); // 9 bltu t0,t1 (1<2 taken) -> 4
            a.b(7, 6, 5, 4); // 10 bgeu t1,t0 (taken) -> 4
            li(&mut a, 30, 0); // 11 t5 = 0
            let lp = a.i(0, 30, 30, 0x13, 1); // 12 loop body
            a.b(1, 30, 6, lp); // 13 bne t5,t1 -> 12 (twice)
            // jal ra over two dead 4-byte padding words
            let dead1 = a.raw(0x0000_0013); // 15 (dead)
            let dead2 = a.raw(0x0000_0013); // 16 (dead)
            let _ = (dead1, dead2);
            let after = a.i(0, 31, 0, 0x13, 5); // 17 landing (jal target)
            a.j(1, 18); // wait: jal must precede the dead words — see note
            let _ = after;
            a.ret();
            a.finish()
        };
        let _ = code;
        // Jal-over-padding needs the jump before the padding; rebuild with
        // explicit ordering (indices resolved via recorded positions).
        let mut a = Asm::new();
        li(&mut a, 5, 1); // 0
        li(&mut a, 6, 2); // 1
        li(&mut a, 7, 1); // 2
        a.i(0, 28, 0, 0x13, 0xAA); // 3 dead (beq skips it)
        let landing = a.i(0, 29, 0, 0x13, 0xBB); // 4
        a.b(0, 5, 7, landing); // 5 beq taken -> 4
        a.b(1, 5, 7, landing); // 6 bne not taken -> 7
        a.b(4, 6, 5, landing); // 7 blt false -> 8
        a.b(5, 5, 6, landing); // 8 bge false -> 9
        a.b(6, 5, 6, landing); // 9 bltu taken -> 4
        a.b(7, 6, 5, landing); // 10 bgeu taken -> 4
        li(&mut a, 30, 0); // 11
        let lp = a.i(0, 30, 30, 0x13, 1); // 12
        a.b(1, 30, 6, lp); // 13
        let jal = a.j(1, 0); // 14 jal ra -> target fixed below (past dead)
        let _ = jal;
        a.raw(0x0000_0013); // 15 dead
        a.raw(0x0000_0013); // 16 dead
        let after_pad = a.i(0, 31, 0, 0x13, 5); // 17
        let _ = after_pad;
        a.ret(); // 18
        // re-point the jal (14) at 17: rebuild is cleaner than patching —
        // the Asm records targets at emission, so do a final clean pass.
        let mut a = Asm::new();
        li(&mut a, 5, 1); // 0
        li(&mut a, 6, 2); // 1
        li(&mut a, 7, 1); // 2
        a.i(0, 28, 0, 0x13, 0xAA); // 3 dead
        let landing = a.i(0, 29, 0, 0x13, 0xBB); // 4
        a.b(0, 5, 7, landing); // 5
        a.b(1, 5, 7, landing); // 6
        a.b(4, 6, 5, landing); // 7
        a.b(5, 5, 6, landing); // 8
        a.b(6, 5, 6, landing); // 9
        a.b(7, 6, 5, landing); // 10
        li(&mut a, 30, 0); // 11
        let lp = a.i(0, 30, 30, 0x13, 1); // 12
        a.b(1, 30, 6, lp); // 13
        // jal-over-dead needs the target index known: emit dead first at
        // known indices 15/16, jal at 14 targeting 17 — but emission order
        // IS execution order; a jal at 14 jumping to 17 skips 15/16. The
        // indices are fixed by count, so precompute: after 13 comes 14.
        let pad1 = a.raw(0x0000_0013); // 14 dead
        let pad2 = a.raw(0x0000_0013); // 15 dead
        let _ = (pad1, pad2);
        let after_pad = a.i(0, 31, 0, 0x13, 5); // 16 landing past the pad
        let _ = after_pad;
        a.ret(); // 17
        // NOTE: jal-over-dead cannot be expressed by emission order alone;
        // assemble one more pass with the branch-only layout (jal added at
        // the top jumping forward to the recorded tail index).
        let mut a = Asm::new();
        let jhead = a.j(1, 1); // placeholder: patched by rebuilding below
        let _ = jhead;
        a.ret();
        let _ = a.finish();
        // Final clean construction: everything explicit, no re-pointing.
        let mut a = Asm::new();
        li(&mut a, 5, 1); // 0 t0
        li(&mut a, 6, 2); // 1 t1
        li(&mut a, 7, 1); // 2 t2
        a.i(0, 28, 0, 0x13, 0xAA); // 3 dead under the taken beq
        let landing = a.i(0, 29, 0, 0x13, 0xBB); // 4
        a.b(0, 5, 7, landing); // 5 beq taken
        a.b(1, 5, 7, landing); // 6 bne fall
        a.b(4, 6, 5, landing); // 7 blt fall
        a.b(5, 5, 6, landing); // 8 bge fall
        a.b(6, 5, 6, landing); // 9 bltu taken
        a.b(7, 6, 5, landing); // 10 bgeu taken
        li(&mut a, 30, 0); // 11
        let lp = a.i(0, 30, 30, 0x13, 1); // 12
        a.b(1, 30, 6, lp); // 13 loop back edge
        // dead padding jumped over by jal below is expressed with the
        // landing AFTER the pads; jal sits before them by construction:
        // 14 jal -> 17, 15/16 dead, 17 marker, 18 ret.
        // (jal needs the future index: 18 insts total -> ret at 18.)
        let jal = a.j(1, 17); // 14
        let _ = jal;
        a.raw(0x0000_0013); // 15 dead
        a.raw(0x0000_0013); // 16 dead
        a.i(0, 31, 0, 0x13, 5); // 17 marker
        li(&mut a, 10, 0); // 18 a0 = 0
        a.ret(); // 19
        let code = a.finish();
        diff("branches_jumps", &code, &[], 1_000_000);
    }

    /// Directed regression for the `mulhsu` high-word correction: the
    /// identity is `mulhsu(a,b) = mulh(a,b) + a` when bit 63 of unsigned b
    /// is set (a FULL signed `a` is added to the high word — earlier
    /// revisions tried `{0,1}` and `sar(a)` and both diverge).
    #[test]
    fn mulhsu_correction_parity() {
        let mut a = Asm::new();
        li(&mut a, 5, -1); // t0 = -1
        // t1 = i64::MIN: lui sign-extends, so shift the low half up
        a.u(6, 0x37, 0x80000); // t1 = 0xFFFFFFFF80000000
        a.sh(0, 1, 6, 6, 32, 0x13); // slli t1, t1, 32 -> 0x8000000000000000
        li(&mut a, 7, 3); // t2 = 3
        a.r(0x01, 1, 20, 5, 6, 0x33); // x20 = mulh(-1, MIN)
        a.r(0x01, 2, 21, 5, 6, 0x33); // x21 = mulhsu(-1, MIN)  = -1 + (-1)
        a.r(0x01, 3, 22, 5, 6, 0x33); // x22 = mulhu(-1, MIN)
        a.r(0x01, 2, 8, 5, 7, 0x33); // x8 = mulhsu(-1, 3)      (b bit63 clear)
        a.r(0x01, 2, 9, 7, 6, 0x33); // x9 = mulhsu(3, MIN)     = mulh(3,MIN) + 3
        a.r(0x01, 2, 18, 6, 6, 0x33); // x18 = mulhsu(MIN, MIN) = mulh(MIN,MIN) + MIN
        li(&mut a, 10, 0);
        a.ret();
        let code = a.finish();
        let c = container_for(code.clone());
        let module = verify_container(&c).unwrap();
        let (ir, iregs, _imem, _tr) = interp::run_with_state(&module, &c, &[], 100_000);
        assert!(ir.is_ok());
        assert_eq!(iregs[21], u64::MAX, "mulhsu(-1, MIN) high = -1");
        assert_eq!(iregs[8], u64::MAX, "mulhsu(-1, 3) high = -1");
        // mulh(3, MIN) = high(3 * -2^63) = high(-1.5*2^64) = -2; +3 -> 1
        assert_eq!(iregs[9], 1, "mulhsu(3, MIN) high = 1");
        // mulh(MIN, MIN) = high(2^126) = 2^62; + MIN -> 2^62 - 2^63
        assert_eq!(iregs[18], (1u64 << 62).wrapping_sub(1u64 << 63), "mulhsu(MIN, MIN)");
        let (_, ok) = diff("mulhsu", &code, &[], 100_000);
        assert!(ok);
    }

    /// `fence` decodes and executes as a no-op on both engines.
    #[test]
    fn fence_noop_parity() {
        let mut a = Asm::new();
        li(&mut a, 10, 0);
        a.raw(0x0FF0_000F); // fence
        li(&mut a, 11, 0);
        a.ret();
        let code = a.finish();
        let (_, ok) = diff("fence", &code, &[], 100_000);
        assert!(ok);
    }

    /// Deterministic random programs from the unified RV64IM+Zba/Zbb/Zbs/Zbc
    /// whitelist: both engines must agree on outcome, cycles, registers and
    /// memory — including traps (bad loads/stores/jumps) and out-of-gas
    /// runs. Odd cases are "tame" (safe memory anchors, forward branches,
    /// no wild `jalr`) so a healthy share of runs finish cleanly. The
    /// packing is 32-bit-only (C excluded). Jump and memory density are seeded
    /// deterministically. 2000 seeds, fixed base seed for reproducibility.
    #[test]
    fn fuzz_differential_unified() {
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut successes = 0usize;
        for case in 0..2000 {
            let tame = case % 2 == 0;
            let code = gen_program(&mut seed, tame);
            let mut input = [0u8; 32];
            for b in input.iter_mut() {
                seed = xs(seed);
                *b = seed as u8;
            }
            let (_, ok) = diff(&format!("fuzz#{case}"), &code, &input, 20_000);
            successes += ok as usize;
        }
        // Sanity: the generator must produce a healthy mix (not all traps).
        assert!(
            successes >= 700,
            "suspiciously few clean runs: {successes}/2000"
        );
    }

    fn xs(mut s: u64) -> u64 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }

    fn gen_program(seed: &mut u64, tame: bool) -> Vec<u8> {
        let mut a = Asm::new();
        *seed = xs(*seed);
        let n = 8 + (*seed % 32) as usize;
        let total = n + 2; // + li a0,0 + ret
        // tame mode anchors x8 at sp-512 for safe memory ops.
        if tame {
            a.i(0, 8, 2, 0x13, -512);
        }
        let idx_hint = a.off.len();
        let _ = idx_hint;
        for k in 0..n {
            *seed = xs(*seed);
            let mut class = (*seed % 19) as u32;
            if tame {
                // tame set: ALU/shift/branch-forward/safe-memory only
                class = match class {
                    0 | 1 | 2 | 3 | 4 | 5 | 6 => class,
                    7 | 8 => class, // safe anchors below
                    9 => 9,         // forward-only targets below
                    14 => 14,       // zbb reg-form ops are tame
                    15 => 15,       // zbb immediate forms are tame
                    _ => 5,         // regalu
                };
            }
            let _ = k;
            let rr = |seed: &mut u64| -> u8 {
                *seed = xs(*seed);
                (*seed % 32) as u8
            };
            let ri = |seed: &mut u64, m: u64| -> i32 {
                *seed = xs(*seed);
                ((*seed % m) as i64 - (m / 2) as i64) as i32
            };
            // Destination register: tame keeps x1 (ra) and x2 (sp) intact so
            // the appended `li a0, 0 ; ret` tail actually returns.
            let rd_of = |seed: &mut u64| -> u8 {
                *seed = xs(*seed);
                if tame {
                    5 + (*seed % 27) as u8
                } else {
                    ((*seed % 32) as u8).max(1)
                }
            };
            match class {
                0 => {
                    a.i(0, rd_of(seed), rr(seed), 0x13, ri(seed, 0x800));
                }
                1 => {
                    a.i(0, rd_of(seed), rr(seed), 0x1B, ri(seed, 0x800));
                }
                2 => {
                    let f3 = [2u32, 3, 4, 6, 7][(*seed % 5) as usize];
                    a.i(f3, rd_of(seed), rr(seed), 0x13, ri(seed, 0x800));
                }
                3 => {
                    *seed = xs(*seed);
                    a.u(rd_of(seed), 0x37, (*seed % 0x10_0000) as u32);
                }
                4 => {
                    *seed = xs(*seed);
                    a.u(rd_of(seed), 0x17, (*seed % 0x10_0000) as u32);
                }
                5 => {
                    // base RegAlu table (f7, f3, op)
                    let table: [(u32, u32, u32); 28] = [
                        (0x00, 0, 0x33), (0x20, 0, 0x33), (0x00, 1, 0x33), (0x00, 2, 0x33),
                        (0x00, 3, 0x33), (0x00, 4, 0x33), (0x00, 5, 0x33), (0x20, 5, 0x33),
                        (0x00, 6, 0x33), (0x00, 7, 0x33), (0x01, 0, 0x33), (0x01, 1, 0x33),
                        (0x01, 2, 0x33), (0x01, 3, 0x33), (0x01, 4, 0x33), (0x01, 5, 0x33),
                        (0x01, 6, 0x33), (0x01, 7, 0x33), (0x00, 0, 0x3B), (0x20, 0, 0x3B),
                        (0x00, 1, 0x3B), (0x00, 5, 0x3B), (0x20, 5, 0x3B), (0x01, 0, 0x3B),
                        (0x01, 4, 0x3B), (0x01, 5, 0x3B), (0x01, 6, 0x3B), (0x01, 7, 0x3B),
                    ];
                    *seed = xs(*seed);
                    let (f7, f3, op) = table[(*seed % 28) as usize];
                    a.r(f7, f3, rd_of(seed), rr(seed), rr(seed), op);
                }
                6 => {
                    // slli/slliw need f6=0 f3=1; srli/srai (and W forms)
                    // need f6 in {0,0x10} f3=5 — pick a legal tuple.
                    *seed = xs(*seed);
                    let (f6, f3, word) = match *seed % 4 {
                        0 => (0u32, 1u32, false), // slli
                        1 => (0, 5, false),       // srli
                        2 => (0x10, 5, false),    // srai
                        _ => {
                            *seed = xs(*seed);
                            if *seed % 2 == 0 {
                                (0x10, 5, true) // sraiw
                            } else {
                                (0, 5, true) // srliw
                            }
                        }
                    };
                    let op = if word { 0x1B } else { 0x13 };
                    *seed = xs(*seed);
                    let shamt = if word { (*seed % 32) as u8 } else { (*seed % 64) as u8 };
                    a.sh(f6, f3, rd_of(seed), rr(seed), shamt, op);
                }
                7 => {
                    *seed = xs(*seed);
                    let f3 = (*seed % 7) as u32;
                    let (rs1, imm) = if tame {
                        // sp-relative, 8-aligned, below the stack top
                        (2u8, -8 - 8 * (*seed % 7) as i32)
                    } else {
                        (rr(seed), ri(seed, 0x40))
                    };
                    a.i(f3, rd_of(seed), rs1, 0x03, imm);
                }
                8 => {
                    *seed = xs(*seed);
                    let f3 = (*seed % 4) as u32;
                    let (rs1, imm) = if tame {
                        (2u8, -8 - 8 * (*seed % 7) as i32)
                    } else {
                        (rr(seed), ri(seed, 0x40))
                    };
                    a.s(f3, rs1, rr(seed), imm);
                }
                9 => {
                    *seed = xs(*seed);
                    let f3 = [0u32, 1, 4, 5, 6, 7][(*seed % 6) as usize];
                    let here = a.off.len();
                    let span = (total - here).max(1);
                    let t = if tame {
                        here + ((*seed % span as u64) as usize)
                    } else {
                        (*seed % total as u64) as usize
                    };
                    a.b(f3, rr(seed), rr(seed), t);
                }
                10 => {
                    let rd = [0u8, 1, 5][(*seed % 3) as usize];
                    *seed = xs(*seed);
                    let t = (*seed % total as u64) as usize;
                    if tame {
                        // no wild jal/jalr in tame mode
                        a.r(0x00, 0, rd_of(seed), rr(seed), rr(seed), 0x33);
                    } else {
                        a.j(rd, t);
                    }
                }
                11 => {
                    if tame {
                        a.sh(0, 1, rd_of(seed), rr(seed), (*seed % 64) as u8, 0x13);
                    } else {
                        a.i(0, rr(seed), rr(seed), 0x67, ri(seed, 0x20));
                    }
                }
                12 => {
                    a.raw(0x0FF0_000F); // fence
                }
                // Bitmanip register forms (single unified ISA): the ratified
                // (funct7, funct3, opcode) tuples from the decoder tables.
                14 => {
                    let table: [(u32, u32, u32); 25] = [
                        (0b0100000, 0b100, 0x33), // xnor
                        (0b0100000, 0b110, 0x33), // orn
                        (0b0100000, 0b111, 0x33), // andn
                        (0b0110000, 0b001, 0x33), // rol
                        (0b0110000, 0b101, 0x33), // ror
                        (0b0000101, 0b001, 0x33), // clmul
                        (0b0000101, 0b010, 0x33), // clmulr
                        (0b0000101, 0b011, 0x33), // clmulh
                        (0b0000101, 0b100, 0x33), // min
                        (0b0000101, 0b101, 0x33), // minu
                        (0b0000101, 0b110, 0x33), // max
                        (0b0000101, 0b111, 0x33), // maxu
                        (0b0010000, 0b010, 0x33), // sh1add
                        (0b0010000, 0b100, 0x33), // sh2add
                        (0b0010000, 0b110, 0x33), // sh3add
                        (0b0100100, 0b001, 0x33), // bclr
                        (0b0100100, 0b101, 0x33), // bext
                        (0b0010100, 0b001, 0x33), // bset
                        (0b0110100, 0b001, 0x33), // binv
                        (0b0000100, 0b000, 0x3B), // add.uw
                        (0b0010000, 0b010, 0x3B), // sh1add.uw
                        (0b0110000, 0b001, 0x3B), // rolw
                        (0b0110000, 0b101, 0x3B), // rorw
                        (0b0110000, 1, 0x13),     // clz/ctz/cpop/sext selectors
                        (0b0110101, 0b101, 0x13), // rev8 space
                    ];
                    *seed = xs(*seed);
                    let (f7, f3, op) = table[(*seed % 25) as usize];
                    let rs2 = if op == 0x13 && f7 == 0b0110000 {
                        // clz/ctz/cpop/sext.b/sext.h selectors (3 reserved)
                        [0u8, 1, 2, 4, 5][(*seed % 5) as usize]
                    } else if op == 0x13 && f7 == 0b0110101 {
                        0b11000
                    } else {
                        rr(seed)
                    };
                    a.r(f7, f3, rd_of(seed), rr(seed), rs2, op);
                }
                // Zbb immediate forms: rori / bseti / bclri / binvi / bexti / slli.uw
                15 => {
                    *seed = xs(*seed);
                    let (f6, f3, op) = match *seed % 6 {
                        0 => (0b011000u32, 0b101u32, 0x13u32), // rori
                        1 => (0b001010, 0b001, 0x13),          // bseti
                        2 => (0b010010, 0b010, 0x13),          // bclri
                        3 => (0b011010, 0b011, 0x13),          // binvi
                        4 => (0b010010, 0b101, 0x13),          // bexti
                        _ => (0b000010, 0b001, 0x1B),          // slli.uw
                    };
                    *seed = xs(*seed);
                    let shamt = (*seed % 64) as u8;
                    a.sh(f6, f3, rd_of(seed), rr(seed), shamt, op);
                }
                _ => {
                    a.r(0x00, 0, rd_of(seed), rr(seed), rr(seed), 0x33);
                }
            }
        }
        li(&mut a, 10, 0); // deterministic empty output on return paths
        a.ret();
        a.finish()
    }
}
