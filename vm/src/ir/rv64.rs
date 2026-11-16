//! Prototype IR→RV64IM/C lowering — the "fitsh kernel" feasibility probe.
//!
//! Purpose: measure whether compiling the pure-compute subset of the fitsh IR
//! tree directly to RV64IM machine code produces images whose (size, cycles)
//! profile beats the same IR compiled to HVM bytecode, and by how much. This
//! is a *measurement prototype*, not a consensus component:
//!
//! * Input is the same `IRNodeArray` the HVM backend consumes
//!   (`fitshc::compile_body`), so both channels compile identical IR.
//! * Output is a raw instruction stream plus a trailing literal pool, shaped
//!   for `calc::pack_container` as RX code / R pool / RW arena-bss segments
//!   (see `Rv64Lowered::pack_segments`).
//! * Three lowering modes (`Rv64Mode`): `Im` = profile 0, fixed 4-byte words;
//!   `Imc` = profile 2, RVC compressed encodings where the operands admit
//!   them (emit-time decision, consensus `decode_profile_c` forms only);
//!   `ImcRemap` = `Imc` plus the value pool remapped into the compressed
//!   register window (a0..a5 = x10..x15 first, loop scratch moved to t0/t1).
//! * Every emitted word is re-validated through the consensus decoder
//!   (`calc::decode::decode_at` with the mode's profile) before stats are
//!   reported, so a broken encoder cannot produce plausible-looking numbers.
//!
//! ## Codegen model (one pass, promotion-driven)
//!
//! * Expression-tree walk over a *virtual register stack* of location-
//!   tagged values (`VLoc`): an owned pool register, or a read-only
//!   *reference* to a bank register (a promoted local or the input
//!   descriptor in s4/s5 — operand-position reads cost zero bytes). The
//!   pool size adapts to the body's measured peak expression depth (a dry
//!   probe pass); in `ImcRemap` the freed registers — the compressed-
//!   window pair x14/x15 plus gp/tp/t2/a6/s6..s11 — become a flat local
//!   bank: hottest slots (by static access count) live in registers for
//!   the whole function (the register IS the slot; nothing else reads the
//!   frame). Nesting deeper than 8 is a hard subset limit. No SSA, no
//!   interference graph — the bank is flat, so no writebacks exist.
//! * Branches fuse comparisons (`if x == K` → `addi t, x, -K; c.beqz`,
//!   `while i < n` → `bltu`), else-less ifs drop their dead join jump,
//!   and every branch/jump is emitted as the smallest encoding that
//!   survives relaxation (c.bz/c.j tried optimistically, widened by the
//!   iterative finish-retry ladder — the same strategy as LLVM's RISC-V
//!   assembler backend).
//! * Constant arithmetic absorbs what the constant proves: `i += 1` →
//!   `c.addi; c.beqz fault`, `/16` → `c.srli`, `%16` → `c.andi`,
//!   `x * 256` → `c.slli` + high-bits check, constant divisors lose their
//!   zero-guard. The un-promoted frame is laid out hottest-first so the
//!   most-accessed slots stay inside the c.ld/c.sd ±248 B window, and a
//!   pool-base register (when ≥4 literal sites pay for it) turns each
//!   literal reference into a 2-byte `c.addi`.
//! * Statement runs of `if x == K { slot = V }` (≥4 arms, dense keys,
//!   literal default predecessor) collapse into one bounds-checked
//!   byte-table lookup in the literal pool — HIP5's 16-arm letter tables
//!   become `addi; c.li; bgeu; addi; c.add; lbu` instead of per-arm
//!   compare+branch chains.
//! * Bank segments cross loop backedges: a segment whose reads sit inside
//!   a loop it was not written in extends to that loop's end (the value is
//!   re-read every iteration); write-and-read-in-the-same-innermost-loop
//!   segments stay linear (each iteration re-writes first). The Lower
//!   cursor advances by `stmt_walk_count` — a mirror of the walk's
//!   per-statement counting — and `home_at` distinguishes read-side
//!   (boundary-exclusive) from write-side (inclusive) segment lookup,
//!   because a read at a write's own statement index evaluates first.
//!   `RV64_INV_CHECK=1 cargo test` runs a segment-coverage invariant
//!   checker over every banked access (this caught all four integration
//!   bugs listed in the report).
//! * Integers are u64 in one slot. HVM checked-arithmetic semantics are kept:
//!   overflow / division-by-zero / narrowing-cast overflow trap (guest HALT
//!   with the HVM error code, e.g. `Arithmetic = 90`), through shared one-word
//!   fault thunks.
//! * Bytes values are `{ptr, len}` descriptor pairs (two slots). `buf_cut`
//!   family ops are zero-copy descriptor arithmetic; `++` (CAT) and
//!   `as bytes` (CBYTES) bump-allocate in a fixed RW scratch segment.
//! * Locals (`var`/`let`, `PUT`/`GET`/`+=`) live in an 8/16-byte typed frame
//!   below the entry `sp`; slot types are inferred in a pre-pass. Slots whose
//!   first touch is a write outside any conditional skip zeroing.
//! * `require/assert/throw` and every checked fault jump to one shared trap
//!   epilogue (`li a1,0; li a7,0; ecall` → HALT, exit code in `a0`).
//!   `return <bytes>` copies to the output region, `a0 = len`, `ret` (entry
//!   `ra` is the HALT trampoline). `end` returns an empty output.
//! * The kernel's `data: bytes` parameter is slot 0, read directly from the
//!   ABI input registers (`s4`/`s5`); the prelude `PUT 0 (ROLL0)` binding is
//!   consumed, not evaluated.
//!
//! ## Pinned translatable subset (everything else errors at compile time)
//!
//! * literals/casts: `P0..P3, PU8, PU16, PBUF, PBUFL, PNBUF, PTRUE, PFALSE,
//!   CU8, CU16, CU32, CU64, CBYTES` (`CU128` rejected).
//! * ints: `ADD SUB MUL DIV MOD POW MAX MIN CLAMP ABSDIFF DIVUP DIVEXACT`
//!   (checked), `BSHL BSHR BXOR BOR BAND` (shift masked to 63), `EQ..GE`,
//!   `NOT, AND, OR, CHOOSE`.
//! * bytes: `CAT BYTE CUT LEFT RIGHT LDROP RDROP SIZE`.
//! * locals: `GET GET0..GET3 PUT XOP(+= -= *= /=)`.
//! * control: `IRLIST IRBLOCK IRBLOCKR IRIF IRIFR IRWHILE IRBREAK IRCONTINUE,
//!   REQ AST ERR ABT RET END NOP`, plus the frame prelude `ALLOC`/`POP`.
//! * rejected with a compile error: u128, compo/address/nil values, every
//!   native/action/storage/status/heap/global/log/call opcode, `bytecode {}`
//!   splices, `XLG`/`GETX`/`PUTX`/`ALLOC`-dynamic slots, the row-C
//!   mul-pipeline ops, `sqrt`/`sqrt_up`, stack shuffles.
//!
//! ## Documented deviations from exact HVM semantics
//!
//! All integer widths are widened to u64 (HVM keeps u8..u128 dynamic widths
//! and widens on arithmetic); shift amounts are masked to 63 instead of
//! faulting; `**` with a constant exponent unrolls to checked muls;
//! `x as bytes` is always the 8-byte big-endian u64 view (the score_sum wire
//! format) where HVM is width-aware; comparisons are unsigned 64-bit (kernel
//! ints are non-negative).
//!
//! ## Hard-won invariants (each one was a real bug; see the .loc report)
//!
//! 1. Encode nothing that the consensus decoder hasn't round-tripped. Two
//!    separate bit-spill bugs lived in comparison lowering for weeks: sltu
//!    funct3 3 written as decimal 11, and xori funct3 4 written as 12 —
//!    both leak a bit into the rs1 field, and both passed every naive test
//!    by register/value coincidence. `assembler_encodings_decode` /
//!    `rvc_encodings_decode` pin exact register fields; the HIP5 scorer was
//!    the workload that finally tripped them.
//! 2. Operand order is part of the encoding: `LE` computed `sltu d, rhs`
//!    instead of `sltu rhs, d` and behaved as GE. Same masking-by-coincidence.
//! 3. Scratch registers must never alias the value pool, and ABI registers
//!    must never double as loop scratch (the `ImcRemap` split — pool on
//!    a0..a5, loop scratch on t0/t1, ABI writes only at exit — encodes this).
//! 4. Branch displacement is discoverable only at fixup time: forward
//!    conditional branches past ±4 KiB need the trampoline form, discovered
//!    by `finish` and retried per-site (`lower_body_mode` relaxation loop).
//! 5. A bump arena with no reclamation bounds the kernel by *total* bytes
//!    churn, not peak use — the HIP5 digit-string building allocates
//!    ~dn(dn+1)/2 per record.

use std::collections::HashMap;
use std::collections::HashSet;

use super::{IRNode, IRNodeArray, IRNodeDouble, IRNodeEmpty, IRNodeLeaf, IRNodeParam1, IRNodeParam1Single, IRNodeParam2, IRNodeParam2Single, IRNodeParams, IRNodeParamsSingle, IRNodeSingle, IRNodeText, IRNodeTriple, IRNodeWrapOne};
use crate::calc::container::{self, CalcSegment, CODE_BASE};
use crate::calc::decode::{decode_at, inst_cycles, Inst};
use crate::rt::{decode_local_operand_mark, Bytecode, GasTable, LxOp, SpaceCap, VmrtRes};
use sys::{errf, Ret};

/* ------------------------------------------------------------------ */
/* Registers and memory plan                                           */
/* ------------------------------------------------------------------ */

const ZERO: u8 = 0;
const RA: u8 = 1;
const SP: u8 = 2;
const T0: u8 = 5;
const T1: u8 = 6;
const T2: u8 = 7;
const T3: u8 = 28;
const T4: u8 = 29;
const T5: u8 = 30;
const T6: u8 = 31;
const S0: u8 = 8; // locals frame base
const S1: u8 = 9; // arena bump pointer
const S2: u8 = 18; // output_base (from a2)
const S3: u8 = 19; // output_max (from a3)
const S4: u8 = 20; // input_base (from a0)
const S5: u8 = 21; // input_len (from a1)
const A0: u8 = 10; // scratch; exit code / return length at exit
const A1: u8 = 11; // scratch
const A7: u8 = 17; // ecall number (never a general register)

/// Virtual register stack. Kept at 8 so 15/16 stay dedicated scratch: values
/// popped-but-still-live must never be clobbered by address temporaries.
const VSTACK_REGS: [u8; 8] = [T0, T1, T2, T3, T4, T5, T6, 14];
/// Dedicated scratch, outside the value stack.
const SC1: u8 = 15;
const SC2: u8 = 16;

/// Imc-mode scratch pair. The RVC scratch constants SC1C/SC2C (x30/x31)
/// are safe for ImcRemap only: its pool (VSTACK_ORDER_RVC) excludes them.
/// Imc shares Im's pool (VSTACK_REGS = x5,x6,x7,x28,x29,x30,x31,x14), so
/// x30/x31 are pool positions 5/6 there — any sc1-scoped helper (the
/// BUG-012 Nil tag/flag loads, `li`'s temp) clobbered a live vstack value
/// at depth 6+ (BUG-024, P2-C4 S2: `byte(data, t1 % 40)` at pool depth 5
/// read the slot's Nil FLAG (1) instead of t1 (4) — Imc returned
/// 18|flag=19 where interp/Im/ImcRemap returned 18|t1=22). x12/x13 are
/// outside VSTACK_REGS, outside the Imc (a0,a1) = (x10,x11) pair, and the
/// prologue already saves them to x18/x19 before the body runs.
const SC1I: u8 = 12;
const SC2I: u8 = 13;

/// RVC-mode value pool ORDER: the compressed-window regs x14/x15 come
/// LAST so a shrinking pool (see `lower_body_mode_arena`) releases them
/// first into the local bank. With a full 8-slot pool this is the same
/// mapping as before: a0..a5 in the window, t3/t4 outside.
const VSTACK_ORDER_RVC: [u8; 8] = [A0, A1, 12, 13, T3, T4, 14, 15];
/// Free registers (ImcRemap) outside the value pool, never ABI-bound:
/// gp(x3)/tp(x4) — untouched by the self-contained kernel ABI — plus
/// t2(x7), a6(x16), s6..s11 (x22..x27). The bank prefers the compressed
/// window pair (x14/x15) first, then these.
const BANK_NONWINDOW: [u8; 10] = [3, 4, 7, 16, 22, 23, 24, 25, 26, 27];

/// Where a vstack value lives: an owned pool register, or a reference to a
/// bank (promoted-local) register. Refs are read-only for expression code
/// (only PUT/XOP write bank regs, after the rhs evaluation completes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VLoc {
    Pool(u8),
    Ref(u8),
}

fn loc_reg(l: VLoc) -> u8 {
    match l {
        VLoc::Pool(r) | VLoc::Ref(r) => r,
    }
}

/// Dedicated scratch in RVC mode (t5/t6, outside the window).
const SC1C: u8 = 30;
const SC2C: u8 = 31;

/// `wide_c_ids` namespace split: c.j relaxation ids (from `jal_seq`) are
/// tagged with this bit so they can never collide with c.bz ids (from
/// `branch_seq`). Both counters are far below 2^31 in any real kernel.
const JAL_C_ID_BIT: usize = 0x8000_0000;

/// Diagnostics-only env knobs (`RV64_NO_FWD` / `RV64_NO_DSE` /
/// `RV64_EV_DEBUG`): compiled out of release consensus products so no
/// environment variable can change emitted bytes (D-04 R1). Test binaries
/// and debug builds keep them; `RV64_INV_CHECK` deliberately stays
/// unconditional — it only panics on a violated invariant, it never
/// changes output.
fn diag_env_enabled(name: &str) -> bool {
    #[cfg(any(test, debug_assertions))]
    {
        std::env::var(name).is_ok()
    }
    #[cfg(not(any(test, debug_assertions)))]
    {
        let _ = name;
        false
    }
}

/// Runtime forwarding kill-switch for differential diagnostics (set from
/// `RV64_NO_FWD` at first use; tests flip it in-process).
static FWD_DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// When Some(slot), only this slot's forwarding stays enabled (per-slot
/// isolation diagnostics).
static FWD_ONLY_SLOT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
static FWD_ENV_READ: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Lowering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rv64Mode {
    /// Plain RV64IM (profile 0).
    Im,
    /// RV64IMC (profile 2): compressed encodings, same register map.
    Imc,
    /// RV64IMC + value pool remapped into the RVC window.
    ImcRemap,
}

/// Fixed RW scratch segment (`.bss`) for the bytes arena, declared by
/// `Rv64Lowered::pack_segments`. The exhaustion check is
/// `li end + add + bgeu` against the arena end.
/// The bump arena has no reclamation: every `++` / `as bytes` allocation
/// is live for the whole run, so workloads with heavy bytes churn (e.g.
/// digit-string building per record) bound the arena by total churn, not
/// peak use — a production backend wants GC or per-slot regions here
/// (see report §limitations). The maximum the L1 layout admits (mem_kib
/// 64, io 4800+8, 4 KiB stack band) is 0x9D30; the default fits the
/// naive-bench packing (io 5120+1280) inside mem 64.
pub const ARENA_BASE: u64 = 0x5000;
/// Default arena: 12 KiB, ends at 0x8000.
pub const ARENA_SIZE: u64 = 0x3000;
/// Maximum arena under the L1 64 KiB cap with io 4800+8: ends at 0xED30.
pub const ARENA_SIZE_MAX: u64 = 0x9D30;

const EXIT_ARITH: u64 = 90; // ItrErrCode::Arithmetic
const EXIT_CAST: u64 = 71; // ItrErrCode::CastFail

/* ------------------------------------------------------------------ */
/* Assembler                                                           */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq)]
enum FixKind {
    Branch,
    Jal,
    /// 2-byte c.beqz/c.bnez slot; relaxed to a full Branch on overflow.
    CBranchZ,
    /// 2-byte c.j slot; relaxed to a full Jal on overflow.
    CJ,
    /// `auipc rd, hi; addi rd, rd, lo` pair loading a label address.
    PcRelHiLo,
    /// `addi rd, base, off` — rd = (base label address) + (lit label offset),
    /// patched once both labels resolve (the pool-base rematerialization).
    PoolOff,
    /// `ld rd, off(base)` — off = lit label − pool-base label (a pooled
    /// constant materialization; the base register holds the pool start).
    PoolLd,
}

struct Fixup {
    at: usize,
    label: usize,
    kind: FixKind,
    /// Branch emission sequence id, used to re-target the sites that need
    /// trampolines in the next relaxation pass (see `branch`).
    seq: usize,
    /// Secondary label (the base for PoolOff).
    aux: usize,
}

/// Minimal two-pass assembler with labels. Encoding paths are pinned by the
/// `assembler_encodings_decode` test through the consensus decoder.
struct Asm {
    buf: Vec<u8>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
    /// Emit RVC (C-extension) encodings where the operands admit them.
    rvc: bool,
    /// Dedicated scratch registers (outside the value pool); mode-dependent.
    sc1: u8,
    sc2: u8,
    /// Monotonic id per conditional-branch emission; stable across passes
    /// because lowering is deterministic.
    branch_seq: usize,
    /// Branch sites (by seq id) that resolved out of the ±4 KiB B-type range
    /// in a previous relaxation pass and must use the jal-trampoline form.
    long_branch_ids: std::collections::HashSet<usize>,
    /// c.bz/c.j sites (by seq id) that did not fit the 2-byte ±256 B / ±2 KiB
    /// windows in a previous pass and must be emitted full-width (the
    /// "compress optimistically, relax on overflow" strategy — same ladder
    /// as LLVM's RISCVAsmBackend c.beqz→beq→inversion).
    wide_c_ids: std::collections::HashSet<usize>,
    /// Monotonic id per jal emission (parallel to branch_seq).
    jal_seq: usize,
}

/// `finish` failure: a plain message, or the sites to relax and retry.
#[derive(Debug)]
enum FinishErr {
    Msg(String),
    /// 4-byte conditional branches out of the ±4 KiB B-type range: retry
    /// those sites in the jal-trampoline form.
    BranchOverflow(Vec<usize>),
    /// c.bz / c.j sites out of their ±256 B / ±2 KiB windows: retry those
    /// sites full-width.
    WideCOverflow(Vec<usize>),
}

/// RVC compressed-register window (c.ld/c.sd/c.beqz/c.bnez and the
/// quadrant-1 RR arithmetic only accept x8..x15).
fn rvc_win(r: u8) -> bool {
    (8..=15).contains(&r)
}

impl Asm {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            rvc: false,
            sc1: SC1,
            sc2: SC2,
            branch_seq: 0,
            long_branch_ids: Default::default(),
            wide_c_ids: Default::default(),
            jal_seq: 0,
        }
    }
    fn new_rvc() -> Self {
        // RVC mode also moves the value pool into the compressed window and
        // the scratch registers out of its way (see Rv64Mode::ImcRemap).
        Self::new_rvc_scratch(SC1C, SC2C)
    }
    /// RVC assembler with an explicit scratch pair: ImcRemap keeps x30/x31
    /// (outside VSTACK_ORDER_RVC); Imc must take SC1I/SC2I because its pool
    /// is Im's VSTACK_REGS, which contains x30/x31 (BUG-024).
    fn new_rvc_scratch(sc1: u8, sc2: u8) -> Self {
        Self {
            buf: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            rvc: true,
            sc1,
            sc2,
            branch_seq: 0,
            long_branch_ids: Default::default(),
            wide_c_ids: Default::default(),
            jal_seq: 0,
        }
    }
    fn here(&self) -> usize {
        self.buf.len()
    }
    fn new_label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }
    fn place(&mut self, label: usize) {
        assert!(self.labels[label].is_none(), "label {label} placed twice");
        self.labels[label] = Some(self.buf.len());
    }
    fn word(&mut self, w: u32) {
        self.buf.extend_from_slice(&w.to_le_bytes());
    }

    /* ---- compressed emitters (consensus decode_profile_c forms) ---- */

    fn half(&mut self, h: u16) {
        self.buf.extend_from_slice(&h.to_le_bytes());
    }

    /// Quadrant-2 CI-ish: rd[11:7] full 5-bit, rs2[6:2] full 5-bit.
    /// c.mv(f3=4,b12=0,rs2!=0) / c.add(f3=4,b12=1,rs2!=0) / c.slli(f3=0,
    /// shamt in rs2) / c.jr(f3=4,b12=0,rs2=0) / c.jalr(f3=4,b12=1,rs2=0).
    fn c_q2(f3: u32, bit12: u32, rd: u8, rs2: u8) -> u16 {
        0b10 | ((rd as u16) << 7) | ((f3 as u16) << 13) | ((bit12 as u16) << 12) | ((rs2 as u16) << 2)
    }

    /// Quadrant-1 CI: rd[11:7], funct3[15:13], imm[5]=h12, imm[4:0]=h6:2.
    /// c.addi(f3=0) / c.li(f3=2) / c.lui(f3=3, rd!=2, imm!=0).
    fn c_q1_ci(f3: u32, rd: u8, imm: i32) -> u16 {
        let u = (imm as u16) & 0x3F;
        0b01
            | ((rd as u16) << 7)
            | ((f3 as u16) << 13)
            | ((u & 0x20) << 7)
            | ((u & 0x1F) << 2)
    }

    /// Quadrant-0 CL/CS: rd'/rs2' [4:2]+8, rs1' [9:7]+8,
    /// uimm = (h12:10) | (h6:5) << 3, byte offset = uimm * 8.
    fn c_q0_ldst(f3: u32, r01: u8, base: u8, imm: i32) -> u16 {
        let u = (imm >> 3) as u16; // 0..=31
        0b00
            | ((f3 as u16) << 13)
            | (((u & 7) as u16) << 10)
            | (((base & 7) as u16) << 7)
            | (((r01 & 7) as u16) << 2)
            | (((u >> 3) as u16) << 5)
    }

    /// c.j: 11-bit signed offset, bit-scattered per decode_profile_c.
    fn c_j(off: i32) -> u16 {
        let o = (off as u16) & 0xFFF; // 12-bit imm, sign bit is imm[11]
        0b01
            | (5 << 13)
            | ((o & 0x800) << 1)
            | (((o >> 4) & 1) << 11)
            | (((o >> 8) & 0x3) << 9)
            | (((o >> 10) & 1) << 8)
            | (((o >> 6) & 1) << 7)
            | (((o >> 7) & 1) << 6)
            | (((o >> 1) & 0x7) << 3)
            | (((o >> 5) & 1) << 2)
    }

    /// c.beqz/c.bnez: rs1' [9:7]+8, 9-bit signed offset (±256 B).
    fn c_bz(f3: u32, rs1: u8, off: i32) -> u16 {
        let o = (off as u16) & 0x1FF; // 9-bit imm, sign bit imm[8] -> h12
        0b01
            | ((f3 as u16) << 13)
            | ((o & 0x100) << 4)
            | (((o >> 3) & 0x3) << 10)
            | (((o >> 6) & 0x3) << 5)
            | (((o >> 1) & 0x3) << 3)
            | (((o >> 5) & 1) << 2)
            | (((rs1 & 7) as u16) << 7)
    }

    // branch funct3: BEQ=0 BNE=1 BLT=4 BGE=5 BLTU=6 BGEU=7
    fn branch_word(f3: u32, rs1: u8, rs2: u8, imm: u32) -> u32 {
        0b1100011
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (f3 << 12)
            | (((imm >> 12) & 1) << 31)
            | (((imm >> 5) & 0x3F) << 25)
            | (((imm >> 1) & 0xF) << 8)
            | (((imm >> 11) & 1) << 7)
    }

    fn jal_word(rd: u8, imm: u32) -> u32 {
        0b1101111
            | ((rd as u32) << 7)
            | (((imm >> 1) & 0x3FF) << 21)
            | (((imm >> 11) & 1) << 20)
            | (((imm >> 12) & 0xFF) << 12)
            | (((imm >> 20) & 1) << 31)
    }

    /// Conditional branch, B-type ±4 KiB. Targets beyond that range use a
    /// trampoline: the inverted conditional skips a full-range `jal`.
    /// Which sites need the trampoline is discovered by `finish` and fed
    /// back through `long_branch_ids` (iterative relaxation; lowering is
    /// deterministic so seq ids are stable across passes).
    fn branch(&mut self, f3: u32, rs1: u8, rs2: u8, label: usize) {
        let seq = self.branch_seq;
        self.branch_seq += 1;
        let long = self.long_branch_ids.contains(&seq);
        // Zero-compare branches compress to c.beqz/c.bnez when rs1 is in the
        // window. Backward offsets resolve immediately; forward references
        // are emitted optimistically as a 2-byte slot and relaxed to the
        // full B-type form by `finish` when they miss the ±256 B window
        // (which in turn relaxes to the trampoline past ±4 KiB).
        if self.rvc && !long && rs2 == ZERO && rvc_win(rs1) && (f3 == 0 || f3 == 1) {
            let cf3 = if f3 == 0 { 6 } else { 7 }; // c.beqz=6, c.bnez=7
            if !self.wide_c_ids.contains(&seq) {
                let at = self.here();
                if let Some(t) = self.labels[label] {
                    let off = t as i64 - at as i64;
                    if (-256..=254).contains(&off) && off & 1 == 0 {
                        self.half(Self::c_bz(cf3, rs1, off as i32));
                        return;
                    }
                    if !(-4096..=4094).contains(&off) || off & 1 != 0 {
                        // far backward: trampoline right away
                        self.word(Self::branch_word(f3 ^ 1, rs1, rs2, 8));
                        self.word(Self::jal_word(ZERO, (t - (at + 4)) as u32));
                        return;
                    }
                    self.word(Self::branch_word(f3, rs1, rs2, off as u32));
                    return;
                }
                self.half(Self::c_bz(cf3, rs1, 0));
                self.fixups.push(Fixup { at, label, kind: FixKind::CBranchZ, seq, aux: 0 });
                return;
            }
        }
        let at = self.here();
        if !long {
            if let Some(t) = self.labels[label] {
                // backward: direct when in range
                let off = t as i64 - at as i64;
                if (-4096..=4094).contains(&off) && off & 1 == 0 {
                    self.word(Self::branch_word(f3, rs1, rs2, off as u32));
                    return;
                }
            }
            self.word(Self::branch_word(f3, rs1, rs2, 0));
            self.fixups.push(Fixup { at, label, kind: FixKind::Branch, seq, aux: 0 });
            return;
        }
        // trampoline: inverted conditional skips the long jal
        self.word(Self::branch_word(f3 ^ 1, rs1, rs2, 8));
        if let Some(t) = self.labels[label] {
            let off = t - (at + 4);
            self.word(Self::jal_word(ZERO, off as u32));
        } else {
            self.word(Self::jal_word(ZERO, 0));
            self.fixups.push(Fixup { at: at + 4, label, kind: FixKind::Jal, seq, aux: 0 });
        }
    }
    fn beq(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(0, rs1, rs2, l);
    }
    fn bne(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(1, rs1, rs2, l);
    }
    fn bgeu(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(7, rs1, rs2, l);
    }
    fn bltu(&mut self, rs1: u8, rs2: u8, l: usize) {
        self.branch(6, rs1, rs2, l);
    }

    fn jal_(&mut self, rd: u8, label: usize) {
        let seq = self.jal_seq;
        self.jal_seq += 1;
        // c.j relaxation ids live in a namespace separate from branch ids
        // (both share `wide_c_ids`): a widened branch must never widen an
        // innocent c.j that happens to carry the same seq number — a real
        // size bug this exact sharing shipped for a while.
        let wseq = seq | JAL_C_ID_BIT;
        // rd=0 jumps compress to c.j (±2 KiB). Forward references are again
        // optimistic 2-byte slots relaxed by `finish`.
        if self.rvc && rd == ZERO && !self.wide_c_ids.contains(&wseq) {
            let at = self.here();
            if let Some(t) = self.labels[label] {
                let off = t as i64 - at as i64;
                if (-2048..=2046).contains(&off) && off & 1 == 0 {
                    self.half(Self::c_j(off as i32));
                    return;
                }
            } else {
                self.half(Self::c_j(0));
                self.fixups.push(Fixup { at, label, kind: FixKind::CJ, seq: wseq, aux: 0 });
                return;
            }
        }
        let at = self.here();
        self.word(Self::jal_word(rd, 0));
        self.fixups.push(Fixup { at, label, kind: FixKind::Jal, seq: usize::MAX, aux: 0 });
    }
    fn ret(&mut self) {
        if self.rvc {
            self.half(Self::c_q2(4, 0, RA, 0)); // c.jr ra (c.ret)
            return;
        }
        self.word(0b1100111u32 | ((RA as u32) << 15)); // jalr x0, ra, 0
    }
    fn ecall(&mut self) {
        self.word(0x0000_0073);
    }
    fn pcrel_addr(&mut self, rd: u8, label: usize) {
        let at = self.here();
        self.word(0b0010111u32 | ((rd as u32) << 7)); // auipc rd, 0 (patched)
        self.word(0b0010011u32 | ((rd as u32) << 7) | ((rd as u32) << 15)); // addi rd, rd, 0
        self.fixups.push(Fixup { at, label, kind: FixKind::PcRelHiLo, seq: usize::MAX, aux: 0 });
    }

    /// `addi rd, base, <lit - base>`: literal address relative to a
    /// function-wide pool-base register (2-byte c.addi when the offset is
    /// small), resolved by `finish` once the pool layout is known.
    fn addi_label_off(&mut self, rd: u8, base_reg: u8, base_label: usize, lit_label: usize) {
        let at = self.here();
        // addi rd, base, 0 (patched)
        self.word(0b0010011u32 | ((rd as u32) << 7) | ((base_reg as u32) << 15));
        self.fixups.push(Fixup {
            at,
            label: lit_label,
            kind: FixKind::PoolOff,
            seq: usize::MAX,
            aux: base_label,
        });
    }

    /// `ld rd, <lit - base>(base_reg)`: load a pooled 8-byte constant
    /// relative to the function-wide pool-base register.
    fn ld_label_off(&mut self, rd: u8, base_reg: u8, base_label: usize, lit_label: usize) {
        let at = self.here();
        // ld rd, 0(base_reg) (imm patched by finish)
        self.word(0b0000011u32 | ((rd as u32) << 7) | (3 << 12) | ((base_reg as u32) << 15));
        self.fixups.push(Fixup {
            at,
            label: lit_label,
            kind: FixKind::PoolLd,
            seq: usize::MAX,
            aux: base_label,
        });
    }

    fn r(&mut self, f7: u32, rs2: u8, rs1: u8, f3: u32, rd: u8, opcode: u32) {
        self.word(
            (f7 << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((rd as u32) << 7)
                | opcode,
        );
    }
    fn i(&mut self, imm: i32, rs1: u8, f3: u32, rd: u8, opcode: u32) {
        assert!(
            (-2048..=2047).contains(&imm),
            "rv64 proto: i-type imm {imm} out of range"
        );
        let imm = (imm as u32) & 0xFFF;
        self.word((imm << 20) | ((rs1 as u32) << 15) | (f3 << 12) | ((rd as u32) << 7) | opcode);
    }
    fn s(&mut self, imm: i32, rs2: u8, rs1: u8, f3: u32) {
        assert!(
            (-2048..=2047).contains(&imm),
            "rv64 proto: s-type imm {imm} out of range"
        );
        let imm = (imm as u32) & 0xFFF;
        self.word(
            ((imm >> 5) << 25)
                | ((rs2 as u32) << 20)
                | ((rs1 as u32) << 15)
                | (f3 << 12)
                | ((imm & 0x1F) << 7)
                | 0b0100011,
        );
    }
    fn addi(&mut self, rd: u8, rs1: u8, imm: i32) {
        if self.rvc && rd != ZERO && (-32..=31).contains(&imm) {
            if rs1 == ZERO {
                self.half(Self::c_q1_ci(2, rd, imm)); // c.li
                return;
            }
            if rd == rs1 {
                self.half(Self::c_q1_ci(0, rd, imm)); // c.addi
                return;
            }
        }
        self.i(imm, rs1, 0, rd, 0b0010011);
    }
    fn ld(&mut self, rd: u8, rs1: u8, imm: i32) {
        if self.rvc && rvc_win(rd) && rvc_win(rs1) && (0..=248).contains(&imm) && imm % 8 == 0 {
            self.half(Self::c_q0_ldst(3, rd, rs1, imm)); // c.ld
            return;
        }
        self.i(imm, rs1, 3, rd, 0b0000011);
    }
    /// P3-KERNELS M2-d: 4-byte-stride rodata list loads (u32-packed
    /// tables). lwu zero-extends, so a table whose values all fit u32
    /// loads identically to the u64 form. No RVC compressed form exists
    /// for lwu (c.lw is RV32-only) — always the 4B base encoding.
    fn lwu(&mut self, rd: u8, rs1: u8, imm: i32) {
        self.i(imm, rs1, 6, rd, 0b0000011);
    }
    /// P3-IDIOMS: 2-byte BE-assembly groups (hacd200 :629 shape). lhu
    /// ZERO-extends (f3=5 — f3=1 would be the sign-extending lh!). No
    /// RVC compressed form (c.lh does not exist in RVC) — always 4B.
    fn lhu(&mut self, rd: u8, rs1: u8, imm: i32) {
        self.i(imm, rs1, 5, rd, 0b0000011);
    }
    fn sd(&mut self, rs2: u8, rs1: u8, imm: i32) {
        if self.rvc && rvc_win(rs2) && rvc_win(rs1) && (0..=248).contains(&imm) && imm % 8 == 0 {
            self.half(Self::c_q0_ldst(7, rs2, rs1, imm)); // c.sd
            return;
        }
        self.s(imm, rs2, rs1, 3);
    }
    fn lbu(&mut self, rd: u8, rs1: u8, imm: i32) {
        self.i(imm, rs1, 4, rd, 0b0000011);
    }
    fn sb(&mut self, rs2: u8, rs1: u8, imm: i32) {
        self.s(imm, rs2, rs1, 0);
    }
    fn mv(&mut self, rd: u8, rs: u8) {
        if self.rvc && rd != ZERO && rs != ZERO {
            self.half(Self::c_q2(4, 0, rd, rs)); // c.mv
            return;
        }
        self.addi(rd, rs, 0);
    }
    fn add(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if self.rvc && rd == rs1 && rd != ZERO && rs2 != ZERO {
            self.half(Self::c_q2(4, 1, rd, rs2)); // c.add
            return;
        }
        self.r(0, rs2, rs1, 0, rd, 0b0110011);
    }
    /// RR arithmetic compressible as c.sub/c.xor/c.or/c.and (quadrant-1
    /// funct3=4, bits[11:10]=11; `sel`: 00=sub 01=xor 10=or 11=and).
    fn rr_win(&mut self, sel: u32, rd: u8, rs1: u8, rs2: u8) -> bool {
        if self.rvc && rd == rs1 && rd != ZERO && rvc_win(rd) && rvc_win(rs2) {
            let h = 0b01u16
                | (4 << 13)
                | (0b11 << 10)
                | (((sel & 3) as u16) << 5)
                | (((rd & 7) as u16) << 7)
                | (((rs2 & 7) as u16) << 2);
            self.half(h);
            return true;
        }
        false
    }
    fn sub(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(0, rd, rs1, rs2) {
            self.r(0x20, rs2, rs1, 0, rd, 0b0110011);
        }
    }
    fn xor(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(1, rd, rs1, rs2) {
            self.r(0, rs2, rs1, 4, rd, 0b0110011);
        }
    }
    fn or_(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(2, rd, rs1, rs2) {
            self.r(0, rs2, rs1, 6, rd, 0b0110011);
        }
    }
    fn and_(&mut self, rd: u8, rs1: u8, rs2: u8) {
        if !self.rr_win(3, rd, rs1, rs2) {
            self.r(0, rs2, rs1, 7, rd, 0b0110011);
        }
    }
    /// andi (c.andi when rd is in the window and imm fits imm6).
    fn andi(&mut self, rd: u8, rs1: u8, imm: i32) {
        if self.rvc && rd == rs1 && rvc_win(rd) && (-32..=31).contains(&imm) {
            // quadrant-1 CB: f3=4, bits[11:10]=10, imm6 scattered like c.andi
            let u = (imm as u16) & 0x3F;
            let h = 0b01u16
                | (4 << 13)
                | (0b10 << 10)
                | ((u & 0x20) << 7)
                | ((u & 0x1F) << 2)
                | (((rd & 7) as u16) << 7);
            self.half(h);
            return;
        }
        self.i(imm, rs1, 7, rd, 0b0010011);
    }

    fn slli_(&mut self, rd: u8, rs1: u8, shamt: u32) {
        if self.rvc && rd == rs1 && rd != ZERO && (1..=63).contains(&shamt) {
            self.half(Self::c_q2(0, shamt >> 5, rd, shamt as u8 & 0x1F)); // c.slli
            return;
        }
        self.word(
            (shamt << 20) | ((rs1 as u32) << 15) | (1 << 12) | ((rd as u32) << 7) | 0b0010011,
        );
    }
    /// srli by immediate (c.srli when rd is in the window).
    fn srli_(&mut self, rd: u8, rs1: u8, shamt: u32) {
        if self.rvc && rd == rs1 && rvc_win(rd) && (1..=63).contains(&shamt) {
            let h = 0b01u16
                | (4 << 13)
                | (0b00 << 10)
                | (((shamt & 0x1F) as u16) << 2)
                | (((shamt >> 5) as u16) << 12)
                | (((rd & 7) as u16) << 7);
            self.half(h); // c.srli
            return;
        }
        self.word(
            (shamt << 20) | ((rs1 as u32) << 15) | (5 << 12) | ((rd as u32) << 7) | 0b0010011,
        );
    }

    /// Load a constant into `rd` as a positive 64-bit value.
    fn li(&mut self, rd: u8, v: u64) {
        assert!(rd != ZERO, "li into x0");
        if v > 0xFFFF_FFFF {
            // 64-bit constant: hi half shifted up, or the low half in.
            let hi32 = v >> 32;
            let lo32 = v & 0xFFFF_FFFF;
            self.li_pair(rd, hi32);
            self.slli_(rd, rd, 32);
            if lo32 != 0 {
                self.li(self.sc2, lo32);
                self.r(0, self.sc2, rd, 6, rd, 0b0110011); // or rd, rd, sc2
            }
            return;
        }
        if v <= 2047 {
            self.addi(rd, ZERO, v as i32);
            return;
        }
        let hi = ((v + 0x800) >> 12) as u32;
        if hi < 0x8_0000 {
            self.li_pair(rd, v);
            return;
        }
        // v ≥ 0x7FFF_F800 (rounded hi would overflow 20 bits): two 16-bit
        // halves; the temp must differ from the destination.
        let lo16 = (v & 0xFFFF) as u64;
        let hi16 = v >> 16;
        let tmp = if rd == self.sc1 { self.sc2 } else { self.sc1 };
        self.li_pair(rd, hi16);
        self.slli_(rd, rd, 16);
        match lo16 {
            0 => {}
            0..=2047 => self.i(lo16 as i32, rd, 6, rd, 0b0010011), // ori
            _ => {
                self.li_pair(tmp, lo16);
                self.r(0, tmp, rd, 6, rd, 0b0110011); // or rd, rd, tmp
            }
        }
    }

    /// lui+addi pair for constants whose rounded hi fits 20 bits (< 2^31).
    fn li_pair(&mut self, rd: u8, v: u64) {
        let hi = ((v + 0x800) >> 12) as u32;
        let lo_i = v as i64 - ((hi as u64) << 12) as i64; // [-2048, 2047]
        if hi != 0 {
            if self.rvc && rd != ZERO && rd != SP && hi <= 31 {
                // c.lui (rd != x2, nonzero 6-bit signed hi; sext is positive
                // for hi <= 31 so the constant stays non-negative).
                self.half(Self::c_q1_ci(3, rd, hi as i32));
            } else {
                self.word((hi << 12) | ((rd as u32) << 7) | 0b0110111); // lui
            }
            if lo_i != 0 {
                self.addi(rd, rd, lo_i as i32);
            }
        } else {
            self.addi(rd, ZERO, lo_i as i32);
        }
    }

    fn finish(mut self) -> Result<Vec<u8>, FinishErr> {
        let mut overflow = Vec::new();
        let mut wide = Vec::new();
        for f in &self.fixups {
            // P3-GUARDTAIL GT-c: a never-placed label is a compiler bug —
            // surface it through the error channel instead of panicking
            // (doc 66 §1.4: user-reachable panics must not abort the
            // process; the message lands via the FinishErr::Msg arm at the
            // relaxation loop, which rethrows it with `rv64 lower:` intact)
            let target = self.labels[f.label].ok_or_else(|| {
                FinishErr::Msg(format!(
                    "rv64 lower: internal: fixup label {} never placed (compiler bug)",
                    f.label
                ))
            })?;
            let off = target as i64 - f.at as i64;
            // 2-byte slots first: relax to full-width on window overflow.
            match f.kind {
                FixKind::CBranchZ => {
                    if !(-256..=254).contains(&off) || off & 1 != 0 {
                        wide.push(f.seq);
                        continue;
                    }
                    let h = u16::from_le_bytes(self.buf[f.at..f.at + 2].try_into().unwrap());
                    let f3 = ((h >> 13) & 7) as u32;
                    let rs1 = (((h >> 7) & 7) + 8) as u8;
                    let patched = Self::c_bz(f3, rs1, off as i32);
                    self.buf[f.at..f.at + 2].copy_from_slice(&patched.to_le_bytes());
                    continue;
                }
                FixKind::CJ => {
                    if !(-2048..=2046).contains(&off) || off & 1 != 0 {
                        wide.push(f.seq);
                        continue;
                    }
                    let patched = Self::c_j(off as i32);
                    self.buf[f.at..f.at + 2].copy_from_slice(&patched.to_le_bytes());
                    continue;
                }
                _ => {}
            }
            let word = u32::from_le_bytes(self.buf[f.at..f.at + 4].try_into().unwrap());
            let patched = match f.kind {
                FixKind::Branch => {
                    if !(-4096..=4094).contains(&off) || off & 1 != 0 {
                        overflow.push(f.seq);
                        continue;
                    }
                    let imm = off as u32 & 0x1FFF;
                    word
                        | (((imm >> 12) & 1) << 31)
                        | (((imm >> 5) & 0x3F) << 25)
                        | (((imm >> 1) & 0xF) << 8)
                        | (((imm >> 11) & 1) << 7)
                }
                FixKind::Jal => {
                    if !(-0x10_0000..=0xF_FFFE).contains(&off) || off & 1 != 0 {
                        return Err(FinishErr::Msg(format!(
                            "rv64 proto: jal displacement {off} out of range (kernel body too large)"
                        )));
                    }
                    let imm = off as u32 & 0x1F_FFFF;
                    word
                        | (((imm >> 1) & 0x3FF) << 21)
                        | (((imm >> 11) & 1) << 20)
                        | (((imm >> 12) & 0xFF) << 12)
                        | (((imm >> 20) & 1) << 31)
                }
                FixKind::PcRelHiLo => {
                    let hi = ((off + 0x800) >> 12) as u32 & 0xFF_FFF;
                    let lo = off - ((hi as i64) << 12);
                    let rd = ((word >> 7) & 0x1F) as u32;
                    let w_lo = ((lo as u32 & 0xFFF) << 20) | (rd << 15) | (rd << 7) | 0b0010011;
                    self.buf[f.at + 4..f.at + 8].copy_from_slice(&w_lo.to_le_bytes());
                    (hi << 12) | (rd << 7) | 0b0010111
                }
                FixKind::PoolOff => {
                    // GT-c: same error-channel conversion as the fixup label
                    // above (user-reachable panic → Err, doc 66 §1.4)
                    let base = self.labels[f.aux].ok_or_else(|| {
                        FinishErr::Msg(
                            "rv64 lower: internal: pool base label never placed (compiler bug)"
                                .to_string(),
                        )
                    })?;
                    let rel = target as i64 - base as i64;
                    if !(-2048..=2047).contains(&rel) {
                        return Err(FinishErr::Msg(format!(
                            "rv64 proto: pool-base offset {rel} out of range"
                        )));
                    }
                    // clear ONLY the imm[11:0] field (bits 31:20); a mask
                    // reaching below bit 20 would wipe rs1/funct3 (a real bug
                    // this exact patch shipped for a few minutes)
                    word & !(0xFFFu32 << 20) | (((rel as u32) & 0xFFF) << 20)
                }
                FixKind::PoolLd => {
                    // GT-c: same error-channel conversion as PoolOff above
                    let base = self.labels[f.aux].ok_or_else(|| {
                        FinishErr::Msg(
                            "rv64 lower: internal: pool base label never placed (compiler bug)"
                                .to_string(),
                        )
                    })?;
                    let rel = target as i64 - base as i64;
                    if !(-2048..=2047).contains(&rel) {
                        return Err(FinishErr::Msg(format!(
                            "rv64 proto: pool-base constant offset {rel} out of range"
                        )));
                    }
                    word & !(0xFFFu32 << 20) | (((rel as u32) & 0xFFF) << 20)
                }
                FixKind::CBranchZ | FixKind::CJ => unreachable!("handled above"),
            };
            self.buf[f.at..f.at + 4].copy_from_slice(&patched.to_le_bytes());
        }
        if !wide.is_empty() {
            // Sizes only grow across passes, so escalating the c-slots first
            // is monotone and converges.
            return Err(FinishErr::WideCOverflow(wide));
        }
        if !overflow.is_empty() {
            return Err(FinishErr::BranchOverflow(overflow));
        }
        Ok(self.buf)
    }
}

/* ------------------------------------------------------------------ */
/* Types + pre-pass                                                    */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Int,
    Bytes,
    /// P3-V2IMPL (A-25): a compo-list VALUE under the admitted v2 shape set
    /// (doc 74 E1 / doc 73 E7). Never a vstack value — it only classifies
    /// plan()/node_res_ty expressions and slot states. Everything a list
    /// value would flow into outside the five admitted consumers is a
    /// compile-time reject (plan() is the single admit/reject source).
    List,
}

/// GT-c T8-style: human-readable slot type names for error messages (no
/// Debug leakage — the words match the fitsh surface: `int`, `bytes`,
/// the `data` parameter, lists).
fn slot_ty_name(t: SlotTy) -> &'static str {
    match t {
        SlotTy::None => "unbound",
        SlotTy::Int => "int",
        SlotTy::Bytes => "bytes",
        SlotTy::Input => "the input parameter (bytes)",
        SlotTy::List => "list",
    }
}

/// Same for the plan-side value classification ([`Ty`]).
fn ty_name(t: Ty) -> &'static str {
    match t {
        Ty::Int => "int",
        Ty::Bytes => "bytes",
        Ty::List => "list",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotTy {
    None,
    Int,
    Bytes,
    /// The kernel's single `data: bytes` parameter — lives in the ABI input
    /// registers (s4/s5), not in the locals frame.
    Input,
    /// P3-V2IMPL: a compo list under the admitted v2 shape set. The slot's
    /// own 8-byte frame cell is the length shadow (growable lists); the
    /// element data block is a separate frame region (`ListKind`). List
    /// slots never enter the register bank (the bank build matches
    /// `Int | Bytes` only) and never take part in forwarding/DSE — the web
    /// timeline records no events for them, and every generic PUT/XOP/GET
    /// path is intercepted for them by construction.
    List,
}

/// P3-V2IMPL: final storage decision for one list slot, decided by the
/// `plan()` walk (creation shape + APPEND/CLEAR reachability refinement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    NotList,
    /// `[]` / `new_list()` creation, never appended/cleared: len ≡ 0, no
    /// storage at all (any item_get faults 85 / folds false).
    Empty,
    /// All-literal `[lit0..litN-1]`, never appended/cleared: a read-only
    /// u64 table (N×8B, little-endian, 8-byte stride) in the literal pool /
    /// R segment. The creation site emits nothing.
    Rodata { n: u64 },
    /// Computed-element `[e0..eN-1]`, never appended/cleared: frame block
    /// of N×8B filled by per-element stores at creation; static length N.
    FrameStatic { n: u64 },
    /// APPEND/CLEAR reachable, or `[]`/`new_list()` with appends: frame
    /// block of 128×8B = 1024B (decision D2 — the interpreter cap is 128
    /// and every append guards `len < 128` with the 40 thunk), plus the
    /// length shadow in the slot's own frame cell. `init_n` = creation
    /// length (0 for NEWLIST, N for a PACKLIST that later mutates).
    Growable { init_n: u64 },
}

/// P3-V2IMPL: the list side of a `FramePlan` — `kind` is index-aligned with
/// slots; `lits` carries the compile-time element values of `Rodata`
/// tables (empty for the other kinds).
#[derive(Debug, Clone, Default)]
struct ListPlanData {
    kind: Vec<ListKind>,
    lits: Vec<Vec<u64>>,
}

/// P3-KERNELS M2-d: whether an all-literal table's values all fit u32 —
/// the deterministic rule selecting the 4-byte-stride rodata packing
/// (identical loaded values via lwu zero-extension).
fn list_lits_fit_u32(vals: &[u64]) -> bool {
    !vals.is_empty() && vals.iter().all(|&v| v < (1u64 << 32))
}

/// Every child of `node` in evaluation order.
fn children(node: &dyn IRNode) -> Vec<&dyn IRNode> {
    fn push_box<'a>(out: &mut Vec<&'a dyn IRNode>, b: &'a Box<dyn IRNode>) {
        out.push(&**b);
    }
    let any = node.as_any();
    let mut out = Vec::new();
    if let Some(n) = any.downcast_ref::<IRNodeWrapOne>() {
        push_box(&mut out, &n.node);
    } else if let Some(n) = any.downcast_ref::<IRNodeSingle>() {
        push_box(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeDouble>() {
        push_box(&mut out, &n.subx);
        push_box(&mut out, &n.suby);
    } else if let Some(n) = any.downcast_ref::<IRNodeTriple>() {
        push_box(&mut out, &n.subx);
        push_box(&mut out, &n.suby);
        push_box(&mut out, &n.subz);
    } else if let Some(n) = any.downcast_ref::<IRNodeParamsSingle>() {
        push_box(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeParam1Single>() {
        push_box(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeParam2Single>() {
        push_box(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeArray>() {
        for s in &n.subs {
            push_box(&mut out, s);
        }
    }
    out
}

/// Pre-pass result: slot types + whether each slot must be zero-initialized
/// (a slot whose first touch is a read, or whose first write sits under a
/// conditional, must be zeroed; `var x = 0` prologues disappear otherwise)
/// + the P3-V2IMPL list storage plan.
struct FramePlan {
    slot: Vec<SlotTy>,
    needs_zero: Vec<bool>,
    list: ListPlanData,
    /// P3-PAIRPKG: remove+insert pair marks (Root on the REMOVE statement,
    /// Skip on the INSERT sibling) — produced by the same plan() sequence
    /// walk, consumed by the lowering's statement dispatch.
    pair: PairPlan,
}

/* ------------------------------------------------------------------ */
/* P3-V2IMPL: compo-list v2 shape analysis (plan() = single source of  */
/* truth for admit/reject/lower-data; doc 74 E3/E6, doc 73 E7/E8).     */
/* ------------------------------------------------------------------ */

/// The source shape of a list-creating `PUT`: `[]`/`new_list()` (a bare
/// NEWLIST leaf) or `[e0..eN-1]` (the `[elements…, count, PACKLIST]` IRLIST
/// wrapper built by `context.rs build_list_node`). Returns the element
/// array + count for the PACKLIST form.
enum ListCreate<'a> {
    NewList,
    PackList { arr: &'a IRNodeArray, n: u64 },
}

fn list_create_src(node: &dyn IRNode) -> Option<ListCreate<'_>> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return list_create_src(&*w.node);
    }
    let op = Bytecode::try_from_u8(node.bytecode()).ok()?;
    match op {
        Bytecode::NEWLIST => {
            node.as_any().downcast_ref::<IRNodeLeaf>()?;
            Some(ListCreate::NewList)
        }
        Bytecode::IRLIST => {
            let arr = node.as_any().downcast_ref::<IRNodeArray>()?;
            // [..., count_literal, PACKLIST]: the wrapper must have ≥2 subs,
            // the last is the PACKLIST marker leaf, the second-to-last the
            // element count literal, and the count must equal the number of
            // element subs in front of it.
            if arr.subs.len() < 2 {
                return None;
            }
            let last = &*arr.subs[arr.subs.len() - 1];
            let pack = last.as_any().downcast_ref::<IRNodeLeaf>()?;
            if pack.inst != Bytecode::PACKLIST {
                return None;
            }
            let cnt = &*arr.subs[arr.subs.len() - 2];
            let v = crate::lang::ir_literal_value(cnt).ok().flatten()?;
            let n = v.extract_u128().ok()?;
            if n != (arr.subs.len() - 2) as u128 {
                return None;
            }
            if n > u64::MAX as u128 {
                return None;
            }
            Some(ListCreate::PackList { arr, n: n as u64 })
        }
        _ => None,
    }
}

/// Does this expression's VALUE classify as a list under the shape set?
/// Used by plan()'s escape check (a list value flowing into any consumer
/// outside the five admitted ops is a reject naming that consumer's
/// opcode) and by the PUT arm's alias detection.
fn expr_is_list_valued(node: &dyn IRNode, slot: &[SlotTy]) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return expr_is_list_valued(&*w.node, slot);
    }
    let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return false };
    use Bytecode::*;
    match op {
        NEWLIST | PACKLIST | APPEND | CLEAR => true,
        // P3-PAIRPKG: remove/insert yield the receiver (bytecode metadata
        // hasretval=1) — a pair-marked node at a block drop position is
        // the admitted mutation form (the escape-check gate consults the
        // pair marks); unmarked nodes were already plan-rejected.
        REMOVE | INSERT => true,
        GET | GET0 | GET1 | GET2 | GET3 => slot_get_index(node)
            .map(|i| matches!(slot.get(i as usize), Some(SlotTy::List)))
            .unwrap_or(false),
        CHOOSE | IRIFR => node
            .as_any()
            .downcast_ref::<IRNodeTriple>()
            .map(|t| {
                expr_is_list_valued(&*t.suby, slot) || expr_is_list_valued(&*t.subz, slot)
            })
            .unwrap_or(false),
        IRLIST | IRBLOCK | IRBLOCKR => node
            .as_any()
            .downcast_ref::<IRNodeArray>()
            .filter(|_| node.hasretval())
            .and_then(|a| a.subs.last())
            .map(|s| expr_is_list_valued(&**s, slot))
            .unwrap_or(false),
        _ => false,
    }
}

/// P3-PAIRPKG: does this expression subtree reference the given slot at
/// all (any GET, directly or as a list-op receiver)? Every reference to a
/// list flows through a GET node, so a plain GET-index descent is the
/// complete predicate. The pair recognizer refuses receiver references in
/// the index and value expressions: the interpreter evaluates them AFTER
/// the remove executed (shifted view — doc 75 E6 k6 probe; `length(l)`
/// mid-pair differs, `l[..]` reads the shifted element).
fn expr_touches_slot(node: &dyn IRNode, s: usize) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return expr_touches_slot(&*w.node, s);
    }
    if slot_get_index(node).map(|i| i as usize == s).unwrap_or(false) {
        return true;
    }
    children(node).into_iter().any(|c| expr_touches_slot(c, s))
}

/// P3-PAIRPKG: does this expression subtree contain statement-form nodes?
/// A pair's index/value subtrees must be pure expression forms: the
/// statement numbering cursor (`web_idx`, `WebWalk::walk_stmt`) counts
/// nested statement nodes, and the subsumed INSERT skips its dispatch —
/// any statement form inside would desync the numbering.
fn expr_has_stmt_form(node: &dyn IRNode) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return expr_has_stmt_form(&*w.node);
    }
    let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return false };
    if matches!(
        op,
        Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR | Bytecode::IRIF | Bytecode::IRWHILE
    ) {
        return true;
    }
    children(node).into_iter().any(|c| expr_has_stmt_form(c))
}

/// The receiver slot of an admitted list op (ITEMGET/APPEND/CLEAR/LENGTH/
/// HASKEY): the receiver must be a plain GET of a List-typed slot. Every
/// other receiver form (a fresh literal, a bytes read, a computed value)
/// keeps the historical reject — the canonical subset text naming the op,
/// which the fitshc boundary (kernel.rs OP_CONSTRUCT) translates.
fn list_receiver_slot(node: &dyn IRNode, slot: &[SlotTy], op: Bytecode) -> Ret<usize> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return list_receiver_slot(&*w.node, slot, op);
    }
    match slot_get_index(node)
        .filter(|i| matches!(slot.get(*i as usize), Some(SlotTy::List)))
    {
        Some(i) => Ok(i as usize),
        None => errf!("rv64 lower: opcode {op:?} is outside the translatable kernel subset"),
    }
}

fn plan(nodes: &[Box<dyn IRNode>], has_input: bool) -> Ret<FramePlan> {
    let mut slot: Vec<SlotTy> = Vec::new();
    let mut needs_zero: Vec<bool> = Vec::new();
    let mut seen: Vec<bool> = Vec::new();
    let mut lst = ListSt::default();
    // P3-PAIRPKG: pair marks collected by the statement-sequence walk
    let mut pairs = PairPlan::default();
    fn grow(
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt,
        idx: usize,
    ) {
        while slot.len() <= idx {
            slot.push(SlotTy::None);
            seen.push(false);
            needs_zero.push(true);
        }
        lst.grow_to(idx);
    }
    // P3-V2IMPL: the list side of the plan walk. `kind` holds the CREATION
    // candidate (Empty / Rodata / FrameStatic); `appended`/`cleared` mark
    // mutation reachability; the final storage class is refined after the
    // walk (any mutation upgrades the slot to Growable, D2). P3-PAIRPKG:
    // `paired` is the remove+insert pair touch — the same Growable upgrade
    // (a pair mutates the element block; the length shadow is untouched by
    // the FUSED form but the slot is no longer read-only).
    #[derive(Default)]
    struct ListSt {
        kind: Vec<ListKind>,
        lits: Vec<Vec<u64>>,
        appended: Vec<bool>,
        cleared: Vec<bool>,
        paired: Vec<bool>,
    }
    impl ListSt {
        fn grow_to(&mut self, idx: usize) {
            while self.kind.len() <= idx {
                self.kind.push(ListKind::NotList);
                self.lits.push(Vec::new());
                self.appended.push(false);
                self.cleared.push(false);
                self.paired.push(false);
            }
        }
    }
    // P3-V2IMPL: PUT whose source is list-valued — creation (NEWLIST /
    // PACKLIST wrapper), alias (GET of another list), re-bind (PUT into an
    // existing list slot) or an append/clear expression result. Routed
    // BEFORE the generic child walk: the PACKLIST marker sub is not a
    // value child.
    #[allow(clippy::too_many_arguments)]
    fn walk_list_put(
        src: &dyn IRNode,
        idx: usize,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
    ) -> Ret<()> {
        let _ = has_input;
        grow(slot, seen, needs_zero, lst, idx);
        if matches!(slot[idx], SlotTy::List) {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (re-binding the list in slot {idx}; lists are single-assignment — \
                 mutate with append/clear instead)"
            );
        }
        if !matches!(slot[idx], SlotTy::None) {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (slot {idx} is already {:?}; a list value cannot be stored into it)",
                slot[idx]
            );
        }
        if conditional {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (list slot {idx} is declared under a conditional; declare lists \
                 unconditionally)"
            );
        }
        match list_create_src(src) {
            Some(ListCreate::NewList) => {
                slot[idx] = SlotTy::List;
                seen[idx] = true;
                // the len shadow store always executes at the creation, and
                // the static classes never read the slot cell: no entry zero
                needs_zero[idx] = false;
                lst.kind[idx] = ListKind::Empty;
                Ok(())
            }
            Some(ListCreate::PackList { arr, n }) => {
                if n == 0 {
                    return errf!(
                        "rv64 lower: opcode PACKLIST is outside the translatable \
                         kernel subset (zero-element pack; write `[]` for an empty list)"
                    );
                }
                if n > 128 {
                    return errf!(
                        "rv64 lower: opcode PACKLIST is outside the translatable \
                         kernel subset ({n} elements exceed the 128-item list capacity)"
                    );
                }
                let mut vals: Vec<u64> = Vec::with_capacity(n as usize);
                let mut all_lit = true;
                for (i, e) in arr.subs[..n as usize].iter().enumerate() {
                    walk(&**e, slot, seen, needs_zero, lst, pairs, false, has_input)?;
                    let ty = node_res_ty(&**e, slot)?;
                    if ty != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode PACKLIST is outside the translatable \
                             kernel subset (element {i} is not a u64 value)"
                        );
                    }
                    let lit = crate::lang::ir_literal_value(&**e)
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract_u128().ok())
                        .filter(|x| *x <= u64::MAX as u128);
                    match lit {
                        Some(x) => vals.push(x as u64),
                        None => all_lit = false,
                    }
                }
                slot[idx] = SlotTy::List;
                seen[idx] = true;
                needs_zero[idx] = false;
                lst.kind[idx] = if all_lit {
                    ListKind::Rodata { n }
                } else {
                    ListKind::FrameStatic { n }
                };
                lst.lits[idx] = vals;
                Ok(())
            }
            None => errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (aliasing a list value into slot {idx}; list values cannot be copied)"
            ),
        }
    }
    fn walk(
        node: &dyn IRNode,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
    ) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*w.node, slot, seen, needs_zero, lst, pairs, conditional, has_input);
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        // Writes under IRIF/CHOOSE/IRWHILE do not dominate later reads.
        let inner_cond = conditional || matches!(op, Bytecode::IRIF | Bytecode::IRIFR | Bytecode::CHOOSE | Bytecode::IRWHILE);
        // P3-V2IMPL: list-CREATING PUTs bypass the generic child walk (see
        // walk_list_put). Other list-valued PUT sources (GET alias, choose
        // arms, append results) walk their children first — the deeper
        // consumer gets named — and then the PUT arm below rejects.
        if op == Bytecode::PUT {
            if let Some(pn) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                if !(has_input && pn.para == 0) && list_create_src(&*pn.subx).is_some() {
                    return walk_list_put(
                        &*pn.subx,
                        pn.para as usize,
                        slot,
                        seen,
                        needs_zero,
                        lst,
                        pairs,
                        conditional,
                        has_input,
                    );
                }
            }
        }
        // P3-PAIRPKG: statement arrays walk as a SEQUENCE — the adjacent
        // REMOVE+INSERT pair recognition needs direct siblings. The
        // PACKLIST creation wrapper ([elems, count, PACKLIST]) is a value
        // form with no statement shape and keeps the generic child walk.
        // The array's own match-arm processing (the escape check below)
        // still runs afterwards.
        let seq_arr = matches!(
            op,
            Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR
        ) && list_create_src(node).is_none();
        if seq_arr {
            if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                // a block keeps its tail value iff it is IRLIST/IRBLOCKR
                // (the eval-position block arm mirrors this exactly: only
                // those two ops keep; a pair at such a tail would have no
                // value to keep — refused in walk_try_pair, M3 precedent)
                let keep_tail = op != Bytecode::IRBLOCK;
                walk_stmt_seq(
                    &arr.subs,
                    keep_tail,
                    slot,
                    seen,
                    needs_zero,
                    lst,
                    pairs,
                    inner_cond,
                    has_input,
                )?;
            } else {
                for c in children(node) {
                    walk(c, slot, seen, needs_zero, lst, pairs, inner_cond, has_input)?;
                }
            }
        } else {
            for c in children(node) {
                walk(c, slot, seen, needs_zero, lst, pairs, inner_cond, has_input)?;
            }
        }
        match op {
            Bytecode::PUT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: PUT node shape unexpected")?;
                if has_input && n.para == 0 {
                    // The prelude `PUT 0 (ROLL0)` param binding: slot 0 IS
                    // the calc input; the ROLL0 source is stack machinery.
                    grow(slot, seen, needs_zero, lst, 0);
                    slot[0] = SlotTy::Input;
                    seen[0] = true;
                    needs_zero[0] = false;
                    return Ok(());
                }
                let ty = node_res_ty(&*n.subx, slot)?;
                let idx = n.para as usize;
                grow(slot, seen, needs_zero, lst, idx);
                if matches!(slot[idx], SlotTy::List) {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (slot {idx} holds a list; only list-producing expressions may \
                         bind it)"
                    );
                }
                // P3-V2IMPL: a list-valued source into a non-list slot is
                // the alias escape (children were walked first, so a list
                // consumed by a deeper opcode was already named there)
                if ty == Ty::List {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (aliasing a list value into slot {idx}; list values cannot be \
                         copied)"
                    );
                }
                if !seen[idx] {
                    seen[idx] = true;
                    if !conditional {
                        needs_zero[idx] = false;
                    }
                }
                slot[idx] = match (slot[idx], ty) {
                    (SlotTy::None, Ty::Int) => SlotTy::Int,
                    (SlotTy::None, Ty::Bytes) => SlotTy::Bytes,
                    (a, b)
                        if (a == SlotTy::Int && b == Ty::Int)
                            || (a == SlotTy::Bytes && b == Ty::Bytes) => a,
                    (a, b) => {
                        // GT-c T8-style: named types instead of Debug
                        return errf!(
                            "rv64 lower: local slot {idx} used with conflicting types {} vs {}",
                            slot_ty_name(a),
                            ty_name(b)
                        )
                    }
                };
                Ok(())
            }
            Bytecode::XOP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: XOP node shape unexpected")?;
                let (_, idx) = decode_local_operand_mark(n.para);
                if has_input && idx == 0 {
                    // GT-c: asserted core substring "input parameter cannot be assigned"
                        // stays contiguous (rv64_gen_corpus.rs P2 message pin);
                        // the hint appends after it
                        return errf!(
                            "rv64 lower: the input parameter cannot be assigned; the call-data \
                             argument is read-only — copy it into a local first \
                             (e.g. `var t = data`) and assign to that"
                        );
                }
                let idx = idx as usize;
                grow(slot, seen, needs_zero, lst, idx);
                seen[idx] = true; // XOP reads the old value: must be zeroed
                slot[idx] = match slot[idx] {
                    SlotTy::None | SlotTy::Int => SlotTy::Int,
                    SlotTy::List => {
                        return errf!(
                            "rv64 lower: opcode XOP is outside the translatable kernel \
                             subset (slot {idx} holds a list; compound assignment on a \
                             list is not supported)"
                        );
                    }
                    other => {
                        // GT-c T8-style: no Debug leak — name the slot type
                        return errf!(
                            "rv64 lower: local slot {idx} mixes bytes with int (slot type {})",
                            slot_ty_name(other)
                        )
                    }
                };
                Ok(())
            }
            Bytecode::GET | Bytecode::GET0 | Bytecode::GET1 | Bytecode::GET2 | Bytecode::GET3 => {
                let (idx, is_input) = match op {
                    Bytecode::GET => (
                        node.as_any()
                            .downcast_ref::<IRNodeParam1>()
                            .ok_or("rv64 lower: GET node shape unexpected")?
                            .para as usize,
                        false,
                    ),
                    Bytecode::GET0 => (0, has_input),
                    Bytecode::GET1 => (1, false),
                    Bytecode::GET2 => (2, false),
                    _ => (3, false),
                };
                grow(slot, seen, needs_zero, lst, idx);
                if is_input {
                    slot[idx] = SlotTy::Input;
                }
                seen[idx] = true;
                Ok(())
            }
            // ---- P3-V2IMPL: the five admitted list consumers ------------
            Bytecode::ITEMGET | Bytecode::HASKEY | Bytecode::APPEND => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                if op != Bytecode::APPEND {
                    let kty = node_res_ty(&*n.suby, slot)?;
                    if kty != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode {op:?} is outside the translatable kernel \
                             subset (the list index must be a u64 value)"
                        );
                    }
                } else {
                    let vty = node_res_ty(&*n.suby, slot)?;
                    if vty != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode APPEND is outside the translatable kernel \
                             subset (the appended value must be a u64 value)"
                        );
                    }
                    lst.grow_to(rs);
                    lst.appended[rs] = true;
                }
                Ok(())
            }
            Bytecode::LENGTH | Bytecode::CLEAR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                if op == Bytecode::CLEAR {
                    lst.grow_to(rs);
                    lst.cleared[rs] = true;
                }
                Ok(())
            }
            // ---- every other compo opcode: outside the subset, named -----
            Bytecode::NEWMAP
            | Bytecode::PACKMAP
            | Bytecode::PACKTUPLE
            | Bytecode::TUPLE2LIST
            | Bytecode::INSERT
            | Bytecode::REMOVE
            | Bytecode::MERGE
            | Bytecode::KEYS
            | Bytecode::VALUES
            | Bytecode::TAKEFIRST
            | Bytecode::TAKELAST
            | Bytecode::CLONE
            | Bytecode::UNPACK => {
                errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset"
                )
            }
            // ---- escape check: a list value flowing into any other op ----
            _ => {
                // The `[..]` creation wrapper itself is not an escape — its
                // legality is decided by its consumer (the PUT arm above or
                // the positional checks of the five admitted ops).
                let is_packlist_wrapper = op == Bytecode::IRLIST
                    && list_create_src(node).is_some();
                // Statement blocks (IRLIST/IRBLOCK/IRBLOCKR) DROP their
                // children's values — an append/clear there is the admitted
                // mutation statement form, its list result is not consumed
                // by the block. Every other list-valued child of a block is
                // a pointless drop (fail-fast: B-family, generator- and
                // target-unreachable).
                let is_block = matches!(
                    op,
                    Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR
                );
                if !is_packlist_wrapper {
                    for c in children(node) {
                        if expr_is_list_valued(c, slot) {
                            let dropped_mutation = is_block && {
                                let cop = Bytecode::try_from_u8(c.bytecode())
                                    .map(|o| {
                                        matches!(
                                            o,
                                            Bytecode::APPEND | Bytecode::CLEAR
                                        )
                                    })
                                    .unwrap_or(false);
                                // unwrap WrapOne first
                                let wrapped = c
                                    .as_any()
                                    .downcast_ref::<IRNodeWrapOne>()
                                    .map(|w| {
                                        Bytecode::try_from_u8(w.node.bytecode())
                                            .map(|o| {
                                                matches!(
                                                    o,
                                                    Bytecode::APPEND | Bytecode::CLEAR
                                                )
                                            })
                                            .unwrap_or(false)
                                    })
                                    .unwrap_or(false);
                                // P3-PAIRPKG: a pair-marked remove (Root) or
                                // insert (Skip) at a drop position is the
                                // admitted mutation form — its list result is
                                // not consumed by the block (the fused form
                                // has no value at all)
                                let cu = fuse_unwrap(&*c);
                                let marked = pairs.skip.contains(&node_addr(cu))
                                    || pairs.root.contains_key(&node_addr(cu));
                                cop || wrapped || marked
                            };
                            if dropped_mutation {
                                continue;
                            }
                            return errf!(
                                "rv64 lower: opcode {op:?} is outside the translatable \
                                 kernel subset (a list value cannot be used by {op:?})"
                            );
                        }
                    }
                }
                Ok(())
            }
        }
    }
    // P3-PAIRPKG: the statement-sequence walk — the single pair recognition
    // site (same statement-array traversal order the lowering's two
    // sequence dispatch sites mirror). Sibling adjacency means subs[i] and
    // subs[i+1] with NOTHING between: any intervening statement or control
    // flow breaks the pair (the negative family pins this).
    #[allow(clippy::too_many_arguments)]
    fn walk_stmt_seq(
        subs: &[Box<dyn IRNode>],
        keep_tail: bool,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
    ) -> Ret<()> {
        let mut i = 0usize;
        while i < subs.len() {
            if let Some(consumed) = walk_try_pair(
                subs,
                i,
                keep_tail,
                slot,
                seen,
                needs_zero,
                lst,
                pairs,
                conditional,
                has_input,
            )? {
                i += consumed;
                continue;
            }
            walk(&*subs[i], slot, seen, needs_zero, lst, pairs, conditional, has_input)?;
            i += 1;
        }
        Ok(())
    }

    /// Try to recognize the adjacent remove+insert pair at (subs[i],
    /// subs[i+1]). Returns Some(2) when the pair is admitted (both
    /// statements consumed — the insert's receiver/index subtrees are the
    /// same shapes the remove side already walked). Returns None to fall
    /// through to the normal per-statement walk, where an unpaired
    /// REMOVE/INSERT hits its opcode-named reject arm.
    #[allow(clippy::too_many_arguments)]
    fn walk_try_pair(
        subs: &[Box<dyn IRNode>],
        i: usize,
        keep_tail: bool,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
    ) -> Ret<Option<usize>> {
        if i + 1 >= subs.len() {
            return Ok(None);
        }
        // post-WRAP shapes (the statement dispatch sees the same nodes)
        let rm = fuse_unwrap(&*subs[i]);
        let ins = fuse_unwrap(&*subs[i + 1]);
        if !matches!(
            Bytecode::try_from_u8(rm.bytecode()),
            Ok(Bytecode::REMOVE)
        ) || !matches!(
            Bytecode::try_from_u8(ins.bytecode()),
            Ok(Bytecode::INSERT)
        ) {
            return Ok(None);
        }
        // A pair at a kept tail is refused: remove/insert yield the
        // receiver value and the fused form has none to keep (the M3
        // chain-tail precedent). Fall through — the remove then reaches
        // its reject arm naming REMOVE.
        if keep_tail && i + 2 == subs.len() {
            return Ok(None);
        }
        let Some(rn) = rm.as_any().downcast_ref::<IRNodeDouble>() else {
            return Ok(None);
        };
        let Some(nn) = ins.as_any().downcast_ref::<IRNodeTriple>() else {
            return Ok(None);
        };
        // same receiver slot, both plain GET-of-List
        let Ok(rs) = list_receiver_slot(&*rn.subx, slot, Bytecode::REMOVE) else {
            return Ok(None);
        };
        let Ok(ns) = list_receiver_slot(&*nn.subx, slot, Bytecode::INSERT) else {
            return Ok(None);
        };
        if rs != ns {
            return Ok(None);
        }
        // index/value types (fall-through keeps the canonical reject text)
        if node_res_ty(&*rn.suby, slot)? != Ty::Int {
            return Ok(None);
        }
        if node_res_ty(&*nn.subz, slot)? != Ty::Int {
            return Ok(None);
        }
        // structurally equal indices (same literal or same slot read —
        // the M3 structural-key precedent)
        if fuse_struct_key(&*rn.suby) != fuse_struct_key(&*nn.suby) {
            return Ok(None);
        }
        // receiver-free discipline: the interpreter evaluates BOTH index
        // expressions and the value expression AFTER the remove executed
        // (shifted view; `length(l)` mid-pair differs) — any receiver
        // reference in idx/value changes the pair's meaning (doc 98 §Method)
        if expr_touches_slot(&*rn.suby, rs)
            || expr_touches_slot(&*nn.suby, rs)
            || expr_touches_slot(&*nn.subz, rs)
        {
            return Ok(None);
        }
        // pure expression forms only (statement numbering stays aligned
        // when the subsumed insert skips its dispatch)
        if expr_has_stmt_form(&*rn.suby)
            || expr_has_stmt_form(&*nn.suby)
            || expr_has_stmt_form(&*nn.subz)
        {
            return Ok(None);
        }
        // constant index if the (equal) index expressions fold to a
        // literal — the emission takes the direct-offset form
        let const_idx = fuse_const(&*rn.suby);
        pairs.root.insert(node_addr(rm), PairRec { slot: rs, const_idx });
        pairs.skip.insert(node_addr(ins));
        lst.grow_to(rs);
        lst.paired[rs] = true;
        // walk the receiver GET + the index subtree (the remove side owns
        // both — the insert's are the same shapes) and the value subtree
        walk(&*rn.subx, slot, seen, needs_zero, lst, pairs, conditional, has_input)?;
        walk(&*rn.suby, slot, seen, needs_zero, lst, pairs, conditional, has_input)?;
        walk(&*nn.subz, slot, seen, needs_zero, lst, pairs, conditional, has_input)?;
        Ok(Some(2))
    }
    // P3-PAIRPKG: the body itself is a statement sequence (pair recognition
    // on direct top-level siblings; the tail-value keep flag is false).
    walk_stmt_seq(
        nodes,
        false,
        &mut slot,
        &mut seen,
        &mut needs_zero,
        &mut lst,
        &mut pairs,
        false,
        has_input,
    )?;
    // P3-V2IMPL: refine the list storage classes — any APPEND/CLEAR
    // reachability upgrades the slot to the growable form (frame block
    // 1024B + len shadow, decision D2). P3-PAIRPKG: a pair touch does the
    // same (the fused form stores into the element block; the length
    // shadow itself is net-zero — see the M6 lemmas).
    if !slot.is_empty() {
        lst.grow_to(slot.len() - 1);
    }
    for idx in 0..slot.len() {
        if !matches!(slot[idx], SlotTy::List) {
            continue;
        }
        let mutated = lst.appended[idx] || lst.cleared[idx] || lst.paired[idx];
        lst.kind[idx] = match lst.kind[idx] {
            ListKind::Empty if mutated => ListKind::Growable { init_n: 0 },
            ListKind::Rodata { n } if mutated => ListKind::Growable { init_n: n },
            ListKind::FrameStatic { n } if mutated => ListKind::Growable { init_n: n },
            k => k,
        };
    }
    let list = ListPlanData { kind: lst.kind, lits: lst.lits };
    let pair = pairs;
    Ok(FramePlan { slot, needs_zero, list, pair })
}

/* ------------------------------------------------------------------ */
/* Nil-tracking pre-pass (BUG-012).                                    */
/*                                                                     */
/* The consensus interpreter keeps HVM slots as dynamic Values: a slot  */
/* whose declaring `var` never executed reads back Nil, and the fault   */
/* fires at the CONSUMING instruction (GET itself never faults):        */
/*   - EQ/NEQ ......... Arithmetic(90) unless BOTH sides are Nil        */
/*   - LT/GT/LE/GE .... Arithmetic(90) on ANY Nil operand               */
/*   - int arithmetic / XOP old value / `as bytes` ... CastFail(71)     */
/*   - bytes family ... BytesHandle(91); u16 params ... CastParamFail   */
/*     (72); require/throw value ... CastBeValueFail (76)              */
/*   - bool coercion (&&/||/!), casts to uint, size(), bare return:     */
/*     NO fault (Nil→false / 0 / 0 / empty output)                      */
/* Nil also PROPAGATES through assignment (`r = t9` stores Nil into r) */
/* and through CHOOSE/IRIFR arm selection, so the analysis is a        */
/* flow-sensitive may-Nil dataflow over the statement tree. The result */
/* is consumed by the lowering: a per-slot init FLAG byte in the frame */
/* (prologue-zeroed, set on every non-Nil store, copied from the       */
/* source's TAG when the source may be Nil) and a TAG byte stack       */
/* indexed by vstack position carrying each may-Nil value's Nil bit    */
/* to its consumer, where the guard faults with the exact interpreter  */
/* class. Slots proven non-Nil at every point (all anchor kernels:     */
/* unconditional declarations, loop-standard induction) get NO flag,   */
/* NO tag traffic and NO guards — the definite-assignment refinement.  */
/* ------------------------------------------------------------------ */

/// Per-node Nil info recorded by the pre-pass, keyed by node address
/// (stable for the frozen tree; the lowering only ever looks up).
#[derive(Default, Clone, Copy)]
struct NilOcc {
    /// this node's VALUE may be Nil when produced (GET of a may-Nil slot,
    /// CHOOSE/IRIFR with a may-Nil arm, CAT with a may-Nil empty-fold
    /// side); consumers query their operand nodes
    val: bool,
    /// XOP only: the OLD slot value this XOP reads may be Nil (guard 71)
    xop_old: bool,
}

fn node_addr(node: &dyn IRNode) -> usize {
    node as *const dyn IRNode as *const u8 as usize
}

struct NilPlan {
    /// slot idx → needs an init flag byte in the frame
    tracked: Vec<bool>,
    occ: HashMap<usize, NilOcc>,
}

fn nil_plan(nodes: &[Box<dyn IRNode>], slot: &[SlotTy], has_input: bool) -> NilPlan {
    // flow state: slot idx → may currently hold Nil. Untouched slots are
    // Nil until first written; the `data` param slot never is.
    let mut st: Vec<bool> = slot.iter().map(|s| !matches!(s, SlotTy::Input)).collect();
    let n_slots = slot.len();
    let mut plan = NilPlan { tracked: vec![false; n_slots], occ: HashMap::new() };
    if n_slots == 0 {
        return plan;
    }
    // `has_input` only names slot 0 as the param when the kernel takes one
    if !has_input {
        if let Some(first) = st.first_mut() {
            if matches!(slot[0], SlotTy::Input) {
                *first = false;
            }
        }
    }

    fn join(a: &mut Vec<bool>, b: &[bool]) {
        for (x, y) in a.iter_mut().zip(b.iter()) {
            *x = *x || *y;
        }
    }

    struct W<'a> {
        slot: &'a [SlotTy],
        plan: &'a mut NilPlan,
    }
    impl<'a> W<'a> {
        fn occ_val(&self, node: &dyn IRNode) -> bool {
            self.plan.occ.get(&node_addr(node)).map(|o| o.val).unwrap_or(false)
        }
        /// may-Nil of an operand subtree: reads the children's recorded
        /// bits (they were walked before the consumer processes them)
        fn expr_may_nil(&self, node: &dyn IRNode) -> bool {
            if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
                return self.expr_may_nil(&*w.node);
            }
            match Bytecode::try_from_u8(node.bytecode()).ok() {
                Some(Bytecode::CHOOSE) | Some(Bytecode::IRIFR) => {
                    let t = node
                        .as_any()
                        .downcast_ref::<IRNodeTriple>()
                        .expect("triple shape checked in walk");
                    self.expr_may_nil(&*t.suby) || self.expr_may_nil(&*t.subz)
                }
                Some(Bytecode::IRLIST) | Some(Bytecode::IRBLOCK) | Some(Bytecode::IRBLOCKR) => {
                    if !node.hasretval() {
                        return false;
                    }
                    let arr = node
                        .as_any()
                        .downcast_ref::<IRNodeArray>()
                        .expect("array shape checked in walk");
                    arr.subs.last().map(|s| self.expr_may_nil(&**s)).unwrap_or(false)
                }
                _ => self.occ_val(node),
            }
        }
        fn walk(&mut self, node: &dyn IRNode, st: &mut Vec<bool>) {
            if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
                self.walk(&*w.node, st);
                let v = self.occ_val(&*w.node);
                self.plan.occ.entry(node_addr(node)).or_default().val = v;
                return;
            }
            let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return };
            use Bytecode::*;
            match op {
                PUT => {
                    let n = node.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    self.walk(&*n.subx, st);
                    let idx = n.para as usize;
                    if idx >= st.len() || matches!(self.slot.get(idx), Some(SlotTy::Input)) {
                        return; // input-param assignment: lowering rejects it
                    }
                    let src_may = self.expr_may_nil(&*n.subx);
                    if src_may {
                        // the flag store copies the source's tag
                        self.plan.tracked[idx] = true;
                    }
                    st[idx] = src_may;
                }
                XOP => {
                    let n = node.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    self.walk(&*n.subx, st);
                    let (_, idx) = decode_local_operand_mark(n.para);
                    let idx = idx as usize;
                    if idx >= st.len() {
                        return;
                    }
                    let old_may = st[idx];
                    let e = self.plan.occ.entry(node_addr(node)).or_default();
                    e.xop_old = old_may;
                    self.plan.tracked[idx] |= old_may;
                    st[idx] = false; // the op faults on Nil or computes non-Nil
                }
                GET | GET0 | GET1 | GET2 | GET3 => {
                    let idx = match op {
                        GET => node.as_any().downcast_ref::<IRNodeParam1>().unwrap().para as usize,
                        GET0 => 0,
                        GET1 => 1,
                        GET2 => 2,
                        _ => 3,
                    };
                    let may = idx < st.len()
                        && st[idx]
                        && !matches!(self.slot.get(idx), Some(SlotTy::Input));
                    self.plan.occ.entry(node_addr(node)).or_default().val = may;
                    if may {
                        self.plan.tracked[idx] = true;
                    }
                }
                CHOOSE | IRIFR | IRIF => {
                    let n = node.as_any().downcast_ref::<IRNodeTriple>().unwrap();
                    self.walk(&*n.subx, st); // condition: value discarded
                    let pre = st.clone();
                    self.walk(&*n.suby, st);
                    let then_exit = st.clone();
                    *st = pre;
                    self.walk(&*n.subz, st);
                    join(st, &then_exit);
                    if op != Bytecode::IRIF {
                        // the selected arm value passes through
                        let v = self.expr_may_nil(&*n.suby) || self.expr_may_nil(&*n.subz);
                        self.plan.occ.entry(node_addr(node)).or_default().val = v;
                    }
                }
                IRWHILE => {
                    let n = node.as_any().downcast_ref::<IRNodeDouble>().unwrap();
                    // fixpoint entry state: iteration k>=1 sees the body's
                    // stores. Monotone, bounded by the slot count.
                    let pre = st.clone();
                    let mut entry = pre.clone();
                    loop {
                        let mut s_try = entry.clone();
                        self.walk(&*n.subx, &mut s_try); // cond re-evaluates
                        self.walk(&*n.suby, &mut s_try); // body
                        let mut next = pre.clone();
                        join(&mut next, &s_try);
                        if next == entry {
                            break;
                        }
                        entry = next;
                    }
                    *st = entry;
                }
                _ => {
                    for c in children(node) {
                        self.walk(c, st);
                    }
                    // value-position blocks pass their LAST sub's value
                    // through (Nil included)
                    if node.hasretval()
                        && matches!(op, IRLIST | IRBLOCK | IRBLOCKR)
                    {
                        let arr = node.as_any().downcast_ref::<IRNodeArray>().unwrap();
                        let v = arr
                            .subs
                            .last()
                            .map(|s| self.occ_val(&**s))
                            .unwrap_or(false);
                        self.plan.occ.entry(node_addr(node)).or_default().val = v;
                    }
                }
            }
        }
    }
    let mut w = W { slot, plan: &mut plan };
    for n in nodes {
        w.walk(&**n, &mut st);
    }
    plan
}

/* ------------------------------------------------------------------ */
/* P3-IDIOMS: constant-offset byte-assembly groups → one wide load.    */
/* Recognition is a static scan run AFTER plan()+nil_plan() (it needs  */
/* slot types and the may-Nil occ map); the marks are consumed by the  */
/* lower. Semantics proof: doc 94 Method/Lemmas 1-6. Every accept/reject   */
/* decision below is a deterministic function of the IR.               */
/* ------------------------------------------------------------------ */

/// The recognized fusion form (doc 94 admission shape v1).
#[derive(Clone, Copy, Debug, PartialEq)]
enum FuseForm {
    /// statement-accumulated LE chain (keccak absorb shape):
    /// `PUT s, byte(recv,c0)` / `PUT s, s | byte(recv,c0+k)<<8k`
    LeChain,
    /// single-expression LE |/^ tree, leaves = const-offset byte()
    LeExpr,
    /// BE mul-add tree, all-const indexes (hacd200 :629 shape)
    BeConst,
    /// BE mul-add tree, leaf0 idx = BASE, leaf_k idx = ADD(BASE,k)
    /// (sha256 input-ring shape)
    BeVar,
}

/// Node key → mark. Root marks sit on the FIRST non-WRAP node of the
/// group's value root (the node eval() actually dispatches on); Skip
/// marks sit on the chain-continuation PUT statements (post-WRAP, the
/// node stmt_one_inner dispatches on).
#[derive(Clone, Debug)]
enum FuseMark {
    Root { form: FuseForm, c0: u64, n: u64 },
    Skip,
}

type FuseMap = HashMap<usize, FuseMark>;

/* ------------------------------------------------------------------ */
/* P3-PAIRPKG (M6): adjacent REMOVE+INSERT pair — the language-level   */
/* faithful position-overwrite simulation (doc 98; design doc 74       */
/* E12(2), doc 75 E6/E8). plan()'s statement-array sequence walk is    */
/* the single recognition source: a pair whose receiver is one List    */
/* slot and whose structurally equal index expressions never touch     */
/* that slot lowers to ONE guarded `sd`; the insert statement is fully */
/* subsumed. Everything else keeps the opcode-named reject.            */
/* ------------------------------------------------------------------ */

/// One recognized pair, recorded by plan() on the REMOVE statement node
/// (post-WRAP address — the node the statement dispatch sees).
#[derive(Debug, Clone, Copy)]
struct PairRec {
    /// shared receiver slot (the paired touch forces the Growable class)
    slot: usize,
    /// both index exprs fold to this literal; None = the variable-index
    /// form (the expression is evaluated once at the remove root)
    const_idx: Option<u64>,
}

/// plan()'s pair marks: Root on the REMOVE statement (the fused emission
/// site), Skip on the INSERT statement (fully subsumed — zero code). The
/// lowering consumes them at the two statement-sequence dispatch sites
/// (the function-body loop and the block arm); a statement walk that
/// bypasses the sibling context must never see these opcodes unmarked
/// (unmarked REMOVE/INSERT are plan-rejected).
#[derive(Default, Clone)]
struct PairPlan {
    root: HashMap<usize, PairRec>,
    skip: HashSet<usize>,
}

fn fuse_unwrap(node: &dyn IRNode) -> &dyn IRNode {
    // WRAP is transparent; CU64 is the identity cast on Int (rv64 values
    // are 64-bit) and carries no runtime guard. CU8/16/32 NARROW with a
    // runtime 71 face — deliberately NOT unwrapped (fail-safe: a cast
    // wrapped leaf is simply not recognized).
    let mut cur = node;
    loop {
        if let Some(w) = cur.as_any().downcast_ref::<IRNodeWrapOne>() {
            cur = &*w.node;
            continue;
        }
        if matches!(Bytecode::try_from_u8(cur.bytecode()), Ok(Bytecode::CU64)) {
            if let Some(c) = cur.as_any().downcast_ref::<IRNodeSingle>() {
                cur = &*c.subx;
                continue;
            }
        }
        return cur;
    }
}

fn fuse_const(node: &dyn IRNode) -> Option<u64> {
    crate::lang::ir_literal_value(node)
        .ok()
        .flatten()
        .and_then(|v| v.extract_u128().ok())
        .filter(|x| *x <= u64::MAX as u128)
        .map(|x| x as u64)
}

/// A `byte(recv, idx)` leaf: returns (receiver node, index node).
fn fuse_byte_leaf(node: &dyn IRNode) -> Option<(&dyn IRNode, &dyn IRNode)> {
    let b = fuse_unwrap(node);
    if !matches!(Bytecode::try_from_u8(b.bytecode()), Ok(Bytecode::BYTE)) {
        return None;
    }
    let n = b.as_any().downcast_ref::<IRNodeDouble>()?;
    Some((&*n.subx, &*n.suby))
}

/// The receiver must be a GET/GET0-3 of one Bytes|Input slot; returns
/// the slot index.
fn fuse_recv_slot(node: &dyn IRNode, slot: &[SlotTy]) -> Option<usize> {
    let idx = fuse_get_slot(node)?;
    match slot.get(idx)? {
        SlotTy::Input | SlotTy::Bytes => Some(idx),
        _ => None,
    }
}

/// Raw GET slot index (any slot type — used for the chain accumulator).
fn fuse_get_slot(node: &dyn IRNode) -> Option<usize> {
    let g = fuse_unwrap(node);
    let op = Bytecode::try_from_u8(g.bytecode()).ok()?;
    match op {
        Bytecode::GET => Some(g.as_any().downcast_ref::<IRNodeParam1>()?.para as usize),
        Bytecode::GET0 => Some(0),
        Bytecode::GET1 => Some(1),
        Bytecode::GET2 => Some(2),
        Bytecode::GET3 => Some(3),
        _ => None,
    }
}

/// Deterministic structural key (canonical serialization) used to pin
/// the var-base form: leaf k's index must be `ADD(BASE, k)` with the
/// SAME BASE subtree as leaf 0.
fn fuse_struct_key(node: &dyn IRNode) -> String {
    let u = fuse_unwrap(node);
    let op = Bytecode::try_from_u8(u.bytecode());
    let mut s = format!("{:?}", op);
    if let Some(v) = fuse_const(u) {
        s.push_str(&format!("#lit{v}"));
        return s;
    }
    for c in children(u) {
        s.push('(');
        s.push_str(&fuse_struct_key(c));
        s.push(')');
    }
    s
}

fn fuse_nil_ok(node: &dyn IRNode, nilp: &NilPlan) -> bool {
    !nilp.occ.get(&node_addr(fuse_unwrap(node))).map(|o| o.val).unwrap_or(false)
}

/// Try to recognize a byte-assembly group rooted at `node` (an
/// expression-position value root). Returns the form + params.
fn fuse_recognize_expr(
    node: &dyn IRNode,
    slot: &[SlotTy],
    nilp: &NilPlan,
) -> Option<(FuseForm, usize, u64, u64)> {
    let root = fuse_unwrap(node);
    let op = Bytecode::try_from_u8(root.bytecode()).ok()?;
    match op {
        Bytecode::BOR | Bytecode::BXOR => {
            fuse_recognize_le_expr(root, slot, nilp)
        }
        Bytecode::ADD => fuse_recognize_be(root, slot, nilp),
        _ => None,
    }
}

/// LE |/^ tree flatten: (recv, idx, shift) per term — BOR/BXOR internal
/// nodes free, terms are bare byte leaves (shift 0) or BSHL of one.
/// Shared by recognition and fused emission (deterministic on the tree).
fn fuse_flatten_le<'a>(
    node: &'a dyn IRNode,
    terms: &mut Vec<(&'a dyn IRNode, &'a dyn IRNode, u64)>,
) -> Option<()> {
    match Bytecode::try_from_u8(fuse_unwrap(node).bytecode()) {
        Ok(Bytecode::BOR) | Ok(Bytecode::BXOR) => {
            let n = fuse_unwrap(node).as_any().downcast_ref::<IRNodeDouble>()?;
            fuse_flatten_le(&*n.subx, terms)?;
            fuse_flatten_le(&*n.suby, terms)
        }
        _ => {
            let u = fuse_unwrap(node);
            if matches!(Bytecode::try_from_u8(u.bytecode()), Ok(Bytecode::BSHL)) {
                let d = u.as_any().downcast_ref::<IRNodeDouble>()?;
                let sh = fuse_const(&*d.suby)?;
                let (recv, idx) = fuse_byte_leaf(&*d.subx)?;
                terms.push((recv, idx, sh));
            } else {
                let (recv, idx) = fuse_byte_leaf(node)?;
                terms.push((recv, idx, 0));
            }
            Some(())
        }
    }
}

/// BE mul-add tree flatten: (recv, idx, mult) per term — ADD internal
/// nodes free, terms are `byte·lit` (either operand order) or a bare
/// byte leaf (mult 1).
fn fuse_flatten_be<'a>(
    node: &'a dyn IRNode,
    terms: &mut Vec<(&'a dyn IRNode, &'a dyn IRNode, u64)>,
) -> Option<()> {
    match Bytecode::try_from_u8(fuse_unwrap(node).bytecode()) {
        Ok(Bytecode::ADD) => {
            let n = fuse_unwrap(node).as_any().downcast_ref::<IRNodeDouble>()?;
            fuse_flatten_be(&*n.subx, terms)?;
            fuse_flatten_be(&*n.suby, terms)
        }
        Ok(Bytecode::MUL) => {
            let n = fuse_unwrap(node).as_any().downcast_ref::<IRNodeDouble>()?;
            if let Some((recv, idx)) = fuse_byte_leaf(&*n.subx) {
                let m = fuse_const(&*n.suby)?;
                terms.push((recv, idx, m));
            } else if let Some((recv, idx)) = fuse_byte_leaf(&*n.suby) {
                let m = fuse_const(&*n.subx)?;
                terms.push((recv, idx, m));
            } else {
                return None;
            }
            Some(())
        }
        _ => {
            let (recv, idx) = fuse_byte_leaf(node)?;
            terms.push((recv, idx, 1));
            Some(())
        }
    }
}

/// LE |/^ tree: flatten the tree, require N ∈ {2,4,8} canonical terms
/// (offset c0+k with shift exactly 8k), one receiver.
fn fuse_recognize_le_expr(
    root: &dyn IRNode,
    slot: &[SlotTy],
    nilp: &NilPlan,
) -> Option<(FuseForm, usize, u64, u64)> {
    let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64)> = Vec::new();
    fuse_flatten_le(root, &mut terms)?;
    fuse_finish_le(terms, slot, nilp, FuseForm::LeExpr)
}

fn fuse_finish_le(
    terms: Vec<(&dyn IRNode, &dyn IRNode, u64)>,
    slot: &[SlotTy],
    nilp: &NilPlan,
    form: FuseForm,
) -> Option<(FuseForm, usize, u64, u64)> {
    let n = terms.len() as u64;
    if !matches!(n, 2 | 4 | 8) {
        return None;
    }
    let (recv0, idx0, sh0) = terms[0];
    let rs = fuse_recv_slot(recv0, slot)?;
    if !fuse_nil_ok(recv0, nilp) {
        return None;
    }
    let c0 = fuse_const(idx0)?;
    for (k, (recv, idx, sh)) in terms.iter().enumerate() {
        let k = k as u64;
        if fuse_recv_slot(*recv, slot) != Some(rs) || !fuse_nil_ok(*recv, nilp) {
            return None;
        }
        let c = fuse_const(*idx)?;
        if c != c0 + k || *sh != 8 * k {
            return None;
        }
        // the byte value and its index must both be non-Nil (the leaf's
        // 72 face is absent only for a non-Nil index)
        if !fuse_nil_ok(*idx, nilp) {
            return None;
        }
    }
    let _ = sh0;
    Some((form, rs, c0, n))
}

/// BE mul-add tree: flatten ADD terms, require N ∈ {2,4}, per-position
/// multiplier 2^(8·(N-1-k)), consecutive offsets. Const indexes →
/// BeConst; leaf0 = BASE / leaf_k = ADD(BASE,k) → BeVar.
fn fuse_recognize_be(
    root: &dyn IRNode,
    slot: &[SlotTy],
    nilp: &NilPlan,
) -> Option<(FuseForm, usize, u64, u64)> {
    let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64)> = Vec::new(); // (recv, idx, mult)
    fuse_flatten_be(root, &mut terms)?;
    let n = terms.len() as u64;
    if !matches!(n, 2 | 4) {
        return None;
    }
    let (recv0, idx0, m0) = terms[0];
    let rs = fuse_recv_slot(recv0, slot)?;
    if !fuse_nil_ok(recv0, nilp) {
        return None;
    }
    // per-term receiver/nil/offset/consecutiveness + multiplier pin
    for (k, (recv, idx, m)) in terms.iter().enumerate() {
        let k = k as u64;
        if fuse_recv_slot(*recv, slot) != Some(rs) || !fuse_nil_ok(*recv, nilp) {
            return None;
        }
        if *m != 1u64 << (8 * (n - 1 - k)) {
            return None;
        }
        if !fuse_nil_ok(*idx, nilp) {
            return None;
        }
    }
    // const-index form: every index is a literal
    let consts: Option<Vec<u64>> =
        terms.iter().map(|(_, idx, _)| fuse_const(*idx)).collect();
    if let Some(cs) = consts {
        let c0 = cs[0];
        for (k, c) in cs.iter().enumerate() {
            if *c != c0 + k as u64 {
                return None;
            }
        }
        return Some((FuseForm::BeConst, rs, c0, n));
    }
    // var-base form: leaf0 index is BASE; leaf k index = ADD(BASE, k)
    // with the SAME BASE subtree (structural key), and BASE non-Nil
    let base = idx0;
    if !fuse_nil_ok(base, nilp) {
        return None;
    }
    let key0 = fuse_struct_key(base);
    for (k, (_, idx, _)) in terms.iter().enumerate().skip(1) {
        let u = fuse_unwrap(*idx);
        if !matches!(Bytecode::try_from_u8(u.bytecode()), Ok(Bytecode::ADD)) {
            return None;
        }
        let d = u.as_any().downcast_ref::<IRNodeDouble>()?;
        let (a, b) = (&*d.subx, &*d.suby);
        let (other, off) = if fuse_const(a) == Some(k as u64) {
            (b, fuse_const(a)?)
        } else if fuse_const(b) == Some(k as u64) {
            (a, fuse_const(b)?)
        } else {
            return None;
        };
        if off != k as u64 || fuse_struct_key(other) != key0 || !fuse_nil_ok(*idx, nilp) {
            return None;
        }
    }
    Some((FuseForm::BeVar, rs, 0, n))
}

/// Statement-chain recognition (LeChain): consecutive PUTs
/// `PUT s, byte(recv,c0)` / `PUT s, s|byte(recv,c0+k)<<8k`.
/// Returns the consumed statement count.
#[allow(clippy::too_many_arguments)]
fn fuse_try_chain(
    subs: &[Box<dyn IRNode>],
    start: usize,
    keep_last: bool,
    slot: &[SlotTy],
    nilp: &NilPlan,
    marks: &mut FuseMap,
) -> Option<usize> {
    #[allow(clippy::too_many_arguments)]
    fn get_put<'a>(
        n: &'a dyn IRNode,
        slot: &[SlotTy],
        tag: &str,
    ) -> Option<(usize, &'a dyn IRNode)> {
        let u = fuse_unwrap(n);
        if !matches!(Bytecode::try_from_u8(u.bytecode()), Ok(Bytecode::PUT)) {
            return None;
        }
        if u.hasretval() {
            return None; // value-position PUT: never a statement chain link
        }
        let p = u.as_any().downcast_ref::<IRNodeParam1Single>()?;
        let s = p.para as usize;
        match slot.get(s) {
            Some(SlotTy::Int) => {}
            _ => return None,
        }
        Some((s, &*p.subx))
    }
    // statement 0: PUT s, byte(recv, c0)
    let Some((s_slot, v0)) = get_put(&*subs[start], slot, &format!("start{start}")) else {
        return None;
    };
    let Some((recv0, idx0)) = fuse_byte_leaf(v0) else {
        let u = fuse_unwrap(v0);
        return None;
    };
    let Some(rs) = fuse_recv_slot(recv0, slot) else {
        return None;
    };
    if rs == s_slot {
        return None; // the byte receiver cannot be the accumulator slot itself
    }
    if !fuse_nil_ok(recv0, nilp) || !fuse_nil_ok(idx0, nilp) {
        return None;
    }
    let Some(c0) = fuse_const(idx0) else {
        return None;
    };
    // extend: PUT s, BOR(GET s, BSHL(byte(recv, c0+k), 8k)) — both operand orders
    let mut k = 1u64;
    while start + (k as usize) < subs.len() {
        if keep_last && start + (k as usize) + 1 == subs.len() {
            break; // the chain tail must never be the kept value
        }
        let Some((s2, v)) = get_put(&*subs[start + k as usize], slot, &format!("ext s{start}+{k}")) else {
            break;
        };
        if s2 != s_slot {
            break;
        }
        let Ok(bop) = Bytecode::try_from_u8(fuse_unwrap(v).bytecode()) else { break };
        if bop != Bytecode::BOR {
            break;
        }
        let Some(d) = fuse_unwrap(v).as_any().downcast_ref::<IRNodeDouble>() else {
            break;
        };
        let (x, y) = (&*d.subx, &*d.suby);
        let (acc, term) = if fuse_get_slot(x).is_some() {
            (x, y)
        } else {
            (y, x)
        };
        let acc_slot = fuse_get_slot(acc);
        if acc_slot != Some(s_slot) || !fuse_nil_ok(acc, nilp) {
            break;
        }
        // term: BSHL(byte(recv, c0+k), 8k) or a bare byte at shift 0 (k=0 impossible here)
        let tu = fuse_unwrap(term);
        let (leaf, want_shift) =
            if matches!(Bytecode::try_from_u8(tu.bytecode()), Ok(Bytecode::BSHL)) {
                let Some(dd) = tu.as_any().downcast_ref::<IRNodeDouble>() else {
                    break;
                };
                (&*dd.subx, fuse_const(&*dd.suby))
            } else {
                (term, None)
            };
        let Some((recv, idx)) = fuse_byte_leaf(leaf) else {
            break;
        };
        if fuse_recv_slot(recv, slot) != Some(rs)
            || !fuse_nil_ok(recv, nilp)
            || !fuse_nil_ok(idx, nilp)
        {
            break;
        }
        let Some(c) = fuse_const(idx) else {
            break;
        };
        if c != c0 + k || want_shift != Some(8 * k) {
            break;
        }
        k += 1;
    }
    let n = k;
    if !matches!(n, 2 | 4 | 8) {
        return None;
    }
    // mark: root on statement 0's value top (post-WRAP), Skip on the rest
    let (_, vtop) = get_put(&*subs[start], slot, "mark")?;
    marks.insert(
        node_addr(fuse_unwrap(vtop)),
        FuseMark::Root { form: FuseForm::LeChain, c0, n },
    );
    for j in 1..n as usize {
        // Skip is keyed on the STATEMENT node (post-WRAP) — the node
        // stmt_one_inner dispatches on
        marks.insert(node_addr(fuse_unwrap(&*subs[start + j])), FuseMark::Skip);
    }
    Some(n as usize)
}

/// The scan: walk the statement tree; per statement array try chains,
/// per expression node try the value-root forms. Maximal-root policy:
/// once a group root is marked, its subtree is not scanned further.
fn fuse_scan_nodes(
    subs: &[Box<dyn IRNode>],
    keep_last: bool,
    slot: &[SlotTy],
    nilp: &NilPlan,
    marks: &mut FuseMap,
) {
    let mut i = 0usize;
    while i < subs.len() {
        if let Some(consumed) = fuse_try_chain(subs, i, keep_last, slot, nilp, marks) {
            i += consumed;
            continue;
        }
        fuse_scan_node(&*subs[i], slot, nilp, marks);
        i += 1;
    }
}

fn fuse_scan_node(
    node: &dyn IRNode,
    slot: &[SlotTy],
    nilp: &NilPlan,
    marks: &mut FuseMap,
) {
    // expression-root recognition on the post-WRAP dispatch node
    let u = fuse_unwrap(node);
    if let Some((form, rs, c0, n)) = fuse_recognize_expr(u, slot, nilp) {
        let _ = rs;
        marks.insert(node_addr(u), FuseMark::Root { form, c0, n });
        return; // maximal-root: do not descend into the group
    }
    let Ok(op) = Bytecode::try_from_u8(u.bytecode()) else { return };
    // statement arrays carry the keep-last flag (IRLIST/IRBLOCKR keep
    // their tail value; a chain ending there is refused inside the scan)
    if matches!(op, Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR) {
        let keep = op != Bytecode::IRBLOCK;
        if let Some(arr) = u.as_any().downcast_ref::<IRNodeArray>() {
            // the PACKLIST creation wrapper ([elems…, count, PACKLIST]) has
            // no statement shape — chains cannot start, scan as plain nodes
            fuse_scan_nodes(&arr.subs, keep, slot, nilp, marks);
            return;
        }
    }
    for c in children(u) {
        fuse_scan_node(c, slot, nilp, marks);
    }
}

/// Entry: compute the fusion map for one body.
fn fuse_scan(nodes: &[Box<dyn IRNode>], slot: &[SlotTy], nilp: &NilPlan) -> FuseMap {
    let mut marks: FuseMap = HashMap::new();
    fuse_scan_nodes(nodes, false, slot, nilp, &mut marks);
    if diag_env_enabled("RV64_FUSE_DEBUG") {
        println!("  fuse_scan: {} marks", marks.len());
        for (addr, m) in &marks {
            println!("    {addr:x}: {m:?}");
        }
    }
    marks
}

/// Static result type of a node under the pinned subset.
fn node_res_ty(node: &dyn IRNode, slot: &[SlotTy]) -> Ret<Ty> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return node_res_ty(&*w.node, slot);
    }
    let op = Bytecode::try_from_u8(node.bytecode())
        .map_err(|e| format!("rv64 lower: {e}"))?;
    use Bytecode::*;
    let ty = match op {
        P0 | P1 | P2 | P3 | PU8 | PU16 | PTRUE | PFALSE => Ty::Int,
        PBUF | PBUFL | PNBUF => Ty::Bytes,
        CU8 | CU16 | CU32 | CU64 => Ty::Int,
        CBYTES => Ty::Bytes,
        ADD | SUB | MUL | DIV | MOD | POW | MAX | MIN | CLAMP | ABSDIFF | DIVUP | DIVEXACT
        | BSHL | BSHR | BXOR | BOR | BAND | EQ | NEQ | LT | GT | LE | GE | NOT | AND | OR => {
            Ty::Int
        }
        BYTE | SIZE => Ty::Int,
        CAT | CUT | LEFT | RIGHT | LDROP | RDROP => Ty::Bytes,
        // P3-V2IMPL: the admitted v2 list shapes (plan() has already
        // validated them by the time lowering asks). ITEMGET/LENGTH/HASKEY
        // yield scalars; the creation/mutation ops yield the list itself.
        ITEMGET | LENGTH | HASKEY => Ty::Int,
        // P3-PAIRPKG: remove/insert produce the receiver itself (the
        // interpreter pops only the arguments; the compo stays on the
        // stack) — needed by the block drop accounting for a pair root.
        REMOVE | INSERT => Ty::List,
        NEWLIST | PACKLIST | APPEND | CLEAR => Ty::List,
        CHOOSE | IRIFR => {
            let n = node
                .as_any()
                .downcast_ref::<IRNodeTriple>()
                .ok_or("rv64 lower: choose node shape unexpected")?;
            node_res_ty(&*n.suby, slot)?
        }
        IRBLOCK | IRBLOCKR | IRLIST => {
            let n = node
                .as_any()
                .downcast_ref::<IRNodeArray>()
                .ok_or("rv64 lower: block node shape unexpected")?;
            match n.subs.last() {
                Some(last) if last.hasretval() => node_res_ty(&**last, slot)?,
                _ => return errf!("rv64 lower: block used as a value has no value child"),
            }
        }
        GET | GET0 | GET1 | GET2 | GET3 => {
            let idx = match op {
                GET => node
                    .as_any()
                    .downcast_ref::<IRNodeParam1>()
                    .ok_or("rv64 lower: GET node shape unexpected")?
                    .para,
                GET0 => 0,
                GET1 => 1,
                GET2 => 2,
                _ => 3,
            };
            match slot.get(idx as usize) {
                Some(SlotTy::Bytes) | Some(SlotTy::Input) => Ty::Bytes,
                Some(SlotTy::List) => Ty::List,
                _ => Ty::Int,
            }
        }
        XOP => Ty::Int,
        other => {
            return errf!(
                "rv64 lower: opcode {:?} is outside the translatable kernel subset",
                other
            )
        }
    };
    Ok(ty)
}

/// Extract a bytes literal from a node (PBUF/PBUFL payload, PNBUF → empty).
fn bytes_literal(node: &dyn IRNode) -> Option<Vec<u8>> {
    if let Some(leaf) = node.as_any().downcast_ref::<IRNodeLeaf>() {
        return matches!(leaf.inst, Bytecode::PNBUF).then(Vec::new);
    }
    let p = node.as_any().downcast_ref::<IRNodeParams>()?;
    match p.inst {
        Bytecode::PBUF => Some(p.para[1..].to_vec()),
        Bytecode::PBUFL => Some(p.para[2..].to_vec()),
        _ => None,
    }
}

fn lxop_inst(op: LxOp) -> Bytecode {
    match op {
        LxOp::Add => Bytecode::ADD,
        LxOp::Sub => Bytecode::SUB,
        LxOp::Mul => Bytecode::MUL,
        LxOp::Div => Bytecode::DIV,
    }
}

/* ------------------------------------------------------------------ */
/* Public result types                                                 */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy)]
pub struct Rv64Stats {
    pub insts: usize,
    pub code_bytes: usize,
    /// Sum of consensus `inst_cycles` over the re-decoded RX stream.
    pub cycles: u64,
    /// Local slots in the frame.
    pub locals: usize,
}

#[derive(Debug)]
pub struct Rv64Lowered {
    /// RX instruction stream (entry at offset 0; 4-byte words in `Im`, a
    /// 2-byte-aligned mix of compressed and full words in `Imc*`).
    pub code: Vec<u8>,
    /// Read-only literal pool (empty when the body has no bytes literals).
    pub pool: Vec<u8>,
    /// RW arena size declared by `pack_segments` (ARENA_SIZE or BIG).
    pub arena_size: u64,
    pub stats: Rv64Stats,
}

impl Rv64Lowered {
    /// Pack into an admitted calc container: RX code + R pool + RW arena bss.
    /// The container profile follows the lowering mode (0 for `Im`, 2 for `Imc*`).
    pub fn pack_segments(
        &self,
        cap: &SpaceCap,
        mem_kib: u32,
        input_max: u16,
        output_max: u16,
    ) -> VmrtRes<container::CalcContainer> {
        self.pack_segments_profile(cap, mem_kib, input_max, output_max, container::CALC_PROFILE_RV64IM)
    }

    pub fn pack_segments_profile(
        &self,
        cap: &SpaceCap,
        mem_kib: u32,
        input_max: u16,
        output_max: u16,
        profile: u8,
    ) -> VmrtRes<container::CalcContainer> {
        let mut segs = vec![CalcSegment::rx(CODE_BASE as u32, self.code.clone())];
        let pool_vaddr = CODE_BASE as u32 + self.code.len() as u32;
        if !self.pool.is_empty() {
            segs.push(CalcSegment::ro(pool_vaddr, self.pool.clone()));
        }
        segs.push(CalcSegment::rw(ARENA_BASE as u32, Vec::new(), self.arena_size as u32));
        crate::calc::pack_container(
            cap,
            profile,
            mem_kib,
            CODE_BASE as u32,
            input_max,
            output_max,
            segs,
        )
    }
}

/// HVM-channel stats for the same IR: bytecode size and the per-opcode base
/// compute gas (static sum; the HVM side's dynamic byte-metered extras are
/// NOT included, so the HVM compute number is a lower bound — the comparison
/// is conservative in the HVM channel's favor).
pub fn hvm_bytecode_stats(codes: &[u8]) -> Ret<(usize, i64)> {
    let gst = GasTable::new(0);
    let mut pc = 0usize;
    let mut gas = 0i64;
    while pc < codes.len() {
        let op = Bytecode::try_from_u8(codes[pc])?;
        pc += 1;
        pc += match op {
            Bytecode::PBUF => 1 + codes[pc] as usize,
            Bytecode::PBUFL => 2 + u16::from_be_bytes([codes[pc], codes[pc + 1]]) as usize,
            _ => op.metadata().param as usize,
        };
        gas += gst.gas(op as u8);
    }
    Ok((codes.len(), gas))
}

/// Decode `code` with the consensus decoder for `profile` and sum cycle weights.
fn rv_code_stats(code: &[u8], profile: u8, locals: usize) -> Ret<Rv64Stats> {
    let mut off = 0usize;
    let mut insts = 0usize;
    let mut cycles = 0u64;
    while off < code.len() {
        let (inst, len) = decode_at(code, off, profile)?;
        cycles += inst_cycles(&inst);
        insts += 1;
        off += len as usize;
    }
    if off != code.len() {
        return errf!("rv64 lower: code stream does not end on an instruction boundary");
    }
    Ok(Rv64Stats { insts, code_bytes: code.len(), cycles, locals })
}

/* ------------------------------------------------------------------ */
/* A-19 batch 2: dominated constant-mul overflow-check merging          */
/*                                                                     */
/* `x * k` overflow guards on the SAME base register form a domination  */
/* lattice: for k1 < k2 (unsigned, both >= 2),                         */
/*   k1*x > 2^64-1  ==>  x >= ceil(2^64/k1) >= ceil(2^64/k2)           */
/*                     ==>  k2*x >= 2^64                               */
/* so the trap-input set of the k1 check is a subset of the k2 check's */
/* (mechanical arithmetic fact). When the k2 check executes later in   */
/* the same straight-line region, deleting the k1 check only moves the */
/* trap site forward — same iteration, same exit code 90 — which is    */
/* observably identical PROVIDED nothing consensus-observable happens  */
/* between the sites. The three release proofs (main-agent ruling      */
/* 2026-11-14) are built in here, not prose:                           */
/*   (a) constant domination table — k_dominated < k_retained on the   */
/*       same base register (the integer argument above);              */
/*   (b) same-base def-use — no instruction between the sites writes   */
/*       the base register (the copies feeding both checks read the    */
/*       same value);                                                   */
/*   (c) interval scan — every instruction between the sites is pure   */
/*       compute, a frame sd (non-observable by the red-line           */
/*       definition), or an arith-thunk guard branch (exit 90). Arena  */
/*       or out stores, ecall, and any other control flow veto the     */
/*       merge, as do reads of the deleted check's scratch register    */
/*       (its stale value must never be consumed before the retained   */
/*       check redefines the scratch discipline).                      */
/* Deleting bytes mid-buffer invalidates backward branches that hop    */
/* over the cut (their displacement was baked at emission); those are  */
/* re-patched with the shrunk displacement, which always still fits    */
/* (removing bytes between target and site can only narrow it).        */
/* The whole pass is a deterministic function of the emitted stream — */
/* no iteration-order dependence, re-derived per relaxation pass.      */
/* ------------------------------------------------------------------ */

/// Classified instruction kind between a dominated and its dominating
/// check (the interval-scan artifact pinned by the `#[cfg(test)]`
/// mechanical-proof tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulMergeIvKind {
    /// pure compute or a load (loads have no guest-visible side effect)
    Pure,
    /// `sd` off the locals frame base (frame sd/ld are non-observable)
    FrameSd,
    /// a branch to the arith fault thunk — a retained exit-90 guard
    GuardArith,
}

/// One applied dominated-check deletion. Offsets are pre-deletion
/// positions in the pass's emission stream (diagnostics only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MulMergeRecord {
    /// base register both checks' value copies read (same value by (b))
    pub base_reg: u8,
    /// constant of the DELETED (dominated) check
    pub k_dominated: u64,
    /// constant of the RETAINED (dominating) check
    pub k_retained: u64,
    /// pre-deletion offset of the dominated check's first instruction
    pub dom_at: usize,
    /// pre-deletion offset of the retained check's first instruction
    pub ret_at: usize,
    /// classified instructions strictly between the two checks (proof (c))
    pub interval: Vec<MulMergeIvKind>,
}

/// Register an instruction writes, if any (the def-use scanner).
fn inst_rd(i: &Inst) -> Option<u8> {
    use crate::calc::decode::Inst::*;
    Some(match i {
        Lui { rd, .. }
        | Auipc { rd, .. }
        | Jal { rd, .. }
        | Jalr { rd, .. }
        | Load { rd, .. }
        | ImmAlu { rd, .. }
        | ShiftImm { rd, .. }
        | RegAlu { rd, .. }
        | UnAlu { rd, .. }
        | BitImm { rd, .. }
        | Slliuw { rd, .. } => *rd,
        Store { .. } | Branch { .. } | Ecall | Fence | Trap => return None,
    })
}

/// Registers an instruction reads (at most two; the veto scanners).
fn inst_rs(i: &Inst) -> (Option<u8>, Option<u8>) {
    use crate::calc::decode::Inst::*;
    match i {
        Branch { rs1, rs2, .. } | RegAlu { rs1, rs2, .. } => (Some(*rs1), Some(*rs2)),
        Store { rs1, rs2, .. } => (Some(*rs1), Some(*rs2)),
        Load { rs1, .. }
        | ImmAlu { rs1, .. }
        | ShiftImm { rs1, .. }
        | UnAlu { rs1, .. }
        | BitImm { rs1, .. }
        | Slliuw { rs1, .. }
        | Jalr { rs1, .. } => (Some(*rs1), None),
        _ => (None, None),
    }
}

/// One recognized constant-mul overflow check in the emitted stream.
struct MulCheckSite {
    base: u8,
    k: u64,
    /// scratch holding the check value (dead after the branch; reads of it
    /// in a merge interval veto the merge)
    sc: u8,
    /// byte span of the deletable pair (mulhu|srli ..= bne), pre-deletion
    span: (usize, usize),
    /// index of the first check instruction, then of the branch
    first_ix: usize,
    bne_ix: usize,
    deleted: bool,
}

/// Backward def-use recognizer for one check site: starting just before
/// `from`, the last writer of `d` must be `mv d, r` (r != 0, distinct from
/// d and t) and — when `t` is given — the last writer of `t` a plain
/// constant load >= 2. Anything else written into d/t first, or any write
/// of r between its copy and the check, vetoes (None).
///
/// The copy→check region is rescanned FORWARD before accepting: the
/// backward walk visits that region while `base` is still None (it only
/// resolves at the copy, which sits at a lower index), so an in-region
/// base rewrite would otherwise escape the veto and break the same-base
/// premise of the merge (found in main-agent acceptance review — the
/// review's counter-example is pinned by `a19_mulchk_base_write_between_
/// copy_and_check_vetoes`).
fn mul_check_base_const(insts: &[Inst], from: usize, d: u8, t: Option<u8>) -> Option<(u8, u64)> {
    use crate::calc::decode::{ImmAluOp, RegAluOp};
    const WINDOW: usize = 12;
    let mut base: Option<u8> = None;
    let mut k: Option<u64> = None;
    let mut copy_ix: usize = 0;
    let mut j = from;
    let mut steps = 0;
    while j > 0 && steps < WINDOW {
        j -= 1;
        steps += 1;
        let cur = &insts[j];
        let Some(rd) = inst_rd(cur) else { continue };
        if rd == d {
            let crate::calc::decode::Inst::RegAlu { op: RegAluOp::Add, rs1: 0, rs2: r, .. } = cur
            else {
                return None;
            };
            if *r == 0 || *r == d || t == Some(*r) {
                return None;
            }
            base = Some(*r);
            copy_ix = j;
        } else if t.is_some() && rd == t.unwrap() {
            match cur {
                crate::calc::decode::Inst::ImmAlu {
                    op: ImmAluOp::Addi, rs1: 0, imm, ..
                } if *imm >= 2 => k = Some(*imm as u64),
                crate::calc::decode::Inst::Lui { val, .. } if *val >= 2 => k = Some(*val as u64),
                _ => return None,
            }
        }
        if base.is_some() && (t.is_none() || k.is_some()) {
            // copy→check forward rescan: a base write in (copy_ix, from)
            // redefines the value after the copy captured it — the check
            // tests the OLD value while any later same-base site reads the
            // NEW one, so the trap-set inclusion premise breaks. Veto.
            for m in &insts[copy_ix + 1..from] {
                if inst_rd(m) == base {
                    return None;
                }
            }
            return Some((base.unwrap(), k.unwrap_or(0)));
        }
    }
    None
}

/// Decode a whole stream (positions, instructions, lengths).
fn decode_stream(buf: &[u8], profile: u8) -> Option<(Vec<usize>, Vec<Inst>, Vec<usize>)> {
    let mut offs = Vec::with_capacity(buf.len() / 2);
    let mut insts = Vec::with_capacity(buf.len() / 2);
    let mut lens = Vec::with_capacity(buf.len() / 2);
    let mut p = 0usize;
    while p < buf.len() {
        let (inst, len) = decode_at(buf, p, profile).ok()?;
        offs.push(p);
        insts.push(inst);
        lens.push(len as usize);
        p += len as usize;
    }
    Some((offs, insts, lens))
}

/// funct3 of a decoded branch kind (the `branch_word` encoding table).
fn brkind_f3(kind: crate::calc::decode::BrKind) -> u32 {
    use crate::calc::decode::BrKind::*;
    match kind {
        Eq => 0,
        Ne => 1,
        Lt => 4,
        Ge => 5,
        Ltu => 6,
        Geu => 7,
    }
}

/// Re-patch one backward branch/jump whose baked displacement hops a cut
/// `[s, e)`: the site sits above the cut, its target below, so draining
/// `cut` bytes narrows the displacement by exactly `cut` — which always
/// stays inside the same encoding window (only 4-byte jal can be baked
/// backward for far targets; c.j/bentries only shrink toward zero).
fn repatch_hopped_backward(asm: &mut Asm, profile: u8, s: usize, e: usize) {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else { return };
    let cut = e - s;
    for i in 0..insts.len() {
        let pos = offs[i];
        if pos < e {
            continue; // below (or inside — impossible) the cut: unaffected
        }
        // fixup-backed branches are shifted by the caller; only baked
        // (backward-resolved) ones need re-patching here
        if asm.fixups.iter().any(|f| f.at == pos) {
            continue;
        }
        let (new_rel, abs) = match &insts[i] {
            Inst::Branch { target, .. } | Inst::Jal { target, .. } => {
                (*target + cut as i64, pos as i64 + *target)
            }
            _ => continue,
        };
        if abs >= s as i64 {
            continue; // forward (+8 trampoline skip) or not hopping the cut
        }
        match &insts[i] {
            Inst::Branch { kind, rs1, rs2, .. } => {
                if lens[i] == 2 {
                    let cf3 = if matches!(kind, crate::calc::decode::BrKind::Eq) { 6 } else { 7 };
                    let patched = Asm::c_bz(cf3, *rs1, new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::branch_word(brkind_f3(*kind), *rs1, *rs2, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            Inst::Jal { rd, .. } => {
                if lens[i] == 2 {
                    let patched = Asm::c_j(new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::jal_word(*rd, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            _ => unreachable!("matched above"),
        }
    }
}

/// A-19 batch 2 driver: scan the emitted body stream, prove and apply the
/// dominated constant-mul check deletions. Returns the proof records
/// (empty when nothing merged). Deterministic: decisions depend only on
/// the instruction stream, never on iteration order.
fn merge_dominated_mul_checks(
    asm: &mut Asm,
    profile: u8,
    fault_arith: usize,
) -> Vec<MulMergeRecord> {
    use crate::calc::decode::{BrKind, Inst, RegAluOp, ShiftKind, StoreOp};
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else { return Vec::new() };
    // Guard branches to the arith thunk are forward refs at this point
    // (thunks are placed after the body), so the direct form carries a
    // fixup. Far guards (> ±4 KiB, big kernels) use the trampoline form:
    // an inverted branch baked to skip the next `jal`, which itself holds
    // the fault_arith fixup. Both forms are recognized here.
    let has_arith_fixup = |pos: usize| asm.fixups.iter().any(|f| f.at == pos && f.label == fault_arith);
    let trampoline_jal_end = |i: usize| -> Option<usize> {
        match &insts[i] {
            Inst::Branch { target, .. }
                if *target == 8
                    && i + 1 < insts.len()
                    && matches!(insts[i + 1], Inst::Jal { .. })
                    && has_arith_fixup(offs[i + 1]) =>
            {
                Some(offs[i + 1] + lens[i + 1])
            }
            _ => None,
        }
    };
    // ---- recognize check sites (proof input collection) ----
    let mut sites: Vec<MulCheckSite> = Vec::new();
    for i in 1..insts.len() {
        // the check guard is `bne sc, x0, →arith90`: direct (Ne + fixup)
        // or trampoline (inverted Eq skipping the jal)
        let Inst::Branch { rs1: sc, rs2: 0, .. } = insts[i] else { continue };
        let span_end = match &insts[i] {
            Inst::Branch { kind: BrKind::Ne, .. } if has_arith_fixup(offs[i]) => {
                Some(offs[i] + lens[i])
            }
            Inst::Branch { kind: BrKind::Eq, .. } => trampoline_jal_end(i),
            _ => None,
        };
        let Some(span_end) = span_end else { continue };
        let (first_ix, d, k, t_reg) = match insts[i - 1] {
            Inst::RegAlu { op: RegAluOp::Mulhu, rd, rs1, rs2 } if rd == sc => {
                (i - 1, rs1, None::<u64>, Some(rs2))
            }
            Inst::ShiftImm { kind: ShiftKind::Srl, rd, rs1, shamt, .. }
                if rd == sc && shamt >= 1 && shamt <= 63 =>
            {
                (i - 1, rs1, Some(1u64 << (64 - shamt as u32)), None)
            }
            _ => continue,
        };
        // value register: last writer must be a plain copy from a base
        // register; const operand (register form) must be a plain li >= 2
        let (base, k_reg) = match k {
            Some(kp) => match mul_check_base_const(&insts, first_ix, d, None) {
                Some((b, _)) => (b, kp),
                None => continue,
            },
            None => match mul_check_base_const(&insts, first_ix, d, t_reg) {
                Some((b, kk)) => (b, kk),
                None => continue,
            },
        };
        sites.push(MulCheckSite {
            base,
            k: k_reg,
            sc,
            span: (offs[first_ix], span_end),
            first_ix,
            bne_ix: i,
            deleted: false,
        });
    }
    // ---- prove and mark deletions (proofs (a)(b)(c)) ----
    let mut cuts: Vec<(usize, usize)> = Vec::new(); // spans, descending later
    let mut records = Vec::new();
    for ci in 0..sites.len() {
        for ei in 0..ci {
            if sites[ei].deleted {
                continue;
            }
            // (a) constant domination: k_e < k_c on the same base register
            if sites[ei].base != sites[ci].base || sites[ei].k >= sites[ci].k {
                continue;
            }
            let (e_bne_ix, e_base, e_sc, e_span) = {
                let e = &sites[ei];
                (e.bne_ix, e.base, e.sc, e.span)
            };
            let c_first_ix = sites[ci].first_ix;
            // (b)+(c): def-use + interval whitelist over the instructions
            // strictly between the two checks (a trampoline-form dominated
            // site's own jal is inside its span — start past the span end)
            let mut interval = Vec::new();
            let mut ok = true;
            let mut j = e_bne_ix + 1;
            while offs[j] < e_span.1 {
                j += 1;
            }
            while j < c_first_ix {
                let cur = &insts[j];
                if inst_rd(cur) == Some(e_base) {
                    ok = false; // (b) base redefined between the checks
                    break;
                }
                let (r1, r2) = inst_rs(cur);
                if r1 == Some(e_sc) || r2 == Some(e_sc) {
                    ok = false; // stale scratch consumed before redefinition
                    break;
                }
                let cls = match cur {
                    Inst::RegAlu { .. }
                    | Inst::ImmAlu { .. }
                    | Inst::ShiftImm { .. }
                    | Inst::Lui { .. }
                    | Inst::Auipc { .. }
                    | Inst::UnAlu { .. }
                    | Inst::BitImm { .. }
                    | Inst::Slliuw { .. }
                    | Inst::Load { .. } => MulMergeIvKind::Pure,
                    Inst::Store { op: StoreOp::Sd, rs1, .. } if *rs1 == S0 => {
                        MulMergeIvKind::FrameSd
                    }
                    Inst::Branch { .. } if has_arith_fixup(offs[j]) => {
                        MulMergeIvKind::GuardArith
                    }
                    Inst::Branch { .. } if trampoline_jal_end(j).is_some() => {
                        // arith guard in trampoline form: its jal is part of
                        // the guard — skip past it
                        interval.push(MulMergeIvKind::GuardArith);
                        j += 2;
                        continue;
                    }
                    _ => {
                        ok = false; // observable store / ecall / foreign flow
                        break;
                    }
                };
                interval.push(cls);
                j += 1;
            }
            if !ok {
                continue;
            }
            let (e_span, e_k, e_base_reg) = {
                let e = &sites[ei];
                (e.span, e.k, e.base)
            };
            sites[ei].deleted = true;
            cuts.push(e_span);
            records.push(MulMergeRecord {
                base_reg: e_base_reg,
                k_dominated: e_k,
                k_retained: sites[ci].k,
                dom_at: e_span.0,
                ret_at: sites[ci].span.0,
                interval,
            });
        }
    }
    // ---- apply cuts, highest position first ----
    cuts.sort_by(|a, b| b.0.cmp(&a.0));
    for (s, e) in cuts {
        let cut = e - s;
        // all previously applied cuts sit above this one, so (s, e) is
        // still addressable in the current buffer
        // paranoia: no label may live strictly inside the cut, and the
        // only fixups inside must be the deleted branch's own
        if asm.labels.iter().any(|l| l.map_or(false, |p| p > s && p < e)) {
            continue;
        }
        if !asm.fixups.iter().all(|f| {
            !(f.at > s && f.at < e)
                || matches!(f.kind, FixKind::Branch | FixKind::CBranchZ | FixKind::Jal)
        }) {
            continue;
        }
        if !asm.fixups.iter().any(|f| f.at >= s && f.at < e) {
            continue; // the guard branch fixup must be there to remove
        }
        // backward branches hopping the cut have baked displacements
        repatch_hopped_backward(asm, profile, s, e);
        asm.fixups.retain(|f| f.at < s || f.at >= e);
        asm.buf.drain(s..e);
        for l in asm.labels.iter_mut() {
            if let Some(p) = l {
                if *p >= e {
                    *p -= cut;
                }
            }
        }
        for f in asm.fixups.iter_mut() {
            if f.at >= e {
                f.at -= cut;
            }
        }
    }
    records
}

/* ---- P3-GUARDTAIL GT-a: guard thunk copies for 2-byte c.bz reach ---- */

/// One applied guard-copy cluster (GT-a pin record).
#[derive(Debug, Clone, PartialEq)]
pub struct GuardCopyRecord {
    /// exit code the copy materializes (90 = fault_arith, 71 = fault_cast)
    pub code: u64,
    /// final offset of the copy thunk (`li a0, code` first byte), in the
    /// finished body's coordinates
    pub copy_at: usize,
    /// copy tail form: `c.j` straight to the shared thunk (true, 6-byte
    /// copy) or a 4-byte `jal trap` (false, 8-byte copy)
    pub form_cj: bool,
    /// pre-transform offsets of the converted guard branches (original
    /// stream coordinates, ascending)
    pub sites: Vec<usize>,
}

/// Re-patch baked (backward-resolved) branches whose displacement hops an
/// inserted region `[s, e)`: the site sits above the insert, its target
/// below, so the displacement grows in magnitude by exactly `e - s` — the
/// mirror of [`repatch_hopped_backward`]. Grown displacements can leave an
/// encoding window; callers pre-check hopper windows (see
/// `guard_hoppers_ok`) and `validate_branch_windows` post-verifies the
/// whole stream, so any residual violation is a loud internal error, never
/// a silently mis-encoded branch.
fn repatch_widened_backward(asm: &mut Asm, profile: u8, s: usize, e: usize) {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else { return };
    let ins = e - s;
    for i in 0..insts.len() {
        let pos = offs[i];
        // fixup-backed branches are shifted by the caller; only baked
        // (already-resolved) ones need re-patching here
        if asm.fixups.iter().any(|f| f.at == pos) {
            continue;
        }
        let (old_rel, abs_pre) = match &insts[i] {
            Inst::Branch { target, .. } | Inst::Jal { target, .. } => {
                // reconstruct the PRE-insert absolute target in the right
                // frame: the splice already happened, so positions at/above
                // the insert carry the +ins shift while positions below do
                // not. Treating below-insert sites as shifted under-counted
                // abs_pre by `ins` — a trampoline hop landing exactly on the
                // insert start was then classified "unchanged" and left
                // hopping INTO the copy (`li a0,90` on a live path: the
                // generator campaign's spurious Err(90) mode-splits).
                let abs = if pos >= e {
                    pos as i64 - ins as i64 + *target
                } else {
                    pos as i64 + *target
                };
                (*target, abs)
            }
            _ => continue,
        };
        // two cases with a GROWING displacement (checked against encoding
        // windows by `guard_hoppers_ok`, post-verified by
        // `validate_branch_windows`):
        //   site above the insert, target below:  rel -= ins
        //   site below the insert, target at/above it (a spanner — e.g. a
        //   loop back-edge whose header the copy precedes): rel += ins
        // everything else (both sides on one side of the insert) is
        // unchanged.
        let new_rel = if pos >= e {
            if abs_pre >= s as i64 {
                continue; // both above: unchanged
            }
            old_rel - ins as i64
        } else {
            if abs_pre < s as i64 {
                continue; // both below: unchanged
            }
            old_rel + ins as i64
        };
        match &insts[i] {
            Inst::Branch { kind, rs1, rs2, .. } => {
                if lens[i] == 2 {
                    let cf3 = if matches!(kind, crate::calc::decode::BrKind::Eq) { 6 } else { 7 };
                    let patched = Asm::c_bz(cf3, *rs1, new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::branch_word(brkind_f3(*kind), *rs1, *rs2, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            Inst::Jal { rd, .. } => {
                if lens[i] == 2 {
                    let patched = Asm::c_j(new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::jal_word(*rd, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            _ => unreachable!("matched above"),
        }
    }
}

/// Encoding-window limit of a baked branch/jump by decoded shape.
fn baked_window(len: usize, is_branch: bool) -> i64 {
    if is_branch {
        if len == 2 { 254 } else { 4094 }
    } else if len == 2 {
        2046
    } else {
        0xF_FFFE
    }
}

/// Post-apply soundness net: every baked (fixup-free) branch/jump in the
/// stream must sit inside its encoding window. Fixup-backed slots are
/// window-checked by `finish`, so this covers exactly what the byte-level
/// ops above could have disturbed.
fn validate_branch_windows(asm: &Asm, profile: u8) -> Ret<()> {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else {
        return errf!("rv64 lower: internal code/pool layout error");
    };
    for i in 0..insts.len() {
        if asm.fixups.iter().any(|f| f.at == offs[i]) {
            continue;
        }
        let (rel, is_branch) = match &insts[i] {
            Inst::Branch { target, .. } => (*target, true),
            Inst::Jal { target, .. } => (*target, false),
            _ => continue,
        };
        if rel.abs() > baked_window(lens[i], is_branch) || rel & 1 != 0 {
            return errf!(
                "rv64 lower: internal code/pool layout error (baked branch {:#x} rel {} out of window)",
                offs[i],
                rel
            );
        }
    }
    Ok(())
}

/// A planned GT-a transformation in original-stream coordinates. `bound` is
/// the smallest original offset whose byte moves when the op applies (cuts:
/// `site+2` — the top half of the 4-byte branch drains; inserts: `p` —
/// everything at/after the copy shifts). Ops apply in descending `bound`
/// order, so lower ops' coordinates stay valid. `cluster` indexes
/// [`GuardCopyRecord`] / the copy label table.
struct GuardTailOp {
    bound: usize,
    delta: i64,
    cluster: usize,
    /// cut op: site offset (bytes [site, site+2) become the c.bz slot)
    site: Option<usize>,
    /// insert op: copy position, exit code, c.j form
    ins: Option<(usize, u64, bool)>,
}

/// One planned cluster before form commitment / application.
struct GuardTailPlan {
    code: u64,
    /// (offset, fixup seq, branch f3, compare register) per site, ascending
    sites: Vec<(usize, usize, u32, u8)>,
}

/// GT-a driver: give clustered zero-form guard branches (`bne/beq rs1, x0
/// → shared fault thunk`, direct 4-byte fixup form, rs1 in the RVC window)
/// a 2-byte `c.bz` path by inserting a copy thunk — `li a0, <code>` plus a
/// jump — within ±254 B of every site of its cluster.
///
/// Semantics: the copy materializes the SAME exit code into a0 and jumps to
/// the SAME shared thunk (which rewrites a0 with the same constant before
/// trapping), so every trap lands on the identical observable state;
/// taken/not-taken decisions still compare the same register against zero.
/// Pure encoding shape — trap timing and observability are per-value
/// unchanged.
///
/// Determinism: site recognition, clustering, copy placement and form are
/// pure functions of the post-merge instruction stream + the wide_c_ids
/// set (sites widened in a previous relaxation pass stay 4-byte; that set
/// only grows, so the whole plan is monotone-convergent). Planning runs in
/// three phases — cluster geometry (all labels combined, worst-case 8-byte
/// copies), then copy form per cluster with the full op set known, then
/// application — so overlapping clusters of different thunks cannot make a
/// c.j displacement drift out of its window. Re-derived identically on
/// every relaxation pass.
fn insert_guard_thunk_copies(
    asm: &mut Asm,
    profile: u8,
    fault_arith: usize,
    fault_cast: usize,
    trap_label: usize,
) -> Ret<Vec<GuardCopyRecord>> {
    use crate::calc::decode::{BrKind, Inst};
    // debug-only kill switch (same class as the other diag_env_enabled
    // knobs: compiled out of release builds, so production bytes and
    // determinism are untouched) — the rv96 scratch differential harness
    // lowers GT-on and GT-off streams in one process and compares them
    if diag_env_enabled("RV64_GT_DISABLE") {
        return Ok(Vec::new());
    }
    // RVC-only transform: a 2-byte c.bz does not exist in an uncompressed
    // profile, and its site scan would otherwise "convert" plain 4-byte
    // branches into undecodable C encodings (caught by the C3 generator's
    // Im-mode campaign: `internal code/pool layout error`)
    if profile & container::CALC_PROFILE_COMPRESSED == 0 {
        return Ok(Vec::new());
    }
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else {
        return Ok(Vec::new());
    };
    let body_len = asm.buf.len();
    // ---- phase 0: recognize sites (4B direct-fixup zero-form guards) ----
    // trampoline-form guards carry their fixup on the jal, not on the
    // inverted branch, and fall out of the recognition here
    let mut sites: Vec<(usize, usize, usize, u32, u8)> = Vec::new(); // off,label,seq,f3,rs1
    for i in 0..insts.len() {
        let (kind, rs1) = match &insts[i] {
            Inst::Branch { kind: k @ (BrKind::Eq | BrKind::Ne), rs1, rs2: 0, .. } => (k, *rs1),
            _ => continue,
        };
        if lens[i] != 4 || !rvc_win(rs1) {
            continue;
        }
        let Some(f) = asm
            .fixups
            .iter()
            .find(|f| f.at == offs[i] && f.kind == FixKind::Branch)
            .map(|f| (f.label, f.seq))
        else {
            continue;
        };
        let (label, seq) = f;
        if label != fault_arith && label != fault_cast {
            continue;
        }
        // A zero-form window-register guard reaches here only in its
        // WIDENED form: at emission such a site is an optimistic 2-byte
        // c.bz slot (CBranchZ fixup), and `finish` widens it to this 4-byte
        // Branch fixup exactly when the shared thunk sits outside the
        // ±256 B window (seq ∈ wide_c_ids by construction). Those are the
        // proven-unreachable sites a copy thunk serves; the not-yet-widened
        // slots keep their original target and need no copy.
        sites.push((offs[i], label, seq, brkind_f3(*kind), rs1));
    }
    if sites.is_empty() {
        return Ok(Vec::new());
    }
    // ---- phase 1: cluster per thunk label (strict 510 B windows), then
    // resolve feasibility with worst-case 8-byte copies; drop the farthest
    // site of an infeasible or unprofitable window (deterministic) ----
    let mut plans: Vec<GuardTailPlan> = Vec::new();
    let mut accepted_ops: Vec<(usize, i64)> = Vec::new(); // (bound, delta)
    // O(1) shift lookup: prefix sums over a difference array indexed by
    // stream position, rebuilt whenever accepted_ops grows. A linear
    // shift_at here made phase 1 O(sites^3) on guard-heavy bodies
    // (~2200 sites -> 26 min CPU spin, caught on a19 bulk relaxation).
    let mut shift_prefix = vec![0i64; asm.buf.len() + 1];
    let virt_shift = |extra: &[(usize, i64)], x: usize| -> i64 {
        extra.iter().filter(|o| o.0 <= x).map(|o| o.1).sum()
    };
    // safe boundaries (fall-through-impossible positions) as a set — the
    // pick loop probes it per candidate and must not rescan the stream
    let safe_set: std::collections::HashSet<usize> = insts
        .iter()
        .zip(offs.iter().zip(lens.iter()))
        .filter(|(inst, _)| {
            matches!(inst, Inst::Jal { .. } | Inst::Jalr { .. } | Inst::Trap)
        })
        .map(|(_, (o, l))| o + l)
        .collect();
    for &label in &[fault_arith, fault_cast] {
        let mut grp: Vec<&(usize, usize, usize, u32, u8)> =
            sites.iter().filter(|s| s.1 == label).collect();
        grp.sort_by_key(|s| s.0);
        let code = if label == fault_arith { EXIT_ARITH } else { EXIT_CAST };
        let mut win_start = 0usize;
        while win_start < grp.len() {
            let mut end = win_start + 1;
            while end < grp.len() && grp[end].0 - grp[win_start].0 <= 510 {
                end += 1;
            }
            let mut planned = false;
            loop {
                let n = end - win_start;
                if n < 2 {
                    break; // singleton: net ≤ 0 by construction
                }
                // worst-case net (8-byte copy) must be strictly positive;
                // the 6-byte c.j form is decided in phase 2 with full ops
                if 2 * n as i64 - 8 <= 0 {
                    break;
                }
                let span = grp[end - 1].0 - grp[win_start].0;
                // post-cut span (each converted site sheds 2 bytes): a copy
                // inside the cluster reaches both ends only if this holds
                if span - 2 * (n - 1) + 8 > 508 {
                    end -= 1;
                    continue;
                }
                // rough copy window in original coordinates, then exact
                // displacement check over decoded instruction starts
                // (largest start first — deterministic)
                // early-window sites sit before offset 246: keep the
                // rough window arithmetic signed, clamped at 0
                let lo = (grp[end - 1].0 as i64 + 8 - 254).max(0) as usize;
                let hi = grp[win_start].0 + 254;
                // fall-through safety: the copy sits between original
                // instructions, so it is reachable by straight-line flow
                // unless the preceding instruction never falls through.
                // Only boundaries directly after an unconditional control
                // transfer (jal/c.j/c.jr — also after `ebreak`, which traps)
                // qualify; a copy elsewhere would execute `li a0,90; jump
                // to the fault thunk` on an otherwise-trap-free path.
                let mut picked: Option<usize> = None;
                if lo <= hi {
                    let virt: Vec<(usize, i64)> = grp[win_start..end]
                        .iter()
                        .map(|s| (s.0 + 2, -2i64))
                        .collect();
                    for &p in offs.iter().rev() {
                        if p > hi {
                            continue;
                        }
                        if p < lo {
                            break;
                        }
                        if !safe_set.contains(&p) {
                            continue;
                        }
                        let cf = p as i64 + shift_prefix[p] + virt_shift(&virt, p);
                        let ok = grp[win_start..end].iter().all(|s| {
                            let fi = s.0 as i64
                                + shift_prefix[s.0]
                                + virt_shift(&virt, s.0)
                                + i64::from(p <= s.0) * 8;
                            let d = cf - fi;
                            d.abs() <= 254 && d & 1 == 0
                        });
                        let hops = guard_hoppers_ok(asm, profile, p, 8);
                        if ok && hops {
                            picked = Some(p);
                            break;
                        }
                    }
                }
                let Some(p) = picked else {
                    end -= 1;
                    continue;
                };
                plans.push(GuardTailPlan {
                    code,
                    sites: grp[win_start..end]
                        .iter()
                        .map(|s| (s.0, s.2, s.3, s.4))
                        .collect(),
                });
                for s in &grp[win_start..end] {
                    accepted_ops.push((s.0 + 2, -2));
                }
                accepted_ops.push((p, 8));
                // refresh the O(1) shift table for subsequent windows
                for v in shift_prefix.iter_mut() {
                    *v = 0;
                }
                for &(pos, d) in &accepted_ops {
                    shift_prefix[pos] += d;
                }
                for i in 1..shift_prefix.len() {
                    shift_prefix[i] += shift_prefix[i - 1];
                }
                planned = true;
                break;
            }
            // on success skip past the accepted window; on failure retry
            // from the next site as anchor — a failed window's early sites
            // must not swallow later, independently viable clusters
            win_start = if planned { end } else { win_start + 1 };
        }
    }
    if plans.is_empty() {
        return Ok(Vec::new());
    }
    // ---- phase 2 + 2.5: copy form per plan (all ops known, exact
    // geometry) with per-site displacement verification ----
    // `all_ops`: cuts (−2 per site) + inserts; insert deltas start at the
    // worst-case 8 and drop to 6 when the cluster takes the c.j form.
    // Phase 1 picked positions under worst-case 8-byte assumptions; here
    // every converted site's exact final displacement to its copy is
    // re-verified (overlapping clusters of different thunks can interleave
    // their ops), and any cluster with a violating site is dropped wholesale
    // (its ops leave the geometry) until the remaining set verifies clean.
    let mut all_ops: Vec<(usize, i64, usize, bool)> = Vec::new(); // bound, delta, cluster, is_insert
    for (ci, pl) in plans.iter().enumerate() {
        let p = accepted_ops
            .iter()
            .filter(|o| o.1 > 0)
            .nth(ci)
            .map(|o| o.0)
            .expect("insert op per plan");
        for s in &pl.sites {
            all_ops.push((s.0 + 2, -2, ci, false));
        }
        all_ops.push((p, 8, ci, true));
    }
    let final_at = |ops: &[(usize, i64, usize, bool)], x: usize| -> i64 {
        x as i64 + ops.iter().filter(|o| o.0 <= x).map(|o| o.1).sum::<i64>()
    };
    // verify every converted site's displacement to its copy (exact)
    let sites_ok = |ops: &[(usize, i64, usize, bool)], live: &[usize]| -> bool {
        live.iter().all(|&ci| {
            let Some(ins) = ops.iter().find(|o| o.3 && o.2 == ci) else { return true };
            let (p, delta, _, _) = (ins.0, ins.1, ins.2, ins.3);
            let cf = final_at(ops, p) - delta;
            plans[ci].sites.iter().all(|s| {
                let fi = final_at(ops, s.0);
                let d = cf - fi;
                d.abs() <= 254 && d & 1 == 0
            })
        })
    };
    let mut forms: Vec<bool> = vec![false; plans.len()]; // c.j per cluster
    let mut live: Vec<usize> = (0..plans.len()).collect();
    loop {
        // decide forms for the live clusters (c.j first: bigger net, needs
        // the finished copy→thunk displacement inside the ±2 KiB window)
        for &ci in live.iter() {
            let want = 2 * plans[ci].sites.len() as i64;
            let mut chose = (8i64, false);
            for &(d, cj) in &[(6i64, true), (8i64, false)] {
                if want - d <= 0 {
                    continue;
                }
                let trial: Vec<(usize, i64, usize, bool)> = all_ops
                    .iter()
                    .map(|o| match o {
                        (b, _, c, true) if *c == ci => (*b, d, *c, true),
                        _ => *o,
                    })
                    .collect();
                let ins = trial.iter().find(|o| o.3 && o.2 == ci).expect("insert");
                let cf = final_at(&trial, ins.0) - d;
                let thunk_final = body_len as i64 + trial.iter().map(|o| o.1).sum::<i64>();
                // the c.j sits at cf+4 (after the 4-byte li)
                let dist = thunk_final - cf - 4;
                if dist >= -2048 && dist <= 2046 && dist & 1 == 0 {
                    chose = (d, cj);
                    break;
                }
            }
            let ins = all_ops.iter_mut().find(|o| o.3 && o.2 == ci).unwrap();
            ins.1 = chose.0;
            forms[ci] = chose.1;
        }
        if sites_ok(&all_ops, &live) {
            break;
        }
        // drop the lowest-indexed violating cluster and retry (deterministic)
        let mut dropped = None;
        'outer: for &ci in live.iter() {
            let ins = all_ops.iter().find(|o| o.3 && o.2 == ci).expect("insert");
            let (p, delta, _, _) = (ins.0, ins.1, ins.2, ins.3);
            let cf = final_at(&all_ops, p) - delta;
            for s in &plans[ci].sites {
                let d = cf - final_at(&all_ops, s.0);
                if d.abs() > 254 || d & 1 != 0 {
                    dropped = Some(ci);
                    break 'outer;
                }
            }
        }
        let Some(ci) = dropped else {
            break; // no violation attributable to a live cluster: done
        };
        live.retain(|&c| c != ci);
        all_ops.retain(|o| o.2 != ci);
    }
    let mut ops: Vec<GuardTailOp> = Vec::new();
    for &ci in live.iter() {
        let pl = &plans[ci];
        let p = all_ops.iter().find(|o| o.3 && o.2 == ci).map(|o| o.0).unwrap();
        let delta = all_ops.iter().find(|o| o.3 && o.2 == ci).map(|o| o.1).unwrap();
        let form_cj = forms[ci];
        if 2 * pl.sites.len() as i64 - delta <= 0 {
            continue; // unprofitable after form resolution (cannot happen:
                      // phase 1 gated on the 8-byte worst case)
        }
        for s in &pl.sites {
            ops.push(GuardTailOp {
                bound: s.0 + 2,
                delta: -2,
                cluster: ci,
                site: Some(s.0),
                ins: None,
            });
        }
        ops.push(GuardTailOp {
            bound: p,
            delta,
            cluster: ci,
            site: None,
            ins: Some((p, pl.code, form_cj)),
        });
    }
    let mut rec_by_cluster: Vec<Option<GuardCopyRecord>> = vec![None; plans.len()];
    for o in ops.iter().filter(|o| o.ins.is_some()) {
        rec_by_cluster[o.cluster] = Some(GuardCopyRecord {
            code: o.ins.unwrap().1,
            copy_at: 0, // filled after application (final coordinates)
            form_cj: o.ins.unwrap().2,
            sites: plans[o.cluster].sites.iter().map(|s| s.0).collect(),
        });
    }
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    // ---- phase 3: apply, descending by bound ----
    // one label per plan (only live ones get placed)
    let copy_labels: Vec<usize> = (0..plans.len()).map(|_| asm.new_label()).collect();
    ops.sort_by(|a, b| b.bound.cmp(&a.bound));
    for op in &ops {
        if let Some(site) = op.site {
            // rewrite [site, site+2) into the c.bz slot, drain [site+2, site+4)
            let (_off, _label, _seq, f3, rs1) = *sites
                .iter()
                .find(|s| s.0 == site)
                .expect("site recorded");
            let off = site;
            let cf3 = if f3 == 0 { 6 } else { 7 }; // c.beqz / c.bnez
            // repatch FIRST, on the intact stream: after the placeholder
            // write, the drained half [off+2, off+4) still holds the branch
            // word's top half — not a decodable instruction — and the
            // repatch's stream walk would die there and silently skip every
            // baked branch hopping this cut (the HIP5 trap bug)
            repatch_hopped_backward(asm, profile, off + 2, off + 4);
            let patched = Asm::c_bz(cf3, rs1, 0);
            asm.buf[off..off + 2].copy_from_slice(&patched.to_le_bytes());
            asm.buf.drain(off + 2..off + 4);
            for l in asm.labels.iter_mut() {
                if let Some(pp) = l {
                    if *pp >= off + 4 {
                        *pp -= 2;
                    }
                }
            }
            for f in asm.fixups.iter_mut() {
                if f.at >= off + 4 {
                    f.at -= 2;
                }
            }
            // retarget the site's own fixup onto its cluster's copy label
            let f = asm.fixups.iter_mut().find(|f| f.at == off).expect("site fixup");
            f.kind = FixKind::CBranchZ;
            f.label = copy_labels[op.cluster];
        } else if let Some((p, code, form_cj)) = op.ins {
            let delta = op.delta as usize;
            let fill: Vec<u8> = {
                let mut v = vec![0u8; delta];
                // li a0, code (addi a0, x0, imm — 90/71 exceed the c.li range,
                // so this is always the 4-byte I-form, matching the shared
                // thunks' own `asm.li(A0, code)`)
                let w = (((code as u32) & 0xFFF) << 20) | ((A0 as u32) << 7) | 0b0010011;
                v[0..4].copy_from_slice(&w.to_le_bytes());
                if !form_cj {
                    // jal x0, trap — fixup-backed, resolved by finish
                    v[4..8].copy_from_slice(&Asm::jal_word(ZERO, 0).to_le_bytes());
                } else {
                    // c.j placeholder — displacement baked after all ops
                    // (the copy→thunk distance is invariant under the
                    // remaining lower ops, which shift copy and thunk alike)
                    v[4..6].copy_from_slice(&Asm::c_j(0).to_le_bytes());
                }
                v
            };
            asm.buf.splice(p..p, fill);
            for l in asm.labels.iter_mut() {
                if let Some(pp) = l {
                    if *pp >= p {
                        *pp += delta;
                    }
                }
            }
            for f in asm.fixups.iter_mut() {
                if f.at >= p {
                    f.at += delta;
                }
            }
            // push the copy's own jal fixup only AFTER the shift loops — it
            // is written at the copy's in-buffer tail (p+4) and must not
            // shift itself (a self-shifted fixup would patch the wrong word
            // in `finish` and leave the real jal unresolvable)
            if !form_cj {
                asm.fixups.push(Fixup {
                    at: p + 4,
                    label: trap_label,
                    kind: FixKind::Jal,
                    seq: usize::MAX,
                    aux: 0,
                });
            }
            // the copy label sits at the copy's first byte (p in the
            // current buffer); set directly — `place` would pin it to the
            // buffer end, which is the literal pool region here
            assert!(asm.labels[copy_labels[op.cluster]].is_none(), "copy label placed twice");
            asm.labels[copy_labels[op.cluster]] = Some(p);
            repatch_widened_backward(asm, profile, p, p + delta);
        }
    }
    // ---- post-apply: fill copy_at records, bake c.j displacements,
    // validate every baked branch window ----
    let total_shift: i64 = ops.iter().map(|o| o.delta).sum();
    for ci in live.iter() {
        let Some(rec) = rec_by_cluster[*ci].as_mut() else { continue };
        let (p, delta, _, _) = all_ops
            .iter()
            .find(|o| o.3 && o.2 == *ci)
            .copied()
            .expect("insert op");
        let shift: i64 = all_ops.iter().filter(|o| o.0 <= p).map(|o| o.1).sum();
        let cf = p as i64 + shift - delta;
        rec.copy_at = cf as usize;
        if rec.form_cj {
            // bake the c.j displacement (exact: all ops are applied; the
            // copy→thunk distance is invariant under any re-derivation).
            // The c.j sits at cf+4 (after the 4-byte li): displacement =
            // thunk_entry − (cf+4).
            let thunk_final = body_len as i64 + total_shift;
            let dist = thunk_final - cf - 4;
            let at = cf as usize + 4;
            let patched = Asm::c_j(dist as i32);
            asm.buf[at..at + 2].copy_from_slice(&patched.to_le_bytes());
        }
    }
    validate_branch_windows(asm, profile)?;
    Ok(rec_by_cluster.into_iter().flatten().collect())
}

/// Pre-check that inserting `delta` bytes at `p` keeps every baked branch
/// hopping the insert inside its encoding window (the widened displacements
/// are re-encoded by [`repatch_widened_backward`]; later ops can only
/// shrink these again, and `validate_branch_windows` post-verifies).
fn guard_hoppers_ok(asm: &Asm, profile: u8, p: usize, delta: i64) -> bool {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else {
        return false;
    };
    for i in 0..insts.len() {
        let (rel, is_branch) = match &insts[i] {
            Inst::Branch { target, .. } => (*target, true),
            Inst::Jal { target, .. } => (*target, false),
            _ => continue,
        };
        if asm.fixups.iter().any(|f| f.at == offs[i]) {
            continue; // resolved by finish at final positions
        }
        let abs = offs[i] as i64 + rel;
        let grown = if rel < 0 {
            // backward: hopper iff target below the insert ≤ site
            abs < p as i64 && p <= offs[i]
        } else {
            // forward (+8 trampoline skips): hopper iff site below insert ≤ target
            offs[i] < p && p <= abs as usize
        };
        if !grown {
            continue;
        }
        if rel.abs() + delta > baked_window(lens[i], is_branch) {
            return false;
        }
    }
    true
}

/// Lower a compiled fitsh body (IRNodeArray from `fitshc::compile_body`) to
/// Lower a compiled fitsh body (IRNodeArray from `fitshc::compile_body`) to
/// RV64IM profile-0 machine code. Zero-arg kernel.
pub fn lower_body(ir: &IRNodeArray) -> Ret<Rv64Lowered> {
    lower_body_with_input(ir, false)
}

/// `has_input`: the body's slot 0 is the kernel's `data: bytes` parameter,
/// read from the ABI input registers.
pub fn lower_body_with_input(ir: &IRNodeArray, has_input: bool) -> Ret<Rv64Lowered> {
    lower_body_mode(ir, has_input, Rv64Mode::Im)
}

/// Lower in the given mode; `Imc*` emit RVC encodings (consensus profile 2).
pub fn lower_body_mode(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
) -> Ret<Rv64Lowered> {
    lower_body_mode_arena(ir, has_input, mode, ARENA_SIZE)
}

/// `arena_size`: the RW arena bytes (see `ARENA_SIZE` / `ARENA_SIZE_MAX`).
pub fn lower_body_mode_arena(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
) -> Ret<Rv64Lowered> {
    lower_body_mode_arena_impl(ir, has_input, mode, arena_size, false).map(|(lo, _)| lo)
}

/// Debug twin of [`lower_body_mode_arena`]: byte-identical machine code plus
/// a [`LowerDbg`] side-channel (web-walk timeline, per-statement code-offset
/// trace, promotion-plan mirror). Pure addition — the engine is shared with
/// the production entry, and `dbg_engine_matches_production` pins equality.
pub fn lower_body_mode_arena_dbg(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
) -> Ret<(Rv64Lowered, LowerDbg)> {
    lower_body_mode_arena_impl(ir, has_input, mode, arena_size, true)
        .map(|(lo, dbg)| (lo, dbg.expect("dbg requested")))
}

fn lower_body_mode_arena_impl(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
    want_dbg: bool,
) -> Ret<(Rv64Lowered, Option<LowerDbg>)> {
    let mut fp = plan(&ir.subs, has_input)?;
    // BUG-012: flow-sensitive may-Nil analysis (init flags + value tags +
    // consumer guards). Slots proven non-Nil everywhere get no machinery.
    let nilp = nil_plan(&ir.subs, &fp.slot, has_input);
    // P3-IDIOMS: byte-assembly fusion marks — computed once (needs plan's
    // slot types + nilp's may-Nil occ), deterministic, shared by the
    // probe pass and every relaxation pass.
    let fuse = fuse_scan(&ir.subs, &fp.slot, &nilp);
    let profile = match mode {
        Rv64Mode::Im => container::CALC_PROFILE_RV64IM,
        _ => container::CALC_PROFILE_RV64IMC,
    };
    // Iterative branch relaxation: lower, resolve, and when conditional
    // branches exceed the ±4 KiB B-type range, redo the pass with those
    // sites in the jal-trampoline form. Lowering is deterministic, so the
    // per-site ids are stable; each pass only grows the set (converges).
    // ---- ImcRemap promotion plan --------------------------------------
    // Probe pass: the exact peak expression depth D (the lowering is a
    // deterministic function of the body, so a dry run measures it). The
    // pool shrinks to D (min 1, cap 8); the freed registers — the
    // compressed-window pair x14/x15 plus gp/tp/t2/a6/s6..s11 — carry a
    // local bank. Web segments (per-write live ranges, Wimmer-style holes
    // without the machinery) of the hottest slots share those registers:
    // disjoint segments reuse one register, exclusive entry-zero segments
    // keep their own. Soundness lives in the split rule (region-contained
    // reads) and the interval overlap check.
    let mut pool_size = 8usize;
    let mut plan = WebPlan { banked: Vec::new(), homes: Vec::new(), init_zero: Vec::new(), segs_debug: Vec::new(), no_split_debug: Vec::new() };
    let mut pool_base_reg: Option<u8> = None;
    let mut frame_order: Vec<usize> = Vec::new();
    // dbg mirrors of the promotion plan (pure-addition bookkeeping)
    let mut lit_sites = 0usize;
    let mut bank_regs: Vec<u8> = Vec::new();
    if mode == Rv64Mode::ImcRemap {
        let mut probe = Lower::new_mode_arena_nil(&fp.slot, &fp.needs_zero, mode, arena_size, &nilp, &fp.list);
        probe.record_spans = true;
        probe.fuse = fuse.clone();
        probe.pair_root = fp.pair.root.clone();
        probe.pair_skip = fp.pair.skip.clone();
        probe.emit_prologue_body_epilogue(&ir.subs)?;
        let d = probe.max_depth.clamp(1, 8);
        pool_size = d;
        // P2-A27 S1: per-position liveness spans from the probe (empty plan,
        // so every operand is a Pool push — the spans over-approximate the
        // real lowering's pool writes; the INV_CHECK assertion in push_vreg
        // mirrors the resulting busy test at emission time).
        let probe_spans: Vec<Vec<(usize, usize)>> =
            probe.pos_spans.iter().map(|v| merge_spans(v.clone())).collect();
        if diag_env_enabled("RV64_EV_DEBUG") {
            println!("  probe: max_depth={} spans:", probe.max_depth);
            for (k, v) in probe_spans.iter().enumerate() {
                let spans: Vec<String> = v.iter().map(|&(a, b)| format!("[{a},{b}]")).collect();
                let cov: usize = v.iter().map(|&(a, b)| b + 1 - a).sum();
                println!("    pos{k}: cov={cov} {}", spans.join(""));
            }
        }
        if d <= 6 {
            // P2-A27 bank expansion. Candidates are (register, busy) pairs:
            // busy = closed statement intervals during which the expression
            // machinery may write the register. Base list (all fully free):
            // the compressed window pair + gp/tp/t2/a6/a7/s6..s11 + t1/x6
            // (S0: scratch a1 moved to SC2C) + sp/x2 (dead — no ABI stack).
            // a7/x17 is safe because the whole product has exactly ONE ecall
            // (the terminal trap epilogue `li a7,0; ecall` — every fault
            // thunk jumps there too), so its only write happens after no
            // bank read can ever occur. Then
            // the pool registers proper join as busy candidates:
            // VSTACK_ORDER_RVC[k] is written by any push at vstack position
            // k, i.e. exactly on the probe's position-k spans. Positions
            // 0/1 (a0/a1) are excluded — the XOP/forwarding arms use
            // vregs[0]/vregs[1] as statement-level scratch
            // (rv64.rs:3810-3813, 3598-3600), so they can never double as
            // bank homes.
            let busy = |k: usize| -> Vec<(usize, usize)> {
                if k < d { probe_spans[k].clone() } else { Vec::new() }
            };
            let mut regs: Vec<(u8, Vec<(usize, usize)>)> = vec![
                (14, Vec::new()),
                (15, Vec::new()),
                (3, Vec::new()),
                (4, Vec::new()),
                (7, Vec::new()),
                (16, Vec::new()),
                (17, Vec::new()),
                (22, Vec::new()),
                (23, Vec::new()),
                (24, Vec::new()),
                (25, Vec::new()),
                (26, Vec::new()),
                (27, Vec::new()),
                (29, busy(5)),
                (6, Vec::new()),
                (12, busy(2)),
                (13, busy(3)),
                (28, busy(4)),
                (2, Vec::new()),
            ];
            // literal-pool base register: pays for its prologue auipc+addi
            // when several literal sites replace their pairs with c.addi
            lit_sites = count_literal_sites(&ir.subs);
            if lit_sites >= 4 {
                let base_pos = regs.iter().position(|&(r, _)| r == 2).expect("x2 base candidate");
                let (r, _) = regs.remove(base_pos);
                pool_base_reg = Some(r);
            }
            bank_regs = regs.iter().map(|&(r, _)| r).collect();
            plan = web_plan(&ir, &fp, &regs);
            // un-banked slots laid out hottest-first for the c.ld/sd window
            let (reads, writes) = slot_use_counts(&ir.subs);
            let score = |i: usize| {
                reads.get(i).copied().unwrap_or(0) + writes.get(i).copied().unwrap_or(0)
            };
            frame_order = (0..fp.slot.len())
                .filter(|&i| {
                    matches!(fp.slot[i], SlotTy::Int | SlotTy::Bytes)
                        && plan.banked.get(i).copied().unwrap_or(BankKind::None) == BankKind::None
                        && !(has_input && i == 0)
                })
                .collect();
            frame_order.sort_by_key(|&i| std::cmp::Reverse(score(i)));
        }
    }
    let mut long_ids: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut wide_ids: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // A-19 batch 2 merge records of the final relaxation pass (dbg only)
    #[allow(unused_assignments)]
    let mut mul_merges_dbg: Vec<MulMergeRecord> = Vec::new();
    #[allow(unused_assignments)]
    let mut guard_copies_dbg: Vec<GuardCopyRecord> = Vec::new();
    // Dead-store / forwarding timeline from the web walk's event list: the
    // last statement index each slot is read at, and which slots are read
    // by while conditions (loop-carried — never store-eliminable). Static
    // IR properties: stable across relaxation retries.
    let mut timeline = WebWalk::new(&fp.slot);
    timeline.xop_reads = true;
    for n in &ir.subs {
        timeline.walk_stmt(&**n);
    }
    let last_read: Vec<usize> = timeline
        .ev
        .iter()
        .map(|evs| evs.iter().filter(|e| matches!(e.kind, EvKind::Read)).map(|e| e.idx).max().unwrap_or(0))
        .collect();
    // per-slot sorted read indices (backedge liveness queries)
    let read_idx: Vec<Vec<usize>> = timeline
        .ev
        .iter()
        .map(|evs| {
            let mut v: Vec<usize> =
                evs.iter().filter(|e| matches!(e.kind, EvKind::Read)).map(|e| e.idx).collect();
            v.sort_unstable();
            v
        })
        .collect();
    let while_spans = timeline.while_spans.clone();
    let loop_carried = timeline.cond_read.clone();
    // Zero-init elision: a `var x = 0` whose store is the slot's FIRST
    // timeline event, and which no while span re-executes ahead of a later
    // write, collapses into the prologue's frame zeroing (needs_zero is
    // forced on for the slot). Banked slots are excluded: their entry
    // zero is the register init, and a first-event write marks the segment
    // non-init, so the store must stay.
    let mut zero_init_skip: Vec<Option<usize>> = vec![None; fp.slot.len()];
    for s in 0..fp.slot.len() {
        let Some(evs) = timeline.ev.get(s) else { continue };
        let Some(first) = evs.first() else { continue };
        if !matches!(first.kind, EvKind::WriteU) {
            continue;
        }
        let has_later_write = evs.iter().any(|e| first.idx < e.idx && !matches!(e.kind, EvKind::Read));
        let in_loop = while_spans.iter().any(|&(a, b)| a <= first.idx && first.idx <= b);
        if has_later_write && in_loop {
            continue; // the backedge would resurrect a stale non-zero
        }
        if fp.needs_zero.get(s).copied() != Some(false) {
            // already needs the entry zero (read-before-write): the store
            // still establishes the value on paths that re-run the body —
            // keep it
            continue;
        }
        zero_init_skip[s] = Some(first.idx);
        fp.needs_zero[s] = true;
    }
    loop {
        let mut lw = Lower::new_mode_arena_bank(
            &fp.slot,
            &fp.needs_zero,
            mode,
            arena_size,
            pool_size,
            plan.clone(),
            pool_base_reg,
            &frame_order,
            &nilp,
            &fp.list,
        );
        if want_dbg {
            lw.stmt_trace = Some(Vec::new());
        }
        lw.asm.long_branch_ids = long_ids.clone();
        lw.asm.wide_c_ids = wide_ids.clone();
        lw.fuse = fuse.clone();
        lw.pair_root = fp.pair.root.clone();
        lw.pair_skip = fp.pair.skip.clone();
        lw.last_read = last_read.clone();
        lw.loop_carried = loop_carried.clone();
        lw.read_idx = read_idx.clone();
        lw.while_spans = while_spans.clone();
        lw.zero_init_skip = zero_init_skip.clone();
        lw.emit_prologue_body_epilogue(&ir.subs)?;
        // A-19 batch 2: prove + apply dominated constant-mul check merges
        // on the just-emitted body (before the fault thunks append, every
        // guard branch to the arith thunk is still a labeled forward
        // fixup). Re-derived identically on every relaxation pass.
        mul_merges_dbg = merge_dominated_mul_checks(&mut lw.asm, profile, lw.fault_arith);
        // P3-GUARDTAIL GT-a: insert copy thunks so clustered zero-form guard
        // branches reach a 2-byte c.bz. Re-derived identically on every
        // relaxation pass (deterministic function of the emitted stream).
        guard_copies_dbg = insert_guard_thunk_copies(
            &mut lw.asm,
            profile,
            lw.fault_arith,
            lw.fault_cast,
            lw.trap_label,
        )?;
        // Shared fault thunks, then the trap epilogue. Sites jump here with
        // one `jal` instead of inlining `li a0, code; jal trap`. The
        // executable window ends after the epilogue — everything below is
        // the literal pool.
        lw.asm.place(lw.fault_arith);
        lw.asm.li(A0, EXIT_ARITH);
        lw.asm.jal_(ZERO, lw.trap_label);
        lw.asm.place(lw.fault_cast);
        lw.asm.li(A0, EXIT_CAST);
        lw.asm.jal_(ZERO, lw.trap_label);
        // Nil-guard thunks (BUG-012): created lazily at guard sites, so
        // bodies exist only when some guard actually references them
        for &(l, code) in &lw.nil_thunks {
            lw.asm.place(l);
            lw.asm.li(A0, code);
            lw.asm.jal_(ZERO, lw.trap_label);
        }
        lw.asm.place(lw.trap_label);
        lw.asm.addi(A1, ZERO, 0);
        lw.asm.li(A7, 0);
        lw.asm.ecall();
        if lw.depth_err {
            return errf!(
                "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
            );
        }
        let code_len = lw.asm.here();
        // The literal pool is appended in the same buffer so pc-relative
        // fixups resolve against absolute positions; the pool segment vaddr
        // is CODE_BASE + code_len, so offsets stay valid after the split.
        // The pool-base label sits at the pool start (or the empty-pool
        // tail) so every `addi rd, base, off` reaches.
        if let Some((_, l)) = lw.pool_base {
            lw.asm.place(l);
        }
        let entries = std::mem::take(&mut lw.pool_entries);
        for (lbl, bytes) in &entries {
            lw.asm.place(*lbl);
            lw.asm.buf.extend_from_slice(bytes);
        }
        let all = match lw.asm.finish() {
            Ok(all) => all,
            Err(FinishErr::Msg(m)) => return errf!("{m}"),
            Err(FinishErr::BranchOverflow(sites)) => {
                let before = long_ids.len();
                long_ids.extend(sites);
                if long_ids.len() == before {
                    return errf!("rv64 lower: branch relaxation did not converge");
                }
                continue;
            }
            Err(FinishErr::WideCOverflow(sites)) => {
                let before = wide_ids.len();
                wide_ids.extend(sites);
                if wide_ids.len() == before {
                    return errf!("rv64 lower: c-slot relaxation did not converge");
                }
                continue;
            }
        };
        if all.len() < code_len || code_len % 2 != 0 {
            return errf!("rv64 lower: internal code/pool layout error");
        }
        let code = all[..code_len].to_vec();
        let pool = all[code_len..].to_vec();
        let stats = rv_code_stats(&code, profile, lw.slot.len())?;
        let out = Rv64Lowered { code, pool, arena_size, stats };
        if !want_dbg {
            return Ok((out, None));
        }
        // ---- dbg side-channel collection (pure addition) ----
        let dbg = LowerDbg {
            timeline: WebTimelineDbg {
                slot_tys: fp
                    .slot
                    .iter()
                    .map(|s| match s {
                        SlotTy::None => SLOT_TY_NONE,
                        SlotTy::Int => SLOT_TY_INT,
                        SlotTy::Bytes => SLOT_TY_BYTES,
                        SlotTy::Input => SLOT_TY_INPUT,
                        SlotTy::List => SLOT_TY_LIST,
                    })
                    .collect(),
                ev: timeline
                    .ev
                    .iter()
                    .map(|evs| {
                        evs.iter()
                            .map(|e| WebEvDbg {
                                idx: e.idx,
                                kind: match e.kind {
                                    EvKind::Read => WEB_EV_READ,
                                    EvKind::WriteU => WEB_EV_WRITE_U,
                                    EvKind::WriteC => WEB_EV_WRITE_C,
                                },
                                region: e.region,
                            })
                            .collect()
                    })
                    .collect(),
                while_spans: timeline.while_spans.clone(),
                cond_read: timeline.cond_read.clone(),
                region_parent: timeline.region_parent.clone(),
                stmt_total: timeline.idx,
            },
            stmt_ranges: lw.stmt_trace.take().unwrap_or_default(),
            banked: plan
                .banked
                .iter()
                .map(|b| match b {
                    BankKind::None => BANK_KIND_NONE,
                    BankKind::Int => BANK_KIND_INT,
                    BankKind::Bytes => BANK_KIND_BYTES,
                })
                .collect(),
            bank_regs,
            pool_size,
            pool_base_reg,
            frame_off: lw.frame_off.clone(),
            lit_sites,
            mul_merges: mul_merges_dbg,
            guard_copies: guard_copies_dbg,
        };
        return Ok((out, Some(dbg)));
    }
}

/* ---- Pass B: web/segment analysis + interval-shared bank allocation ---- */

/// Per-slot banking kind: the register (pair) IS the slot's home for the
/// whole function; which register depends on the active web segment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BankKind {
    None,
    Int,
    Bytes,
}

/// Static plan: which slots are banked and which register (pair) each web
/// segment of each banked slot lives in, keyed by statement index.
#[derive(Clone)]
struct WebPlan {
    banked: Vec<BankKind>,
    /// per slot: sorted (segment-start stmt idx, regs); Int uses [0].
    homes: Vec<Vec<(usize, [u8; 2])>>,
    /// banked slots whose first segment is an init segment (first touch
    /// not an unconditional write): they need the entry zero
    init_zero: Vec<bool>,
    /// debug copies for the invariant checker (test-only)
    segs_debug: Vec<Vec<Seg>>,
    no_split_debug: Vec<bool>,
}

impl WebPlan {
    /// Register home of `slot` at statement `idx` (Int: [0]; Bytes: both).
    /// Segment boundaries are write statements: a WRITE at the boundary
    /// takes the new segment (inclusive), a READ at it evaluates before
    /// that write and takes the previous one (exclusive).
    fn home_at(&self, slot: usize, idx: usize, for_write: bool) -> Option<[u8; 2]> {
        if !matches!(self.banked.get(slot), Some(BankKind::Int) | Some(BankKind::Bytes)) {
            return None;
        }
        let hs = self.homes.get(slot)?;
        let mut cur = hs.first()?.1;
        let mut chosen_b = hs.first()?.0;
        for (b, regs) in hs {
            let hit = if for_write { *b <= idx } else { *b < idx };
            if hit {
                cur = *regs;
                chosen_b = *b;
            } else {
                break;
            }
        }
        if std::env::var("RV64_INV_CHECK").is_ok() {
            let segs = &self.segs_debug[slot];
            if let Some(sg) = segs.iter().find(|s| s.start == chosen_b) {
                let covers = if for_write {
                    idx >= sg.start
                } else {
                    idx >= sg.start && idx <= sg.end
                };
                if !covers {
                    panic!(
                        "INV slot={slot} idx={idx} for_write={for_write} chosen_b={chosen_b} seg=[{},{}] no-split={}",
                        sg.start, sg.end, self.no_split_debug[slot]
                    );
                }
            }
        }
        Some(cur)
    }
}

#[derive(Clone, Copy)]
enum EvKind {
    Read,
    /// unconditional write: dominates later reads on its region path
    WriteU,
    /// write under if/choose arms: merges into the current segment
    WriteC,
}

#[derive(Clone, Copy)]
struct Ev {
    idx: usize,
    kind: EvKind,
    region: usize,
}

/// One live segment of one slot: `[start, end]` in statement indices; the
/// value is born at `start` (a write, or the zero from entry for init
/// segments) and dead after `end`.
#[derive(Clone)]
struct Seg {
    start: usize,
    end: usize,
    /// value comes from the entry zero: cannot share (a sharer's writes
    /// between the prologue and the segment's reads would clobber it)
    init: bool,
}

/// Statement-order walk mirroring `Lower`'s traversal exactly (the same
/// nodes increment the same counter), recording per-slot access events.
struct WebWalk<'a> {
    slot: &'a [SlotTy],
    /// Timeline mode: record XOP's read-modify-write as an explicit Read
    /// event ahead of its Write. The plan walk keeps the legacy pure-write
    /// events; the dead-store walk must see the old-value read or a store
    /// feeding only an `+=` looks dead and gets dropped.
    xop_reads: bool,
    idx: usize,
    /// writes under if/choose arms are conditional; while bodies are NOT
    /// (their top-of-body writes dominate the body's own reads; the split
    /// rule's region containment handles loop-skip paths)
    if_depth: usize,
    region: usize,
    next_region: usize,
    region_parent: Vec<usize>,
    ev: Vec<Vec<Ev>>,
    /// slots read by some while condition: loop-carried values — never
    /// split (the backedge re-reads the last body write invisibly)
    cond_read: Vec<bool>,
    /// (while idx, post-body idx) spans, filled on body completion
    while_spans: Vec<(usize, usize)>,
    /// if-diamond ranges (hdr, t_hi, e_hi): hdr = the IRIF's own idx,
    /// t_hi/e_hi = last idx of the then/else arm (P2-A27-S2: statement-CFG
    /// liveness for exact segment ends — see `StmtCfg`)
    ifrs: Vec<(usize, usize, usize)>,
    /// enclosing while spans (indices into `while_spans`), for break/continue
    loop_stack: Vec<usize>,
    /// break/continue sites: (stmt idx, enclosing `while_spans` index,
    /// is_break) — their CFG edge leaves the linear chain
    brk_cont: Vec<(usize, usize, bool)>,
}

impl<'a> WebWalk<'a> {
    fn new(slot: &'a [SlotTy]) -> Self {
        Self {
            slot,
            xop_reads: false,
            idx: 0,
            if_depth: 0,
            region: 0,
            next_region: 1,
            region_parent: vec![usize::MAX],
            ev: vec![Vec::new(); slot.len()],
            cond_read: vec![false; slot.len()],
            while_spans: Vec::new(),
            ifrs: Vec::new(),
            loop_stack: Vec::new(),
            brk_cont: Vec::new(),
        }
    }

    fn region_within(&self, r: usize, anc: usize) -> bool {
        let mut cur = r;
        while cur != usize::MAX {
            if cur == anc {
                return true;
            }
            cur = self.region_parent[cur];
        }
        false
    }

    fn record_expr(&mut self, node: &dyn IRNode) {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.record_expr(&*w.node);
        }
        let op = match Bytecode::try_from_u8(node.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        match op {
            Bytecode::GET | Bytecode::GET0 | Bytecode::GET1 | Bytecode::GET2 | Bytecode::GET3 => {
                if let Some(s) = slot_get_index(node) {
                    let s = s as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes)) {
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                    }
                }
            }
            Bytecode::PUT | Bytecode::XOP => {
                // nested inside an expression: always conditional
                for c in children(node) {
                    self.record_expr(c);
                }
                let s = match op {
                    Bytecode::PUT => node
                        .as_any()
                        .downcast_ref::<IRNodeParam1Single>()
                        .map(|p| p.para as usize),
                    _ => node
                        .as_any()
                        .downcast_ref::<IRNodeParam1Single>()
                        .map(|p| decode_local_operand_mark(p.para).1 as usize),
                };
                if let Some(s) = s {
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes)) {
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::WriteC, region: self.region });
                    }
                }
            }
            _ => {
                for c in children(node) {
                    self.record_expr(c);
                }
            }
        }
    }

    fn walk_stmt(&mut self, node: &dyn IRNode) {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.walk_stmt(&*w.node);
        }
        if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
            || node.as_any().downcast_ref::<IRNodeText>().is_some()
        {
            self.idx += 1;
            return;
        }
        let op = match Bytecode::try_from_u8(node.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        self.idx += 1;
        match op {
            Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR => {
                if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                    for sub in &arr.subs {
                        self.walk_stmt(&**sub);
                    }
                }
            }
            Bytecode::IRIF => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
                    self.record_expr(&*n.subx);
                    let hdr = self.idx;
                    self.if_depth += 1;
                    self.walk_stmt(&*n.suby);
                    let t_hi = self.idx - 1;
                    self.walk_stmt(&*n.subz);
                    let e_hi = self.idx - 1;
                    self.if_depth -= 1;
                    self.ifrs.push((hdr, t_hi, e_hi));
                }
            }
            Bytecode::IRIFR | Bytecode::CHOOSE => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
                    self.record_expr(&*n.subx);
                    self.if_depth += 1;
                    self.record_expr(&*n.suby);
                    self.record_expr(&*n.subz);
                    self.if_depth -= 1;
                }
            }
            Bytecode::IRWHILE => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    // loop-carried slots (read in the condition) keep ONE
                    // segment across the loop: the backedge re-reads the
                    // last body write in a way the linear event list cannot
                    // express
                    mark_cond_reads(&*n.subx, &mut self.cond_read);
                    self.record_expr(&*n.subx);
                    let r = self.next_region;
                    self.next_region += 1;
                    self.region_parent.push(self.region);
                    let saved = self.region;
                    self.region = r;
                    let span = self.while_spans.len();
                    self.while_spans.push((self.idx, self.idx));
                    self.loop_stack.push(span);
                    self.walk_stmt(&*n.suby);
                    self.loop_stack.pop();
                    self.while_spans[span].1 = self.idx;
                    self.region = saved;
                }
            }
            Bytecode::PUT => {
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    self.record_expr(&*p.subx);
                    let s = p.para as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes)) {
                        let kind = if self.if_depth == 0 { EvKind::WriteU } else { EvKind::WriteC };
                        self.ev[s].push(Ev { idx: self.idx, kind, region: self.region });
                    }
                }
            }
            Bytecode::XOP => {
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    self.record_expr(&*p.subx);
                    let s = decode_local_operand_mark(p.para).1 as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes)) {
                        if self.xop_reads {
                            // the old value is read before the write (same
                            // statement index: boundary reads evaluate first)
                            self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                        }
                        let kind = if self.if_depth == 0 { EvKind::WriteU } else { EvKind::WriteC };
                        self.ev[s].push(Ev { idx: self.idx, kind, region: self.region });
                    }
                }
            }
            Bytecode::REQ => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    self.record_expr(&*n.subx);
                    self.record_expr(&*n.suby);
                }
            }
            Bytecode::AST | Bytecode::ERR => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeSingle>() {
                    self.record_expr(&*n.subx);
                }
            }
            Bytecode::RET => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeSingle>() {
                    self.record_expr(&*n.subx);
                }
            }
            _ => {
                // break/continue leave the linear chain (jump to the loop's
                // exit/header): record the edge target's span for the CFG
                if matches!(op, Bytecode::IRBREAK | Bytecode::IRCONTINUE) {
                    if let Some(&sp) = self.loop_stack.last() {
                        self.brk_cont.push((self.idx, sp, op == Bytecode::IRBREAK));
                    }
                }
                // expression-fallback statement: eval + drop
                self.record_expr(node);
            }
        }
    }
}

/// Statement-index count of `node` under `WebWalk::walk_stmt` — the Lower
/// cursor must advance by exactly this per statement, or `home_at` lookups
/// desync from the plan (a real bug class: const-select arms count their
/// inner block+PUT+else while the group consumed them as one sub).
fn stmt_walk_count(node: &dyn IRNode) -> usize {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return stmt_walk_count(&*w.node);
    }
    if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
        || node.as_any().downcast_ref::<IRNodeText>().is_some()
    {
        return 1;
    }
    let op = match Bytecode::try_from_u8(node.bytecode()) {
        Ok(o) => o,
        Err(_) => return 1,
    };
    1 + match op {
        Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR => {
            node.as_any()
                .downcast_ref::<IRNodeArray>()
                .map(|arr| arr.subs.iter().map(|s| stmt_walk_count(&**s)).sum())
                .unwrap_or(0)
        }
        Bytecode::IRIF => match node.as_any().downcast_ref::<IRNodeTriple>() {
            Some(n) => stmt_walk_count(&*n.suby) + stmt_walk_count(&*n.subz),
            None => 0,
        },
        Bytecode::IRWHILE => match node.as_any().downcast_ref::<IRNodeDouble>() {
            Some(n) => stmt_walk_count(&*n.suby),
            None => 0,
        },
        _ => 0,
    }
}

/// Flag every slot read inside an expression tree as loop-carried.
fn mark_cond_reads(node: &dyn IRNode, cond_read: &mut Vec<bool>) {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return mark_cond_reads(&*w.node, cond_read);
    }
    if let Some(s) = slot_get_index(node) {
        let s = s as usize;
        if cond_read.len() > s {
            cond_read[s] = true;
        }
        return;
    }
    for c in children(node) {
        mark_cond_reads(c, cond_read);
    }
}

/* ---- P2-A27-S2: statement-level CFG for exact (cfg-sound) liveness ---- */

/// The statement CFG the segment ends are measured on: linear chain + if
/// diamond rewiring (the untaken arm is skipped by the header's false edge,
/// arm tails land at the join) + while backedges/loop exits + break /
/// continue edges. Production port of the census measurement CFG
/// (`rv64_backend_maxlive_census.rs`, `slot_liveness(Def::Cfg)` — doc 10),
/// which doc 12 §6.1 identified as the true lifetime of the init segments
/// the old `seg_end` backedge rule over-extended to the loop end.
struct StmtCfg {
    /// succ[p] = real successors of statement p (p in 1..=n; n+1 = exit)
    succ: Vec<Vec<usize>>,
}

impl StmtCfg {
    /// `spans` = `WebWalk::while_spans` (header idx, last body idx);
    /// `ifrs` = (hdr, t_hi, e_hi); `brk_cont` = (stmt, span idx, is_break).
    fn new(n: usize, spans: &[(usize, usize)], ifrs: &[(usize, usize, usize)], brk_cont: &[(usize, usize, bool)]) -> Self {
        // The landing resolver: the real statement-level landing of a
        // fallthrough that leaves a construct at virtual position q, where
        // the leaving construct's tail is `from`. Innermost-first over the
        // constructs containing `from`: each redirect moves the landing
        // past one enclosing if arm (to its join) and rescans; a landing
        // past an enclosing loop body end is the loop's backedge. `skip`
        // excludes one construct (the resolver must not redirect through
        // the very edge it is resolving).
        fn land(
            mut q: usize,
            from: usize,
            spans: &[(usize, usize)],
            ifrs: &[(usize, usize, usize)],
            skip: &mut Vec<(u8, usize)>,
        ) -> usize {
            loop {
                let mut innermost_loop: Option<(usize, usize)> = None;
                for (k, &(a, b)) in spans.iter().enumerate() {
                    if a <= from && from <= b && !skip.contains(&(b'L', k)) {
                        let better = match innermost_loop {
                            Some((a0, _)) => a > a0,
                            None => true,
                        };
                        if better {
                            innermost_loop = Some((a, b));
                        }
                    }
                }
                if let Some((a, b)) = innermost_loop {
                    if q > b {
                        return a; // enclosing body completes → backedge
                    }
                }
                let mut innermost_if: Option<(usize, usize, usize, usize)> = None;
                for (k, &(hdr, t_hi, e_hi)) in ifrs.iter().enumerate() {
                    if hdr <= from && from <= e_hi && !skip.contains(&(b'I', k)) {
                        let better = match innermost_if {
                            Some((_, h0, _, _)) => hdr > h0,
                            None => true,
                        };
                        if better {
                            innermost_if = Some((k, hdr, t_hi, e_hi));
                        }
                    }
                }
                match innermost_if {
                    Some((k, _hdr, t_hi, e_hi)) => {
                        let arm_end = if from <= t_hi { t_hi } else { e_hi };
                        if q > arm_end {
                            skip.push((b'I', k));
                            q = e_hi + 1; // arm completes → resolve at the join
                            continue;
                        }
                        return q;
                    }
                    None => return q,
                }
            }
        }
        let mut succ: Vec<Vec<usize>> = (0..=n + 1).map(|p| vec![p + 1]).collect();
        // if diamonds: the header's true/false edges enter the then/else
        // arm when it consumed statements, else jump straight to the join
        // (an arm that emitted nothing is skipped, not entered)
        for (k, &(hdr, t_hi, e_hi)) in ifrs.iter().enumerate() {
            let then_arm = t_hi >= hdr + 1;
            let else_arm = e_hi >= t_hi + 1;
            let join = land(e_hi + 1, e_hi, spans, ifrs, &mut vec![(b'I', k)]);
            let te = if then_arm { hdr + 1 } else { join };
            let fe = if else_arm { t_hi + 1 } else { join };
            succ[hdr] = if te == fe { vec![te] } else { vec![te, fe] };
            if then_arm {
                succ[t_hi] = vec![land(t_hi + 1, t_hi, spans, ifrs, &mut vec![(b'I', k)])];
            }
            if else_arm {
                succ[e_hi] = vec![join];
            }
        }
        // while loops: header → body / exit, tail → header. An empty body
        // keeps the exit edge on the header itself (succ[b] would be the
        // header's own self overwrite). The tail overwrite is consistent
        // with any if-rewiring there: an arm ending at the loop tail
        // resolves its join to this same backedge.
        for (k, &(a, b)) in spans.iter().enumerate() {
            let exit = land(b + 1, b, spans, ifrs, &mut vec![(b'L', k)]);
            if b == a {
                let mut s = vec![a];
                if exit != a {
                    s.push(exit);
                }
                succ[a] = s;
            } else {
                succ[a] = vec![a + 1, exit];
                succ[b] = vec![a];
            }
        }
        // break/continue: their fallthrough is dead code; the jump goes to
        // the enclosing loop's exit / header
        for &(idx, sp, is_break) in brk_cont {
            let (a, b) = spans[sp];
            succ[idx] = vec![if is_break {
                land(b + 1, b, spans, ifrs, &mut vec![(b'L', sp)])
            } else {
                a
            }];
        }
        StmtCfg { succ }
    }
}

/// Backward may-liveness per statement for one slot's event list
/// (use-before-def): `live[p]` = the slot's value is live immediately
/// before statement p. Reads are uses (a read at a write's own index uses
/// the OLD value — evaluation precedes the store); WriteU and WriteC both
/// kill, but only on paths through the statement (the if rewiring carries
/// that). Loop backedges are real successors, so a value that may be
/// re-read in a later iteration stays live around the whole loop — while a
/// value fully redefined before every re-read (both diamond arms writing,
/// dominating arm writes) dies at the join. This is exactly the register
/// exclusivity the bank allocator needs: the unit interval must cover
/// every live statement plus every event statement.
fn cfg_live(cfg: &StmtCfg, evs: &[Ev], n: usize) -> Vec<bool> {
    let mut use_ = vec![false; n + 2];
    let mut kill = vec![false; n + 2];
    for e in evs {
        match e.kind {
            EvKind::Read => use_[e.idx] = true,
            EvKind::WriteU | EvKind::WriteC => kill[e.idx] = true,
        }
    }
    let mut live = vec![false; n + 3];
    loop {
        let mut changed = false;
        for p in (1..=n).rev() {
            let mut out = false;
            for &s in &cfg.succ[p] {
                out = out || live[s];
            }
            let l = use_[p] || (out && !kill[p]);
            if l != live[p] {
                live[p] = l;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live
}

/// Build live segments for one slot from its (in-order) events.
///
/// Splitting rule (soundness, unchanged): split the timeline at each
/// unconditional write only when all those writes share one region R and
/// every read after the first write is region-contained in R — then each
/// execution that reaches a read of segment i has passed write i.
/// Otherwise one long segment.
///
/// Segment END (P2-A27-S2): the last statement at which the segment's
/// register must stay exclusively owned = the last statement where any
/// event maps to the segment (reads evaluate before boundary writes;
/// writes clobber) or the segment's value is cfg-live (a sharer there
/// would overwrite it before its reader — backedge re-reads included,
/// which the fixpoint folds in). This replaces the old conservative
/// extension to the outermost loop containing any read: a slot fully
/// redefined on every path before its re-reads (if-diamond both arms
/// writing) now ends at its true last read instead of hogging the
/// register to the loop end. `init` (carries the entry zero from the
/// prologue) is likewise exact: `live[1]` — only when some path reaches a
/// read without passing any write does the entry zero matter.
fn build_segments(evs: &[Ev], within: &WebWalk, cfg: &StmtCfg, n: usize, cond_read: bool) -> Vec<Seg> {
    if evs.is_empty() {
        return Vec::new();
    }
    let live = cfg_live(cfg, evs, n);
    let wu: Vec<usize> = (0..evs.len())
        .filter(|&p| matches!(evs[p].kind, EvKind::WriteU))
        .collect();
    // reads at a write's own statement index happen BEFORE that write
    // (evaluation precedes the store), so a boundary read belongs to the
    // PREVIOUS segment: segment k spans writes w_k..w_{k+1} with reads in
    // (w_k, w_{k+1}]
    let can_split = !cond_read
        && wu.len() >= 2 && {
            let r0 = evs[wu[0]].region;
            wu.iter().all(|&p| evs[p].region == r0)
                && evs.iter().all(|e| {
                    !matches!(e.kind, EvKind::Read)
                        || e.idx <= evs[wu[0]].idx
                        || within.region_within(e.region, r0)
                })
    };
    let mut segs: Vec<Seg> = Vec::new();
    if can_split {
        let w1 = evs[wu[0]].idx;
        // init window: every event strictly before the first unconditional
        // write, plus reads AT it (they evaluate first and see the older
        // value). Its end additionally covers every cfg-live statement up
        // to w1 — the entry/earlier value flowing to its readers.
        let init_mapped =
            |e: &Ev| e.idx < w1 || (e.idx == w1 && matches!(e.kind, EvKind::Read));
        if evs.iter().any(|e| init_mapped(e)) {
            let mut end = 0usize;
            for e in evs.iter().filter(|e| init_mapped(e)) {
                end = end.max(e.idx);
            }
            for p in 1..=w1 {
                if live[p] {
                    end = end.max(p);
                }
            }
            segs.push(Seg { start: evs[0].idx, end, init: live[1] });
        }
        for (i, &p) in wu.iter().enumerate() {
            let start = evs[p].idx;
            let next_w = wu.get(i + 1).map(|&q| evs[q].idx);
            let mapped = |e: &Ev| match e.kind {
                EvKind::Read => e.idx > start && next_w.map_or(true, |nw| e.idx <= nw),
                _ => e.idx >= start && next_w.map_or(true, |nw| e.idx < nw),
            };
            let mut end = start;
            for e in evs.iter().filter(|e| mapped(e)) {
                end = end.max(e.idx);
            }
            let hi = next_w.unwrap_or(n);
            for q in start + 1..=hi {
                if live[q] {
                    end = end.max(q);
                }
            }
            segs.push(Seg { start, end, init: false });
        }
        return segs;
    }
    // one segment: every event maps to it (single home)
    let mut end = 0usize;
    for e in evs {
        end = end.max(e.idx);
    }
    for p in 1..=n {
        if live[p] {
            end = end.max(p);
        }
    }
    segs.push(Seg { start: evs[0].idx, end, init: live[1] });
    segs
}

/* ---- Pass A: constant-select chains → byte-table lookup ---- */

/// Integer literal under the pinned subset (u64 domain), or None.
fn int_literal(node: &dyn IRNode) -> Option<u64> {
    crate::lang::ir_literal_value(node)
        .ok()
        .flatten()
        .and_then(|v| v.extract_u128().ok())
        .filter(|&n| n <= u64::MAX as u128)
        .map(|n| n as u64)
}

/// A bare int literal node (P0..P3/PU8/PU16) with no cast wrapper — safe
/// to fold into any width context (the CU* arms keep their range checks).
fn is_plain_int_literal(node: &dyn IRNode) -> bool {
    if node.as_any().downcast_ref::<IRNodeWrapOne>().is_some() {
        return false;
    }
    matches!(
        Bytecode::try_from_u8(node.bytecode()),
        Ok(Bytecode::P0) | Ok(Bytecode::P1) | Ok(Bytecode::P2) | Ok(Bytecode::P3)
            | Ok(Bytecode::PU8) | Ok(Bytecode::PU16)
    )
}

/// True when lowering this statement emits zero bytes (it may still have
/// vstack effects — the caller keeps lowering it normally).
fn is_zero_byte_stmt(node: &dyn IRNode) -> bool {
    if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
        || node.as_any().downcast_ref::<IRNodeText>().is_some()
    {
        return true;
    }
    let op = match Bytecode::try_from_u8(node.bytecode()) {
        Ok(o) => o,
        Err(_) => return false,
    };
    if matches!(op, Bytecode::NOP | Bytecode::ALLOC) {
        return true;
    }
    if matches!(op, Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR) {
        if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
            return arr.subs.iter().all(|n| is_zero_byte_stmt(&**n));
        }
    }
    false
}

/// True when control cannot fall out of the end of this statement
/// (return/throw/abort/break/continue): a following jump is dead.
fn ends_in_unconditional_jump(node: &dyn IRNode) -> bool {
    let mut n = node;
    loop {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            n = &*w.node;
            continue;
        }
        let op = match Bytecode::try_from_u8(n.bytecode()) {
            Ok(o) => o,
            Err(_) => return false,
        };
        if matches!(
            op,
            Bytecode::RET | Bytecode::END | Bytecode::ABT | Bytecode::ERR | Bytecode::IRBREAK | Bytecode::IRCONTINUE
        ) {
            return true;
        }
        if matches!(op, Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR) {
            if let Some(arr) = n.as_any().downcast_ref::<IRNodeArray>() {
                if let Some(last) = arr.subs.last() {
                    n = &**last;
                    continue;
                }
            }
        }
        return false;
    }
}

/// Allocate the bank: interval-shared register assignment over the web
/// segments. Greedy by slot access count; slots whose segments do not all
/// fit are un-banked entirely (frame homes) and the pass repeats.
/// Merge (and coalesce adjacent) closed statement intervals.
fn merge_spans(mut v: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    v.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(v.len());
    for (s, e) in v {
        match out.last_mut() {
            Some(last) if s <= last.1.saturating_add(1) => {
                last.1 = last.1.max(e);
            }
            _ => out.push((s, e)),
        }
    }
    out
}

/// A bank candidate register and its busy intervals: closed statement
/// ranges during which the expression machinery may write the register
/// (empty for always-free registers). A bank segment may claim the
/// register only when its live range avoids every busy interval.
type BankCand = (u8, Vec<(usize, usize)>);

fn allocate_bank(mut within: WebWalk, counts: &[usize], regs: Vec<BankCand>) -> WebPlan {
    let nslots = within.slot.len();
    if diag_env_enabled("RV64_EV_DEBUG") {
        for s in 0..nslots {
            let evs = &within.ev[s];
            if !evs.is_empty() {
                let idxs: Vec<String> = evs
                    .iter()
                    .map(|e| format!("{}{}", e.idx, match e.kind { EvKind::Read => "r", EvKind::WriteU => "W", EvKind::WriteC => "w" }))
                    .collect();
                println!("  ev slot {s}: {}", idxs.join(" "));
            }
        }
    }
    let mut segs: Vec<Vec<Seg>> = Vec::with_capacity(nslots);
    let n = within.idx;
    let cfg = StmtCfg::new(n, &within.while_spans, &within.ifrs, &within.brk_cont);
    // BUG-023: per-slot read-statement sets, snapshotted before the events
    // are moved into build_segments — the placement conflict test needs
    // them to tell a safe read->write register chain from a clobbering
    // touch at a shared boundary statement.
    let read_at: Vec<std::collections::HashSet<usize>> = (0..nslots)
        .map(|s| {
            within.ev[s]
                .iter()
                .filter(|e| matches!(e.kind, EvKind::Read))
                .map(|e| e.idx)
                .collect()
        })
        .collect();
    for s in 0..nslots {
        let mut evs = std::mem::take(&mut within.ev[s]);
        evs.sort_by_key(|e| (e.idx, !matches!(e.kind, EvKind::Read)));
        segs.push(build_segments(&evs, &within, &cfg, n, within.cond_read[s]));
    }
    struct Unit {
        slot: usize,
        seg: usize,
        start: usize,
        end: usize,
        bytes: bool,
        prio: usize,
    }
    let mut units: Vec<Unit> = Vec::new();
    for s in 0..nslots {
        if !matches!(within.slot[s], SlotTy::Int | SlotTy::Bytes) {
            continue;
        }
        let bytes = matches!(within.slot[s], SlotTy::Bytes);
        for (i, seg) in segs[s].iter().enumerate() {
            // Init segments used to map to [0, usize::MAX] — a whole-program
            // register hog. Only [0, seg.end] is actually live: the entry
            // zero is read within [seg.start, seg.end] and no read of the
            // slot can precede seg.start (it is the slot's first event), so
            // any other unit granted the register strictly after seg.end
            // cannot clobber a value anyone reads. The closed-overlap (and
            // busy) conflict test blocks every sharer inside [0, seg.end],
            // which is exactly the prologue-to-last-read window the old
            // comment worried about.
            let start = if seg.init { 0 } else { seg.start };
            let end = seg.end;
            units.push(Unit {
                slot: s,
                seg: i,
                start,
                end,
                bytes,
                // slot-global count priority (per-unit traffic × loop-depth
                // lifts were tried in S2 and measured WORSE on both anchors:
                // fragmenting the allocation by per-unit traffic starves
                // whole-slot residency — see docs.loc/rv64im/12-* §6)
                prio: counts[s],
            });
        }
    }
    units.sort_by(|a, b| {
        b.prio
            .cmp(&a.prio)
            .then(a.start.cmp(&b.start))
            .then(a.end.cmp(&b.end))
            .then(a.slot.cmp(&b.slot))
            .then(a.seg.cmp(&b.seg))
    });

    let mut banked = vec![BankKind::None; nslots];
    let mut homes: Vec<Vec<(usize, [u8; 2])>> = vec![Vec::new(); nslots];
    let mut assigned: Vec<Vec<(usize, usize)>>; // (list pos, unit idx)

    loop {
        assigned = (0..regs.len()).map(|_| Vec::new()).collect();
        banked = vec![BankKind::None; nslots];
        homes = vec![Vec::new(); nslots];
        let mut failed: Vec<usize> = Vec::new();
        let mut slot_placed = vec![false; nslots];
        let mut evicted: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut fail_why: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        for (uidx, u) in units.iter().enumerate() {
            if evicted.contains(&uidx) {
                continue;
            }
            if !slot_placed[u.slot] && failed.contains(&u.slot) {
                continue;
            }
            let need = if u.bytes { 2 } else { 1 };
            let mut placed: Vec<usize> = Vec::new();
            for ri in 0..regs.len() {
                placed.clear();
                let mut blockers: Vec<usize> = Vec::new();
                let mut ok = true;
                for k in 0..need {
                    let r = ri + k;
                    if r >= regs.len() {
                        ok = false;
                        break;
                    }
                    for &(_, uu) in &assigned[r] {
                        let a = &units[uu];
                        // BUG-023 (P2-C4 S6, 10^4 bombardment): Seg.end is
                        // INCLUSIVE (the value is live AT its end statement
                        // — the cfg fixpoint extends it across the backedge
                        // for loop counters), so the old strict test
                        // (`u.start < a.end`) treated ends as exclusive and
                        // co-located units that merely TOUCH at a boundary
                        // statement. Two shapes, both observed:
                        //   * the placed unit starts where the current one
                        //     ends (`while t0 < 1 { t0 += 1; var t1 = 0u64 }`
                        //     banked t1's init into t0's home — the counter
                        //     reset every iteration and ImcRemap spun to
                        //     OutOfGas while Im/Imc (frame homes, no
                        //     sharing) terminated);
                        //   * the placed unit ENDS where the current one
                        //     starts (priority order places a later-starting
                        //     unit first; its boundary write then clobbers
                        //     the current unit's backedge-live value).
                        // A touch IS safe exactly when the ENDING unit's
                        // slot READS at the boundary statement: reads
                        // evaluate before same-statement writes, so the
                        // value flows through the shared register (`t1 =
                        // t0 + 1` chains — the coalescing the strict test
                        // was written for). Same-slot touches are always
                        // safe (one value, in-place update at the boundary
                        // write).
                        let strict = a.start < u.end && u.start < a.end;
                        let mut touch_unsafe = false;
                        if !strict && a.slot != u.slot {
                            // The read-at-boundary exemption has a second,
                            // loop-shaped hole (BUG-025/026, P2-C4 S2 10^5
                            // round; seeds 0xB0DB...+94042's 4-line delta-min
                            // `var s0 = buf_left_drop(2, buf_left(8, data));
                            // while t0 < 16 { var s1 = buf_left_drop(1, s0) }`
                            // and an 11-line nested-loop spin): if the
                            // ENDING unit is LOOP-CARRIED into a while span
                            // containing the boundary — its def precedes the
                            // span (s0's def is stmt 1, the boundary sits at
                            // stmt 3 inside span (2,..)) — its value must
                            // survive that span's iterations, because its
                            // in-span reads re-execute every iteration with
                            // NO intervening re-def. The surviving slot's
                            // boundary write then clobbers the shared home
                            // and the next iteration's re-reads see the
                            // wrong value (s0's backedge re-read saw s1's
                            // handle — slice count check faulted 71; or a
                            // sharer took a loop counter's home and
                            // ImcRemap spun to OutOfGas) while Im/Imc
                            // (frame homes, no sharing) and the interpreter
                            // ran clean. A unit whose def lies INSIDE every
                            // span containing the boundary is safe: the
                            // next iteration re-executes its def before any
                            // of its reads (the hacd200 anchor's 8 in-loop
                            // boundary chains are all of this shape and
                            // keep their homes — 3072 B pinned).
                            let carried_into = |unit_start: usize, k: usize| {
                                within
                                    .while_spans
                                    .iter()
                                    .any(|&(w0, w1)| w0 <= k && k <= w1 && unit_start < w0)
                            };
                            if a.end == u.start
                                && (carried_into(a.start, a.end)
                                    || !read_at[a.slot].contains(&a.end))
                            {
                                touch_unsafe = true;
                            }
                            if u.end == a.start
                                && (carried_into(u.start, u.end)
                                    || !read_at[u.slot].contains(&u.end))
                            {
                                touch_unsafe = true;
                            }
                        }
                        if (strict || touch_unsafe) && !blockers.contains(&uu) {
                            blockers.push(uu);
                        }
                    }
                    if regs[r].1.iter().any(|&(b0, b1)| b0 <= u.end && u.start <= b1) {
                        ok = false;
                    }
                    if !ok {
                        placed.clear();
                        break;
                    }
                    placed.push(r);
                }
                if !ok || placed.len() != need {
                    continue;
                }
                if blockers.iter().any(|&b| b <= uidx) {
                    // this window is unusable: clear `placed` so the last
                    // attempted window cannot leak past the failed check
                    // below (the BUG-012 double-assignment: with every
                    // window conflicted, a filled `placed` escaped the
                    // `placed.len() != need` guard and two slots shared a
                    // register — wrong values, no INV_CHECK hit)
                    placed.clear();
                    continue;
                }
                for b in blockers.drain(..) {
                    for a in assigned.iter_mut() {
                        a.retain(|&(_, uu)| uu != b);
                    }
                    evicted.insert(b);
                }
                break;
            }
            if placed.len() != need {
                failed.push(u.slot);
                continue;
            }
            slot_placed[u.slot] = true;
            for r in &placed {
                assigned[*r].push((*r, uidx));
            }
        }
        // derive banked flags + homes from what survived the round
        for (uidx, u) in units.iter().enumerate() {
            let mut pair = [0u8; 2];
            let mut got = 0usize;
            for (k, occ) in assigned.iter().enumerate() {
                if occ.iter().any(|&(_, uu)| uu == uidx) {
                    pair[got] = regs[k].0;
                    got += 1;
                }
            }
            if got == 0 {
                continue;
            }
            debug_assert_eq!(got, if u.bytes { 2 } else { 1 }, "split placement");
            homes[u.slot].push((u.start, pair));
        }
        for (s, h) in homes.iter_mut().enumerate() {
            h.sort_by_key(|(b, _)| *b);
            h.dedup();
            if !h.is_empty() {
                banked[s] = if matches!(within.slot[s], SlotTy::Bytes) {
                    BankKind::Bytes
                } else {
                    BankKind::Int
                };
            }
        }
        if failed.is_empty() {
            if diag_env_enabled("RV64_EV_DEBUG") {
                for (uidx, u) in units.iter().enumerate() {
                    let got = assigned
                        .iter()
                        .filter(|occ| occ.iter().any(|&(_, uu)| uu == uidx))
                        .count();
                    let want = if u.bytes { 2 } else { 1 };
                    if got != 0 && got != want {
                        println!(
                            "  bank-PARTIAL slot {} seg{} [{},{}] got {} want {}",
                            u.slot, u.seg, u.start, u.end, got, want
                        );
                    }
                    if got == 0 {
                        println!(
                            "  bank-EVICTED slot {} seg{} [{},{}]",
                            u.slot, u.seg, u.start, u.end
                        );
                    }
                }
            }
            break;
        }
        if diag_env_enabled("RV64_EV_DEBUG") {
            let mut keys: Vec<_> = fail_why.keys().copied().collect();
            keys.sort();
            for s in keys {
                println!("  bank-FAIL {}", fail_why[&s]);
            }
        }
        // un-bank failed slots entirely and retry (their frame homes return)
        let before = units.len();
        units.retain(|u| !failed.contains(&u.slot));
        if units.len() == before {
            if diag_env_enabled("RV64_EV_DEBUG") {
                let mut keys: Vec<_> = fail_why.keys().copied().collect();
                keys.sort();
                for s in keys {
                    println!("  bank-FAIL {}", fail_why[&s]);
                }
                for (ri, (r, busy)) in regs.iter().enumerate() {
                    let spans: Vec<String> =
                        busy.iter().map(|&(b0, b1)| format!("[{b0},{b1}]")).collect();
                    let occ: Vec<String> = assigned[ri]
                        .iter()
                        .map(|&(_, uu)| format!("s{}[{},{}]", units[uu].slot, units[uu].start, units[uu].end))
                        .collect();
                    println!(
                        "  bank-reg x{} busy={} occ={}",
                        r,
                        spans.join(""),
                        occ.join(",")
                    );
                }
            }
            break;
        }
    }
    for h in homes.iter_mut() {
        h.sort_by_key(|(b, _)| *b);
    }
    if diag_env_enabled("RV64_EV_DEBUG") {
        // P2-A27-S2: per-slot segment table + final homes (the structure the
        // census/probe diffing reads; no codegen effect)
        let banked_n = banked
            .iter()
            .filter(|&&k| matches!(k, BankKind::Int | BankKind::Bytes))
            .count();
        println!("  bank-final: {banked_n} banked slots, {n} stmts");
        for s in 0..nslots {
            let sg: Vec<String> = segs[s]
                .iter()
                .map(|g| {
                    format!(
                        "[{},{}]{}",
                        g.start,
                        g.end,
                        if g.init { "i" } else { "" }
                    )
                })
                .collect();
            let hm: Vec<String> = homes[s]
                .iter()
                .map(|(b, r)| {
                    format!(
                        "{}->x{}{}",
                        b,
                        r[0],
                        if matches!(banked.get(s), Some(BankKind::Bytes)) {
                            format!("x{}", r[1])
                        } else {
                            String::new()
                        }
                    )
                })
                .collect();
            let kind = match banked.get(s) {
                Some(BankKind::Int) => "INT",
                Some(BankKind::Bytes) => "BYTES",
                _ => "frame",
            };
            println!("  seg slot {s} {kind}: {} homes={}", sg.join(" "), hm.join(" "));
        }
    }
    let mut init_zero = vec![false; nslots];
    for (s, sg) in segs.iter().enumerate() {
        init_zero[s] = sg.first().map(|x| x.init).unwrap_or(false);
    }
    WebPlan { banked, homes, init_zero, segs_debug: segs, no_split_debug: within.cond_read.clone() }
}

/// Compute the whole promotion plan for a body: event walk + segments +
/// bank allocation. Each candidate register carries its busy intervals —
/// closed statement ranges during which the expression machinery may write
/// it (pool registers above the base depth, see `lower_body_mode_arena`);
/// a segment's live range must avoid all of them.
fn web_plan(ir: &IRNodeArray, fp: &FramePlan, regs_available: &[BankCand]) -> WebPlan {
    let mut w = WebWalk::new(&fp.slot);
    // XOP's old-value read MUST be visible to the segment builder: the
    // read-modify-write evaluates the old value (from the PREVIOUS segment's
    // register — `Lower`'s XOP arm reads `bank_int_read` at the write's own
    // statement index) before the new segment's write lands. With the legacy
    // pure-write plan events that boundary read was invisible, so a segment
    // ended early and the allocator freely shared its register with another
    // slot inside (seg.end, xop idx) — the XOP then read the sharer's value
    // (BUG-008: `a=7; b=a; c=9; a+=1` returned 10 in ImcRemap). Recording
    // the read makes the old segment's unit cover the boundary statement, so
    // the overlap check protects it and `home_at`'s read-side coverage
    // invariant holds for every emitted read.
    w.xop_reads = true;
    for n in &ir.subs {
        w.walk_stmt(&**n);
    }
    let mirror: usize = ir.subs.iter().map(|n| stmt_walk_count(&**n)).sum();
    debug_assert_eq!(mirror, w.idx, "stmt_walk_count mirror desync");
    let (reads, writes) = slot_use_counts(&ir.subs);
    let mut counts = vec![0usize; fp.slot.len()];
    for (i, c) in counts.iter_mut().enumerate() {
        *c = reads.get(i).copied().unwrap_or(0) + writes.get(i).copied().unwrap_or(0);
    }
    allocate_bank(w, &counts, regs_available.to_vec())
}

/// Count the sites that will reference a pool literal (PBUF-family
/// evaluations, literal PUT/CBYTES/CAT folds) — drives the pool-base
/// register decision.
fn count_literal_sites(nodes: &[Box<dyn IRNode>]) -> usize {
    fn walkc(n: &dyn IRNode, count: &mut usize) {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walkc(&*w.node, count);
        }
        let op = match Bytecode::try_from_u8(n.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        use Bytecode::*;
        match op {
            PBUF | PBUFL | PNBUF => *count += 1,
            PUT => {
                if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                    if bytes_literal(&*p.subx).is_some() {
                        *count += 1;
                    }
                }
            }
            CBYTES => {
                if let Some(single) = n.as_any().downcast_ref::<IRNodeSingle>() {
                    if crate::lang::ir_literal_value(&*single.subx).ok().flatten().is_some() {
                        *count += 1;
                    }
                }
            }
            CAT => {
                if let Some(d) = n.as_any().downcast_ref::<IRNodeDouble>() {
                    if bytes_literal(&*d.subx).is_some() && bytes_literal(&*d.suby).is_some() {
                        *count += 1;
                    }
                }
            }
            _ => {}
        }
        for c in children(n) {
            walkc(c, count);
        }
    }
    let mut count = 0;
    for n in nodes {
        walkc(&**n, &mut count);
    }
    count
}

/// Static per-slot access counts over the whole body: (reads, writes),
/// where XOP counts once each way (it reads and writes the slot).
fn slot_use_counts(nodes: &[Box<dyn IRNode>]) -> (Vec<usize>, Vec<usize>) {
    fn walkc(n: &dyn IRNode, reads: &mut Vec<usize>, writes: &mut Vec<usize>) {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walkc(&*w.node, reads, writes);
        }
        let op = match Bytecode::try_from_u8(n.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        use Bytecode::*;
        match op {
            GET | GET0 | GET1 | GET2 | GET3 => {
                let idx = match op {
                    GET => {
                        n.as_any().downcast_ref::<IRNodeParam1>().map(|p| p.para as usize)
                    }
                    GET0 => Some(0),
                    GET1 => Some(1),
                    GET2 => Some(2),
                    _ => Some(3),
                };
                if let Some(i) = idx {
                    if reads.len() <= i {
                        reads.resize(i + 1, 0);
                        writes.resize(i + 1, 0);
                    }
                    reads[i] += 1;
                }
            }
            PUT => {
                if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                    let i = p.para as usize;
                    if writes.len() <= i {
                        reads.resize(i + 1, 0);
                        writes.resize(i + 1, 0);
                    }
                    writes[i] += 1;
                }
            }
            XOP => {
                if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                    let (_, idx) = decode_local_operand_mark(p.para);
                    let i = idx as usize;
                    if writes.len() <= i {
                        reads.resize(i + 1, 0);
                        writes.resize(i + 1, 0);
                    }
                    reads[i] += 1;
                    writes[i] += 1;
                }
            }
            _ => {}
        }
        for c in children(n) {
            walkc(c, reads, writes);
        }
    }
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for n in nodes {
        walkc(&**n, &mut reads, &mut writes);
    }
    (reads, writes)
}

/// Index of the slot a GET/GET0..3 node reads, or None.
fn slot_get_index(node: &dyn IRNode) -> Option<u8> {
    let op = Bytecode::try_from_u8(node.bytecode()).ok()?;
    match op {
        Bytecode::GET => Some(node.as_any().downcast_ref::<IRNodeParam1>()?.para),
        Bytecode::GET0 => Some(0),
        Bytecode::GET1 => Some(1),
        Bytecode::GET2 => Some(2),
        Bytecode::GET3 => Some(3),
        _ => None,
    }
}

/// The single effective statement of an arm body (looking through wrappers
/// and single-statement blocks), or None when the body is empty/compound.
fn effective_single(node: &dyn IRNode) -> Option<&dyn IRNode> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return effective_single(&*w.node);
    }
    if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
        let mut eff = arr.subs.iter().filter(|s| !is_zero_byte_stmt(&***s));
        let first = eff.next()?;
        if eff.next().is_some() {
            return None;
        }
        return effective_single(&**first);
    }
    if is_zero_byte_stmt(node) {
        return None;
    }
    Some(node)
}

/// One matched arm of a constant-select chain.
struct SelArm {
    key: u64,
    val: u64,
}

/// Match `IRIF (EQ <GET s> <int lit>) { PUT w <int lit ≤ 255> }` with an
/// empty else. Returns (read slot, write slot, key, value).
fn match_sel_arm(node: &dyn IRNode) -> Option<(u8, u8, SelArm)> {
    let n = node.as_any().downcast_ref::<IRNodeTriple>()?;
    if n.inst != Bytecode::IRIF {
        return None;
    }
    let c = n.subx.as_any().downcast_ref::<IRNodeDouble>()?;
    if c.inst != Bytecode::EQ {
        return None;
    }
    let (g, k) = match (
        slot_get_index(&*c.subx),
        int_literal(&*c.subx),
        slot_get_index(&*c.suby),
        int_literal(&*c.suby),
    ) {
        (Some(s), None, None, Some(k)) => (s, k),
        (None, Some(k), Some(s), None) => (s, k),
        _ => return None,
    };
    let body = effective_single(&*n.suby)?;
    let p = body.as_any().downcast_ref::<IRNodeParam1Single>()?;
    if p.inst != Bytecode::PUT {
        return None;
    }
    let v = int_literal(&*p.subx)?;
    if v > 255 {
        return None;
    }
    if !is_zero_byte_stmt(&*n.subz) {
        return None;
    }
    Some((g, p.para, SelArm { key: k, val: v }))
}

/// True when the statement before a chain (skipping zero-byte statements)
/// is an unconditional `PUT slot D` — the default value D that table holes
/// must reproduce (nothing else writes the slot between D and the chain).
fn default_predecessor(subs: &[Box<dyn IRNode>], run_start: usize, slot: u8) -> Option<u64> {
    let mut i = run_start;
    while i > 0 {
        i -= 1;
        let n = &*subs[i];
        if is_zero_byte_stmt(n) {
            continue;
        }
        let p = n.as_any().downcast_ref::<IRNodeParam1Single>()?;
        if p.inst != Bytecode::PUT || p.para != slot {
            return None;
        }
        return int_literal(&*p.subx).filter(|v| *v <= 255);
    }
    None
}

/// True when the condition subtree cannot fault (literals, untracked slot
/// reads, comparisons over fault-free operands, logical ops over fault-free
/// operands only). Fault-free conjunctions can be lowered as branch chains
/// without changing observable behavior; impure ones must keep eager
/// evaluation (every operand must still execute — the bytecode channel is
/// eager for AND/OR, so a skipped operand would swallow a consensus-visible
/// trap; BUG-014). A read of a Nil-TRACKED slot counts as faultable: its
/// consumer guard must fire (BUG-012).
fn is_pure_cond(node: &dyn IRNode, tracked: &[bool]) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return is_pure_cond(&*w.node, tracked);
    }
    let op = match Bytecode::try_from_u8(node.bytecode()) {
        Ok(o) => o,
        Err(_) => return false,
    };
    use Bytecode::*;
    match op {
        P0 | P1 | P2 | P3 | PU8 | PU16 | PTRUE | PFALSE => true,
        GET | GET0 | GET1 | GET2 | GET3 => match slot_get_index(node) {
            Some(i) => !tracked.get(i as usize).copied().unwrap_or(false),
            None => true,
        },
        // comparisons and logical wraps are fault-free ONLY when every
        // operand is: `byte(data,25) > 0` faults, so it must not join a
        // short-circuit chain
        EQ | NEQ | LT | GT | LE | GE | NOT | AND | OR => {
            children(node).iter().all(|c| is_pure_cond(*c, tracked))
        }
        _ => false,
    }
}

struct Lower<'a> {
    asm: Asm,
    slot: &'a [SlotTy],
    needs_zero: &'a [bool],
    /// Value pool for this mode (prefix of `VSTACK_ORDER_RVC` in
    /// `ImcRemap`, sized to the body's peak expression depth; the classic
    /// fixed 8-reg pools otherwise).
    vregs: Vec<u8>,
    /// Promoted-local bank: per-slot kind + per-segment register homes.
    plan: WebPlan,
    /// Statement-index cursor mirroring the web walk (drives `home_at`).
    web_idx: usize,
    /// Loop/address scratch registers. In `ImcRemap` these move off the ABI
    /// a0/a1 (x10/x11) — they become pool slots — onto t0/x5 and SC2C/x31
    /// (P2-A27 S0: a1 left t1/x6, which joins the bank; x5 must stay —
    /// `emit_cbytes`/`emit_return` need three concurrent scratches); ABI-
    /// boundary writes (exit code, return length) keep using the x10/x11 consts.
    a0: u8,
    a1: u8,
    /// P2-A27 S1 (probe only): record per-vstack-position liveness spans —
    /// `pos_spans[k]` = statements during which stack position `k` (hence
    /// pool register `vregs[k]`) holds a live value. The probe lowers with
    /// an empty plan, so every operand materializes as a Pool push: its
    /// spans are a sound over-approximation of the real (banked) lowering's
    /// pool-register writes, which is what the busy-candidate conflict test
    /// needs. Recorded on `push_vreg`/`push_ref`/`pop_vreg` and the two
    /// direct re-push sites (`pop_own_replace`, value-if arm unification).
    record_spans: bool,
    span_open: [Option<usize>; 8],
    pos_spans: Vec<Vec<(usize, usize)>>,
    /// P2-A27 S1 (INV_CHECK only): flattened bank-home liveness —
    /// (reg, seg_start, seg_end, is_init) per segment. `push_vreg` asserts
    /// the pool register it hands out is not a live bank home at the
    /// current statement (the plan-level busy test's runtime mirror).
    plan_reg_live: Vec<(u8, usize, usize, bool)>,
    /// Arena end address for the exhaustion check.
    arena_end: u64,
    vstack: Vec<VLoc>,
    /// Set when the body nests deeper than the register pool; the lowered
    /// output is rejected (subset limit, see `push_vreg`).
    depth_err: bool,
    /// Peak vstack depth observed (diagnostics; drives local-bank sizing).
    max_depth: usize,
    frame_bytes: usize,
    /// (label, bytes) in first-use order; laid out consecutively in the pool.
    pool_entries: Vec<(usize, Vec<u8>)>,
    pool_map: HashMap<Vec<u8>, usize>,
    break_labels: Vec<usize>,
    continue_labels: Vec<usize>,
    trap_label: usize,
    fault_arith: usize,
    fault_cast: usize,
    /// Whether the `PUT 0 (ROLL0)` param binding has been consumed.
    param_bound: bool,
    /// Function-wide literal-pool base register (prologue pcrel + per-site
    /// `addi rd, base, off` — 2-byte c.addi per literal reference instead
    /// of an 8-byte auipc+addi pair). None when too few literals pay for it.
    pool_base: Option<(u8, usize)>,
    /// Frame byte offset per slot: un-promoted slots laid out hottest-first
    /// so the most-accessed stay inside the c.ld/c.sd 248-byte window.
    frame_off: Vec<i32>,
    /// Store-to-load forwarding: the pool register that still holds the last
    /// stored value of each un-banked frame slot (killed at every join label
    /// and whenever the register is re-handed by `push_vreg`).
    fwd_int: Vec<Option<u8>>,
    fwd_bytes: Vec<Option<(u8, u8)>>,
    /// Highest statement index at which each slot is read (from the web
    /// walk's event timeline); a store at an index ≥ this is dead.
    last_read: Vec<usize>,
    /// Sorted per-slot read indices + while spans (backedge liveness: a
    /// read earlier in linear order than a store, but inside a loop span
    /// that contains the store, re-executes after it).
    read_idx: Vec<Vec<usize>>,
    while_spans: Vec<(usize, usize)>,
    /// Statement index of an elidable `var x = 0` store per slot (the
    /// prologue's forced frame zero replaces it).
    zero_init_skip: Vec<Option<usize>>,
    /// Slots read by a while condition (loop-carried): their writes are
    /// re-read by the backedge invisibly, so dead-store elimination and
    /// forwarding-invalid timing must both stand down for them.
    loop_carried: Vec<bool>,
    /// Debug side-channel (P0 experiments): when `Some`, `stmt_one` /
    /// `stmt_list` record per-statement code-offset ranges into it.
    /// Write-only bookkeeping — no existing path reads it, so lowering
    /// stays byte-identical (pinned by `dbg_engine_matches_production`).
    stmt_trace: Option<Vec<StmtRangeDbg>>,
    /* ---- BUG-012 Nil tracking ---- */
    /// per-node may-Nil info from the pre-pass (keyed by node address)
    nil_occ: HashMap<usize, NilOcc>,
    /// P3-IDIOMS: byte-assembly fusion marks (Root/Skip), computed once
    /// by fuse_scan() outside the relaxation loop (deterministic).
    fuse: FuseMap,
    /// P3-PAIRPKG: pair marks from plan() — Root on REMOVE statement nodes
    /// (the fused single-`sd` emission site), Skip on the INSERT sibling
    /// (fully subsumed). Consulted at the two statement-sequence dispatch
    /// sites only; the statement walk never meets these opcodes unmarked.
    pair_root: HashMap<usize, PairRec>,
    pair_skip: HashSet<usize>,
    /// slot idx → carries an init flag byte
    tracked: Vec<bool>,
    /// per-slot init-flag frame offset (-1 = untracked)
    flag_off: Vec<i32>,
    /// frame offset of the 8-byte tag stack (-1 = no tracking at all)
    tag_base: i32,
    /// lazily created Nil-guard fault thunks: (label, exit code); bodies
    /// emitted at the end of lowering only when used
    nil_thunks: Vec<(usize, u64)>,
    /// frame offset of the first init-flag byte (prologue zeroing)
    nil_flag_start: i32,
    /* ---- P3-V2IMPL: compo-list v2 shapes ---- */
    /// per-slot storage classes + rodata values (from the plan() walk)
    lists: ListPlanData,
    /// frame byte offset of each list's element data block (-1 = none)
    list_data_off: Vec<i32>,
    /// pool label of each list's rodata table, registered at the (single)
    /// creation site (-1 = none yet)
    list_lbl: Vec<usize>,
}

enum SliceKind {
    Cut,
    Left,
    Right,
    LDrop,
    RDrop,
}

impl<'a> Lower<'a> {
    #[cfg(test)] // the test harness lowers through these thin wrappers
    fn new(slot: &'a [SlotTy], needs_zero: &'a [bool]) -> Self {
        Self::new_mode(slot, needs_zero, Rv64Mode::Im)
    }

    #[cfg(test)]
    fn new_mode(slot: &'a [SlotTy], needs_zero: &'a [bool], mode: Rv64Mode) -> Self {
        Self::new_mode_arena(slot, needs_zero, mode, ARENA_SIZE)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn new_mode_arena(
        slot: &'a [SlotTy],
        needs_zero: &'a [bool],
        mode: Rv64Mode,
        arena_size: u64,
    ) -> Self {
        Self::new_mode_arena_nil(
            slot,
            needs_zero,
            mode,
            arena_size,
            &NilPlan { tracked: vec![false; slot.len()], occ: HashMap::new() },
            &ListPlanData::default(),
        )
    }

    /// Like [`Self::new_mode_arena`] but carrying the Nil-tracking plan
    /// (the ImcRemap probe must see the same guard/tag emission shapes as
    /// the real lowering).
    fn new_mode_arena_nil(
        slot: &'a [SlotTy],
        needs_zero: &'a [bool],
        mode: Rv64Mode,
        arena_size: u64,
        nilp: &NilPlan,
        lists: &ListPlanData,
    ) -> Self {
        let order: Vec<usize> = (0..slot.len()).collect();
        Self::new_mode_arena_bank(
            slot,
            needs_zero,
            mode,
            arena_size,
            8,
            WebPlan { banked: Vec::new(), homes: Vec::new(), init_zero: Vec::new(), segs_debug: Vec::new(), no_split_debug: Vec::new() },
            None,
            &order,
            nilp,
            lists,
        )
    }

    /// `pool_size`/`plan`: the ImcRemap promotion plan computed by
    /// `lower_body_mode_arena` (pool shrunk to the body's peak expression
    /// depth; web segments of hot locals share the freed registers).
    /// `Im`/`Imc` always pass the fixed 8-slot pool and an empty plan.
    /// `lists` is the P3-V2IMPL list storage plan (all-`NotList` for
    /// list-free bodies — the historical layout, byte for byte).
    #[allow(clippy::too_many_arguments)]
    fn new_mode_arena_bank(
        slot: &'a [SlotTy],
        needs_zero: &'a [bool],
        mode: Rv64Mode,
        arena_size: u64,
        pool_size: usize,
        plan: WebPlan,
        pool_base_reg: Option<u8>,
        frame_order: &[usize],
        nilp: &NilPlan,
        lists: &ListPlanData,
    ) -> Self {
        // The memory frame holds only the un-promoted slots, laid out in
        // `frame_order` (hottest first) so the most-accessed offsets stay
        // within the c.ld/c.sd ±248-byte compressed window.
        let mut frame_off = vec![-1i32; slot.len()];
        let mut acc = 0i32;
        for &i in frame_order {
            if i >= slot.len() || frame_off[i] >= 0 {
                continue;
            }
            if plan.banked.get(i).copied().unwrap_or(BankKind::None) != BankKind::None {
                continue;
            }
            frame_off[i] = acc;
            acc += match slot[i] {
                SlotTy::Bytes => 16,
                SlotTy::Input => 0,
                _ => 8,
            };
        }
        // remaining slots (not named in frame_order) append in index order
        for (i, st) in slot.iter().enumerate() {
            if frame_off[i] >= 0 || matches!(st, SlotTy::Input) {
                continue;
            }
            if plan.banked.get(i).copied().unwrap_or(BankKind::None) != BankKind::None {
                continue;
            }
            frame_off[i] = acc;
            acc += match st {
                SlotTy::Bytes => 16,
                SlotTy::Input => 0,
                _ => 8,
            };
        }
        // BUG-012 Nil-tracking bytes: one init flag per tracked slot, then
        // an 8-byte tag stack (one tag byte per vstack position). `acc` is
        // 8-aligned (every slot size is a multiple of 8), so the whole
        // region is word-aligned for the prologue's sd-zeroing.
        let tracked = nilp.tracked.clone();
        let mut flag_off = vec![-1i32; slot.len()];
        let mut tag_base = -1i32;
        let nil_flag_start = acc;
        if tracked.iter().any(|&t| t) {
            for (i, &t) in tracked.iter().enumerate() {
                if t {
                    flag_off[i] = acc;
                    acc += 1;
                }
            }
            acc = (acc + 7) & !7;
            tag_base = acc;
            acc += 8;
        }
        // P3-V2IMPL: list element data blocks, after the Nil region, in
        // slot-index order (a deterministic function of the body — no
        // iteration-order dependence). Growable lists reserve the fixed
        // 128×8B = 1024B (decision D2); static lists take N×8B; rodata /
        // empty lists take nothing (the slot's own 8B cell — allocated in
        // the slot loops above — is the growable length shadow and dead
        // for the static classes).
        let mut list_data_off = vec![-1i32; slot.len()];
        for (i, lk) in lists.kind.iter().enumerate() {
            if i >= slot.len() {
                break;
            }
            let bytes = match lk {
                ListKind::FrameStatic { n } => (*n as usize) * 8,
                ListKind::Growable { .. } => 1024,
                _ => continue,
            };
            list_data_off[i] = acc;
            acc += bytes as i32;
        }
        let frame_bytes = (acc as usize).max(16).next_multiple_of(16);
        let mut asm = match mode {
            Rv64Mode::Im => Asm::new(),
            // BUG-024: Imc's pool is Im's VSTACK_REGS (x30/x31 are pool
            // positions 5/6), so its scratches move to x12/x13; ImcRemap's
            // pool excludes x30/x31 and keeps SC1C/SC2C.
            Rv64Mode::Imc => Asm::new_rvc_scratch(SC1I, SC2I),
            Rv64Mode::ImcRemap => Asm::new_rvc(),
        };
        let vregs: Vec<u8> = match mode {
            Rv64Mode::ImcRemap => VSTACK_ORDER_RVC[..pool_size.min(8)].to_vec(),
            _ => VSTACK_REGS.to_vec(),
        };
        let (a0, a1) = match mode {
            // P2-A27 S0 (docs.loc/rv64im/12-*): a1 moves off T1/x6 — every
            // a1 use site (mulhu/remu guards, shift masks, copy loops) is
            // disjoint from asm.sc2's transient lifetime inside `li`, and a1
            // is never a `li` destination — so SC2C (x31) substitutes 1:1.
            // This frees x6 for the local bank. a0 stays on T0/x5:
            // emit_cbytes/emit_return need THREE concurrent scratches
            // (a0, a1, asm.sc1) and only x30/x31 are always-dead, so x5
            // cannot be freed without rewriting those sequences.
            Rv64Mode::ImcRemap => (T0, SC2C),
            _ => (A0, A1),
        };
        let arena_end = ARENA_BASE + arena_size;
        let trap_label = asm.new_label();
        let fault_arith = asm.new_label();
        let fault_cast = asm.new_label();
        let pool_base = pool_base_reg.map(|r| {
            let l = asm.new_label();
            (r, l)
        });
        // P2-A27 S1 (INV_CHECK only): flatten the bank homes into per-register
        // closed live intervals for the push_vreg assertion. Init segments
        // carry homes keyed at 0 (the allocator maps their live window to
        // [0, seg.end]) while their `Seg.start` is the first event index —
        // match on `g.init && *b == 0` too, otherwise the assertion is blind
        // to every init home.
        let plan_reg_live: Vec<(u8, usize, usize, bool)> = {
            let mut v = Vec::new();
            for (s, hs) in plan.homes.iter().enumerate() {
                for (b, regs_pair) in hs {
                    if let Some(sg) = plan.segs_debug.get(s).and_then(|segs| {
                        segs.iter().find(|g| g.start == *b || (g.init && *b == 0))
                    }) {
                        for &r in regs_pair.iter() {
                            v.push((r, *b, sg.end, sg.init));
                        }
                    }
                }
            }
            v
        };

        Self {
            asm,
            slot,
            needs_zero,
            vregs,
            plan,
            web_idx: 0,
            pool_base,
            frame_off,
            a0,
            a1,
            record_spans: false,
            span_open: [None; 8],
            pos_spans: vec![Vec::new(); 8],
            plan_reg_live,
            arena_end,
            vstack: Vec::new(),
            depth_err: false,
            max_depth: 0,
            frame_bytes,
            pool_entries: Vec::new(),
            pool_map: HashMap::new(),
            break_labels: Vec::new(),
            continue_labels: Vec::new(),
            trap_label,
            fault_arith,
            fault_cast,
            param_bound: false,
            fwd_int: vec![None; slot.len()],
            fwd_bytes: vec![None; slot.len()],
            last_read: vec![0; slot.len()],
            read_idx: vec![Vec::new(); slot.len()],
            while_spans: Vec::new(),
            zero_init_skip: vec![None; slot.len()],
            loop_carried: vec![false; slot.len()],
            stmt_trace: None,
            nil_occ: nilp.occ.clone(),
            fuse: HashMap::new(),
            pair_root: HashMap::new(),
            pair_skip: HashSet::new(),
            tracked,
            flag_off,
            tag_base,
            nil_thunks: Vec::new(),
            nil_flag_start,
            lists: lists.clone(),
            list_data_off,
            list_lbl: vec![usize::MAX; slot.len()],
        }
    }

    /* ---- register stack ---- */

    /// P2-A27 S1: open a liveness span for vstack position `pos` at the
    /// current statement (probe recordings only).
    fn span_open_at(&mut self, pos: usize) {
        if !self.record_spans || pos >= 8 {
            return;
        }
        if self.span_open[pos].is_none() {
            self.span_open[pos] = Some(self.web_idx);
        }
    }

    /// P2-A27 S1: close the spans of positions `new_len..old_len` at the
    /// current statement (the values above the shrunk stack just died).
    fn span_close_from(&mut self, new_len: usize, old_len: usize) {
        if !self.record_spans {
            return;
        }
        for pos in new_len..old_len.min(8) {
            if let Some(o) = self.span_open[pos].take() {
                self.pos_spans[pos].push((o, self.web_idx));
            }
        }
    }

    fn push_vreg(&mut self) -> u8 {
        self.max_depth = self.max_depth.max(self.vstack.len() + 1);
        if self.vstack.len() >= self.vregs.len() {
            // Subset limit: nesting deeper than the value register pool.
            // Emission continues so diagnostics stay simple; the lowered
            // output is rejected in `lower_body_with_input` via `depth_err`.
            self.depth_err = true;
            return self.vregs[self.vregs.len() - 1];
        }
        let r = self.vregs[self.vstack.len()];
        if std::env::var("RV64_INV_CHECK").is_ok() {
            // P2-A27 S1 runtime mirror of the plan-level busy test: the
            // pool register about to be written must not hold any live
            // bank value. Non-init segments are born by a write at
            // `seg_start` (which happens after this statement's expression
            // evaluation), so a pool write at the birth statement itself is
            // harmless; anything in (start, end] is a live read window.
            // Init segments carry the entry zero from the prologue — any
            // overlap kills it.
            for (reg, s0, s1, init) in &self.plan_reg_live {
                let hit =
                    if *init { *s0 <= self.web_idx && self.web_idx <= *s1 } else { *s0 < self.web_idx && self.web_idx <= *s1 };
                if *reg == r && hit {
                    panic!(
                        "INV pool write clobbers bank home reg={} at stmt {} (seg [{},{}] init={})",
                        r, self.web_idx, s0, s1, init
                    );
                }
            }
        }
        self.span_open_at(self.vstack.len());
        self.kill_fwd_reg(r);
        self.vstack.push(VLoc::Pool(r));
        r
    }

    /* ---- store-to-load forwarding & dead stores ---- */

    /// Drop forwarding entries that watch `r` (the register is about to be
    /// overwritten by a new pool value or a bank write).
    fn kill_fwd_reg(&mut self, r: u8) {
        for f in self.fwd_int.iter_mut() {
            if *f == Some(r) {
                *f = None;
            }
        }
        for f in self.fwd_bytes.iter_mut() {
            if let Some((p, l)) = *f {
                if p == r || l == r {
                    *f = None;
                }
            }
        }
    }

    /// Forwarding master switch (diagnostics: `RV64_NO_FWD=1` disables the
    /// store-to-load forwarding while keeping every other path identical).
    fn fwd_slot_ok(&self, slot: usize) -> bool {
        let only = FWD_ONLY_SLOT.load(std::sync::atomic::Ordering::Relaxed);
        only == usize::MAX || only == slot
    }

    fn fwd_enabled(&self) -> bool {
        if !FWD_ENV_READ.swap(true, std::sync::atomic::Ordering::Relaxed) {
            if diag_env_enabled("RV64_NO_FWD") {
                FWD_DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        !FWD_DISABLED.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Join point / branch target: every path merging here can carry a
    /// different register history — forwarding state dies wholesale.
    fn place(&mut self, l: usize) {
        self.fwd_int.fill(None);
        self.fwd_bytes.fill(None);
        self.asm.place(l);
    }

    /// Is the store to `slot` at the current statement dead (no later read
    /// of the slot exists anywhere in the remaining timeline)? Loop-carried
    /// slots are exempt: the backedge re-reads writes the linear timeline
    /// cannot see.
    fn store_is_dead(&self, slot: u8) -> bool {
        if diag_env_enabled("RV64_NO_DSE") {
            return false;
        }
        let s = slot as usize;
        let i = self.web_idx;
        if self.loop_carried.get(s).copied().unwrap_or(false) {
            return false;
        }
        // linear liveness: no read anywhere after the store
        if i < *self.last_read.get(s).unwrap_or(&0) {
            return false;
        }
        // backedge liveness: a read BEFORE the store in linear order, but
        // inside a while span that also contains the store, re-executes
        // after it on the next iteration — the store is live. BUG-022
        // (P2-C4 S6, 10^4 bombardment): the same holds for a read AT the
        // store's own statement index — the self-read of a read-modify-
        // write (`c9 *= X`, `c9 = c9 * X`) evaluates before this
        // iteration's store but re-executes AFTER it on the next pass, so
        // the store feeds it and must not be elided. The old half-open
        // window [a, i) missed exactly that shape: a slot whose ONLY read
        // is its own statement's RHS had its loop-body store dropped, the
        // multiply recomputed from the stale slot value every iteration
        // (overflow never fired), and Im/Imc returned Ok where the
        // interpreter and ImcRemap (banked, never DSE'd) trapped 90.
        let reads = self.read_idx.get(s).map(|v| v.as_slice()).unwrap_or(&[]);
        for &(a, b) in &self.while_spans {
            if a <= i && i <= b {
                // any read in [a, i] — inclusive of the store's own index
                let lo = reads.partition_point(|&r| r < a);
                let hi = reads.partition_point(|&r| r <= i);
                if lo < hi {
                    return false;
                }
            }
        }
        true
    }

    /* ---- BUG-012 Nil guards ---- */

    /// May the value of `node` be Nil at this point (pre-pass verdict)?
    fn nil_val(&self, node: &dyn IRNode) -> bool {
        self.nil_occ.get(&node_addr(node)).map(|o| o.val).unwrap_or(false)
    }

    /// Lazily create (or find) the fault thunk for a Nil-guard exit code.
    fn nil_thunk(&mut self, code: u64) -> usize {
        if let Some(&(l, _)) = self.nil_thunks.iter().find(|&(_, c)| *c == code) {
            return l;
        }
        let l = self.asm.new_label();
        self.nil_thunks.push((l, code));
        l
    }

    /* ---- P3-PAIRPKG: the remove+insert pair --------------------------- *
     * plan()'s sequence walk admitted the pair and marked Root on the     *
     * REMOVE statement / Skip on the INSERT sibling. The fused emission   *
     * (doc 98 §Method, lemmas P1..P4): eval the index ONCE, guard the       *
     * extract faces (Nil → 72, ≥ 2³² → 72), guard the remove bound       *
     * (idx ≥ len → 85 — BEFORE the value evaluation, r4d), evaluate the   *
     * value, then one `sd base+idx*8`. The length shadow is untouched     *
     * (−1 + 1 net zero) and the 81/40 faces are structurally empty.       */

    /// The INSERT sibling of a pair root at `subs[i]`, or None. Defensive:
    /// a Root mark without the marked sibling right after it is an
    /// internal inconsistency (plan and the lowering walk the same
    /// sequence order).
    fn pair_sibling<'s>(&self, subs: &'s [Box<dyn IRNode>], i: usize) -> Option<&'s dyn IRNode> {
        let root = fuse_unwrap(&*subs[i]);
        if !self.pair_root.contains_key(&node_addr(root)) {
            return None;
        }
        if i + 1 >= subs.len() {
            return None;
        }
        let skip = fuse_unwrap(&*subs[i + 1]);
        if !self.pair_skip.contains(&node_addr(skip)) {
            return None;
        }
        Some(&*subs[i + 1])
    }

    /// The fused remove+insert pair: one guarded store into the frame
    /// block. `rm` = the REMOVE statement root, `ins` = the subsumed
    /// INSERT sibling.
    fn emit_pair(&mut self, rm: &dyn IRNode, ins: &dyn IRNode) -> Ret<()> {
        let ru = fuse_unwrap(rm);
        let Some(&rec) = self.pair_root.get(&node_addr(ru)) else {
            return errf!(
                "rv64 lower: opcode REMOVE is outside the translatable kernel subset \
                 (internal: pair emission on an unmarked root)"
            );
        };
        let Some(rn) = ru.as_any().downcast_ref::<IRNodeDouble>() else {
            return errf!("rv64 lower: pair root node shape unexpected");
        };
        let insu = fuse_unwrap(ins);
        let Some(nn) = insu.as_any().downcast_ref::<IRNodeTriple>() else {
            return errf!("rv64 lower: pair sibling node shape unexpected");
        };
        let s = rec.slot;
        if !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
            return errf!(
                "rv64 lower: opcode REMOVE is outside the translatable kernel subset \
                 (internal: pair on a non-growable storage class)"
            );
        }
        // may-Nil value: the interpreter ADMITS a Nil store (check_scalar
        // passes — canbe.rs is_scalar_value contains Nil) and only faults
        // at a later read of the hole (doc 98 §Method, key finding). The u64
        // frame block cannot represent a stored Nil → compile reject
        // (B-family fail-fast; may-Nil PACKLIST element precedent).
        if self.nil_val(&*nn.subz) {
            return errf!(
                "rv64 lower: opcode REMOVE is outside the translatable kernel subset \
                 (the pair value may be Nil: the interpreter stores a Nil element \
                 and faults only when that element is later read, which a u64 frame \
                 block cannot represent — compute the value from definite assignments)"
            );
        }
        let cell = self.slot_offset(s as u8);
        let l72 = self.nil_thunk(72);
        let l85 = self.nil_thunk(85);
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        match rec.const_idx {
            None => {
                // variable index: evaluated ONCE (lemma P3 — the index
                // expressions are structurally equal and receiver-free, so
                // the interpreter's second evaluation is unobservable)
                self.eval_int(&*rn.suby)?;
                let kd = self.vstack.len() - 1;
                if self.nil_val(&*rn.suby) {
                    self.nil_guard_tag(kd, 72);
                }
                let kr = loc_reg(self.vstack[kd]);
                self.asm.li(sc1, 1u64 << 32);
                self.asm.bgeu(kr, sc1, l72); // extract_u32: ≥ 2³² → 72
                self.asm.ld(sc1, S0, cell); // len shadow
                self.asm.bgeu(kr, sc1, l85); // remove bound: idx ≥ len → 85
                // the value evaluation runs AFTER the 85 guard: the
                // interpreter's remove traps before the insert arguments
                // are evaluated (r4d — fault-injection pin, doc 98 §Method)
                self.eval_int(&*nn.subz)?;
                let vd = self.vstack.len() - 1;
                let base = self.list_data_off[s];
                debug_assert!(base >= 0, "list data block missing from the frame plan");
                self.asm.slli_(sc2, kr, 3); // idx * 8 (idx < len ≤ 128)
                self.asm.addi(sc1, S0, base);
                self.asm.add(sc1, sc1, sc2);
                let v = loc_reg(self.vstack[vd]);
                self.asm.sd(v, sc1, 0);
                self.pop_vreg(); // value
                self.pop_vreg(); // idx
            }
            Some(k) => {
                // constant index — static faces first (the interpreter
                // faults 72 BEFORE 85; BUG-028's runtime-guard lesson
                // applies to the guarded form: a Growable constant index
                // checks the live length)
                if k >= 1u64 << 32 {
                    self.asm.jal_(ZERO, l72);
                    return Ok(());
                }
                if k >= 128 {
                    // cap invariant (len ≤ 128 at every write point) makes
                    // any k ≥ 128 an unconditional 85 — no load, no offset
                    // arithmetic wrap face
                    self.asm.jal_(ZERO, l85);
                    return Ok(());
                }
                self.asm.li(sc1, k);
                self.asm.ld(sc2, S0, cell); // len shadow (live)
                self.asm.bgeu(sc1, sc2, l85); // idx ≥ len → 85
                self.eval_int(&*nn.subz)?;
                let vd = self.vstack.len() - 1;
                let base = self.list_data_off[s];
                debug_assert!(base >= 0, "list data block missing from the frame plan");
                let off = base + (k as i32) * 8;
                let v = loc_reg(self.vstack[vd]);
                self.asm.sd(v, S0, off); // base + k*8 fits the imm12 (≤ 2040)
                self.pop_vreg(); // value
            }
        }
        Ok(())
    }

    /* ---- P3-V2IMPL: compo-list v2 shape lowering ---------------------- *
     * plan() is the single admit/reject source: every helper below only   *
     * runs on shapes the plan walk already validated (receiver = GET of a *
     * List slot; index/value expressions are u64-typed). Trap codes and   *
     * their ORDER mirror the interpreter exactly:                        *
     *   - itemget: extract_u32 first — key Nil → 72, key ≥ 2³² → 72       *
     *     (value/convert.rs), then idx ≥ len → CompoNoFindItem(85)        *
     *     (value/list.rs:36-40);                                         *
     *   - append: value Nil → CastBeValueFail(76) (check_scalar), then    *
     *     len ≥ 128 → OutOfCompoLen(40) (compo.rs:160-172);              *
     *   - packlist creation: element expressions fault in IR order; the  *
     *     deferred check_scalar pass cannot fire (may-Nil elements are   *
     *     compile-rejected, the B-family fail-fast precedent);           *
     *   - length/haskey fold (u64 width — the pinned D5/D7 family).      */

    /// The list slot index of an admitted consumer's receiver (defensive
    /// re-validation; plan() already guaranteed the shape).
    fn list_slot_of(&self, node: &dyn IRNode, op: Bytecode) -> Ret<usize> {
        list_receiver_slot(node, self.slot, op)
    }

    /// The pool label of a list's rodata table; `must_register` registers
    /// it on first (creation) use.
    fn list_rodata_lbl(&mut self, s: usize) -> Ret<usize> {
        if self.list_lbl.get(s).copied().unwrap_or(usize::MAX) != usize::MAX {
            return Ok(self.list_lbl[s]);
        }
        let vals = self.lists.lits.get(s).cloned().unwrap_or_default();
        // P3-KERNELS M2-d: values that all fit u32 pack as a 4-byte-stride
        // LE table (deterministic rule; 256B saved on the sha256 K table).
        // The stride predicate is recomputed at the load sites from the
        // same lits, so packing and loading cannot disagree.
        let u32p = list_lits_fit_u32(&vals);
        let mut bytes = Vec::with_capacity(vals.len() * if u32p { 4 } else { 8 });
        for v in &vals {
            if u32p {
                bytes.extend_from_slice(&(*v as u32).to_le_bytes());
            } else {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let lbl = self.lit_label(&bytes);
        self.list_lbl[s] = lbl;
        Ok(lbl)
    }

    /// Whether this slot's Rodata table is u32-packed (see
    /// `list_rodata_lbl`; the empty table is irrelevant but answers false).
    fn list_rodata_u32(&self, s: usize) -> bool {
        self.lists
            .lits
            .get(s)
            .map(|v| list_lits_fit_u32(v))
            .unwrap_or(false)
    }

    /// Emit `idx ≥ 2³² → 72-thunk` (the extract_u32 boundary) and
    /// `idx ≥ len → 85-thunk`; `idx_reg` holds the variable index.
    fn emit_list_bounds_guards(
        &mut self,
        idx_reg: u8,
        s: usize,
        static_len: Option<u64>,
    ) -> Ret<()> {
        let l72 = self.nil_thunk(72);
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, 1u64 << 32);
        self.asm.bgeu(idx_reg, sc1, l72);
        let l85 = self.nil_thunk(85);
        match static_len {
            Some(0) => {
                // empty list: every index is out of bounds
                self.asm.jal_(ZERO, l85);
            }
            Some(n) => {
                self.asm.addi(sc1, ZERO, n as i32);
                self.asm.bgeu(idx_reg, sc1, l85);
            }
            None => {
                let cell = self.slot_offset(s as u8);
                self.asm.ld(sc1, S0, cell);
                self.asm.bgeu(idx_reg, sc1, l85);
            }
        }
        Ok(())
    }

    /// Load list element `idx_reg` into `dst` in place (base materialized
    /// in sc1, scaled index in sc2; block offsets are < 2040 and the
    /// scaled index < 1024 by the 128-item cap).
    fn emit_list_load(&mut self, dst: u8, idx_reg: u8, s: usize, rodata: Option<usize>) -> Ret<()> {
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        match rodata {
            Some(lbl) => {
                self.lit_addr(sc1, lbl);
            }
            None => {
                let base = self.list_data_off[s];
                debug_assert!(base >= 0, "list data block missing from the frame plan");
                self.asm.addi(sc1, S0, base);
            }
        }
        // u32-packed rodata tables: 4-byte stride + lwu (zero-extend keeps
        // the loaded value identical to the u64 form)
        let u32p = rodata.is_some() && self.list_rodata_u32(s);
        self.asm.slli_(sc2, idx_reg, if u32p { 2 } else { 3 });
        self.asm.add(sc1, sc1, sc2);
        if u32p {
            self.asm.lwu(dst, sc1, 0);
        } else {
            self.asm.ld(dst, sc1, 0);
        }
        Ok(())
    }

    /// The (single) list-creation PUT: fill the frame block, or register
    /// the rodata table, or nothing at all (empty static lists).
    fn emit_list_create(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        let s = slot_idx as usize;
        let kind = self
            .lists
            .kind
            .get(s)
            .copied()
            .ok_or("rv64 lower: list slot without a storage plan")?;
        match list_create_src(src) {
            Some(ListCreate::NewList) => {
                match kind {
                    ListKind::Empty => Ok(()),
                    ListKind::Growable { .. } => {
                        // len shadow ← 0 (always an explicit store: the
                        // slot cell is the only runtime length state)
                        let cell = self.slot_offset(slot_idx);
                        self.asm.sd(ZERO, S0, cell);
                        Ok(())
                    }
                    _ => errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (internal: NEWLIST source with storage class {kind:?})"
                    ),
                }
            }
            Some(ListCreate::PackList { arr, n }) => {
                match kind {
                    ListKind::Rodata { .. } => {
                        // Read-only table: the literals cannot fault, so
                        // the creation site emits nothing at all.
                        self.list_rodata_lbl(s)?;
                        Ok(())
                    }
                    ListKind::FrameStatic { .. } | ListKind::Growable { .. } => {
                        if let ListKind::Growable { init_n } = kind {
                            if init_n != n {
                                return errf!(
                                    "rv64 lower: opcode PUT is outside the translatable kernel \
                                     subset (internal: PACKLIST count {n} vs plan {init_n})"
                                );
                            }
                        }
                        let base = self.list_data_off[s];
                        debug_assert!(base >= 0, "list data block missing from the frame plan");
                        for (i, e) in arr.subs[..n as usize].iter().enumerate() {
                            // may-Nil elements are a B-family compile reject:
                            // the interpreter's deferred check_scalar(76)
                            // cannot be ordered faithfully without keeping
                            // every element's tag live past the pool limit
                            if self.nil_val(&**e) {
                                return errf!(
                                    "rv64 lower: opcode PACKLIST is outside the translatable \
                                     kernel subset (element {i} may be Nil; bind list elements \
                                     unconditionally first)"
                                );
                            }
                            self.eval_int(&**e)?;
                            let v = self.pop_reg();
                            self.asm.sd(v, S0, base + (i as i32) * 8);
                        }
                        if let ListKind::Growable { .. } = kind {
                            let cell = self.slot_offset(slot_idx);
                            let sc1 = self.asm.sc1;
                            self.asm.addi(sc1, ZERO, n as i32);
                            self.asm.sd(sc1, S0, cell);
                        }
                        Ok(())
                    }
                    _ => errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (internal: PACKLIST source with storage class {kind:?})"
                    ),
                }
            }
            None => errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (unsupported list creation shape)"
            ),
        }
    }

    /// Fault when the tag byte of the value at vstack position `depth`
    /// says Nil.
    fn nil_guard_tag(&mut self, depth: usize, code: u64) {
        let t = self.nil_thunk(code);
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.tag_base + depth as i32);
        self.asm.beq(sc, ZERO, t);
    }

    /// Fault when slot `s`'s init flag says unassigned (XOP old value,
    /// fused comparisons reading the slot directly).
    fn nil_guard_flag(&mut self, s: u8, code: u64) {
        let t = self.nil_thunk(code);
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.flag_off[s as usize]);
        self.asm.beq(sc, ZERO, t);
    }

    /// EQ/NEQ semantics: fault 90 iff EXACTLY one side is Nil (two Nils
    /// compare equal on the interpreter) — the tags differ.
    fn nil_guard_eq_tags(&mut self, dx: usize, dy: usize) {
        let t = self.nil_thunk(90);
        let (s1, s2) = (self.asm.sc1, self.asm.sc2);
        self.asm.lbu(s1, S0, self.tag_base + dx as i32);
        self.asm.lbu(s2, S0, self.tag_base + dy as i32);
        self.asm.bne(s1, s2, t);
    }

    /// GET of a tracked may-Nil slot: publish the slot's Nil bit as the
    /// value's tag at vstack position `depth`.
    fn nil_wtag_from_flag(&mut self, s: u8, depth: usize) {
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.flag_off[s as usize]);
        self.asm.sb(sc, S0, self.tag_base + depth as i32);
    }

    /// Copy a tag byte between vstack positions (CHOOSE select).
    fn nil_copy_tag(&mut self, from: usize, to: usize) {
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.tag_base + from as i32);
        self.asm.sb(sc, S0, self.tag_base + to as i32);
    }

    /// Publish a proven non-Nil tag byte (1) at vstack position `depth`
    /// (CHOOSE select of a mechanism-less arm, BUG-019: the arm's
    /// evaluation writes no tag bytes, and the stale byte at the result
    /// position — prologue zero reads as "Nil" — must not survive into a
    /// position the consumer's guard will read).
    fn nil_set_tag_one(&mut self, depth: usize) {
        let sc = self.asm.sc1;
        self.asm.addi(sc, ZERO, 1);
        self.asm.sb(sc, S0, self.tag_base + depth as i32);
    }

    /// PUT into a tracked slot from a may-Nil source: the slot's flag
    /// copies the source value's tag (Nil-ness propagates through
    /// assignment, exactly like the interpreter's slot stores).
    fn nil_set_flag_from_tag(&mut self, s: u8, depth: usize) {
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.tag_base + depth as i32);
        self.asm.sb(sc, S0, self.flag_off[s as usize]);
    }

    /// PUT into a tracked slot from a provably non-Nil source.
    fn nil_set_flag_one(&mut self, s: u8) {
        let sc = self.asm.sc1;
        self.asm.addi(sc, ZERO, 1);
        self.asm.sb(sc, S0, self.flag_off[s as usize]);
    }

    /// Consumer guards for the int binary arm / branch comparison arm:
    /// EQ/NEQ fault on exactly-one-Nil (90); ordered comparisons fault on
    /// any Nil (90); arithmetic faults on any Nil (71 — the interpreter's
    /// to_uint runs before the op). Called with both operands evaluated,
    /// before any pops: x sits at `len-2`, y at `len-1`.
    fn nil_guard_int_pair(&mut self, op: Bytecode, lx: bool, ly: bool) {
        use Bytecode::*;
        let n = self.vstack.len();
        match op {
            EQ | NEQ if lx && ly => self.nil_guard_eq_tags(n - 2, n - 1),
            EQ | NEQ if lx => self.nil_guard_tag(n - 2, 90),
            EQ | NEQ if ly => self.nil_guard_tag(n - 1, 90),
            LT | GT | LE | GE => {
                if lx {
                    self.nil_guard_tag(n - 2, 90);
                }
                if ly {
                    self.nil_guard_tag(n - 1, 90);
                }
            }
            _ => {
                let code = if matches!(op, EQ | NEQ | LT | GT | LE | GE) { 90 } else { 71 };
                if lx {
                    self.nil_guard_tag(n - 2, code);
                }
                if ly {
                    self.nil_guard_tag(n - 1, code);
                }
            }
        }
    }

    /// Push a reference to a bank register (a promoted local read in
    /// operand position — no materialization at all).
    fn push_ref(&mut self, r: u8) {
        self.max_depth = self.max_depth.max(self.vstack.len() + 1);
        self.span_open_at(self.vstack.len());
        self.vstack.push(VLoc::Ref(r));
    }

    fn pop_vreg(&mut self) -> VLoc {
        let old_len = self.vstack.len();
        match self.vstack.pop() {
            Some(l) => {
                self.span_close_from(old_len - 1, old_len);
                l
            }
            None => {
                // IR shape bug (pop without push); reject the output.
                self.depth_err = true;
                VLoc::Pool(self.vregs[0])
            }
        }
    }

    /// Pop an operand that is only READ by its consumer (bank refs pass
    /// through untouched — zero traffic).
    fn pop_reg(&mut self) -> u8 {
        loc_reg(self.pop_vreg())
    }

    /// The in-place-update pattern (NOT/casts/POW): pop the operand, return
    /// its owned pool register WITH the value already pushed back on top —
    /// the consumer overwrites that register in place.
    fn pop_own_replace(&mut self) -> u8 {
        match self.pop_vreg() {
            VLoc::Pool(r) => {
                self.span_open_at(self.vstack.len());
                self.vstack.push(VLoc::Pool(r));
                r
            }
            VLoc::Ref(r) => {
                let t = self.push_vreg();
                // a live fwd entry can name this very pool register (it was
                // killed above, but the VALUE still matches): self-moves
                // are free
                if t != r {
                    self.asm.mv(t, r);
                }
                t
            }
        }
    }

    /// Pop the top two operands of a binary op and return
    /// `(d, ra_read, rb_read)` where `d` is the destination pool register
    /// already holding the left value: the left operand's pool register
    /// reused when it was Pool, or a fresh register loaded with `mv` when
    /// it was a bank reference.
    fn pop2_dest(&mut self) -> (u8, u8, u8) {
        let rb = self.pop_reg();
        let la = self.pop_vreg();
        let d = self.push_vreg();
        match la {
            VLoc::Pool(r) => {
                debug_assert_eq!(d, r);
                (d, r, rb)
            }
            VLoc::Ref(r) => {
                // d can BE r when the ref names the pool register the
                // shrunk stack just re-handed (forwarded GET): the value is
                // already in place
                if d != r {
                    self.asm.mv(d, r);
                }
                (d, r, rb)
            }
        }
    }

    /* ---- misc ---- */

    fn slot_offset(&self, idx: u8) -> i32 {
        let off = self.frame_off.get(idx as usize).copied().unwrap_or(-1);
        assert!(off >= 0, "rv64 lower: local slot {idx} not in the frame plan");
        off
    }

    /// Write-side int home (inclusive at segment boundaries).
    fn bank_int(&self, idx: u8) -> Option<u8> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Int) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, true)?;
        Some(h[0])
    }

    /// Read-side int home (exclusive at segment boundaries).
    fn bank_int_read(&self, idx: u8) -> Option<u8> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Int) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, false)?;
        Some(h[0])
    }

    /// Read-side bytes home.
    fn bank_bytes(&self, idx: u8) -> Option<(u8, u8)> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Bytes) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, false)?;
        Some((h[0], h[1]))
    }

    /// Write-side bytes home.
    fn bank_bytes_write(&self, idx: u8) -> Option<(u8, u8)> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Bytes) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, true)?;
        Some((h[0], h[1]))
    }


    /// Exit-code-1 trap (assert/abort): inline li+jal, rare enough. Sites
    /// with a fixed 90/71 exit code jump straight to the shared thunks.
    fn trap(&mut self, code: u64) {
        self.asm.li(A0, code);
        self.asm.jal_(ZERO, self.trap_label);
    }

    /// Materialize the constant `v` into `rd`. Large constants (≥2048, the
    /// lui+addi pair) come from the read-only pool via the pool-base
    /// register when one is active: one 4-byte `ld` instead of a 6-byte
    /// pair, deduplicated across every re-materialization site.
    fn li_k(&mut self, rd: u8, v: u64) {
        // Only values beyond one lui+addi pair justify the pool entry (an
        // 8-byte pool literal amortized against 2 saved bytes per site is
        // a wash for ≤0xFFFF constants — HIP5's popcount masks lose money
        // pooling).
        if v > 0xFFFF {
            if let Some((b, bl)) = self.pool_base {
                // little-endian: the load reads a raw 64-bit word
                let lbl = self.lit_label(&v.to_le_bytes());
                self.asm.ld_label_off(rd, b, bl, lbl);
                return;
            }
        }
        self.asm.li(rd, v);
    }

    /// Address of a pool literal into `rd`: the shared base register when
    /// available (c.addi), else an auipc+addi pair.
    fn lit_addr(&mut self, rd: u8, lbl: usize) {
        if let Some((b, bl)) = self.pool_base {
            self.asm.addi_label_off(rd, b, bl, lbl);
        } else {
            self.asm.pcrel_addr(rd, lbl);
        }
    }

    fn lit_label(&mut self, bytes: &[u8]) -> usize {
        if let Some(i) = self.pool_map.get(bytes) {
            return self.pool_entries[*i].0;
        }
        let lbl = self.asm.new_label();
        self.pool_map.insert(bytes.to_vec(), self.pool_entries.len());
        self.pool_entries.push((lbl, bytes.to_vec()));
        lbl
    }

    /* ---- top level ---- */

    /// Prologue + body + fallthrough exit. (Thunks + trap epilogue are
    /// appended by `lower_body_with_input`.)
    fn emit_prologue_body_epilogue(&mut self, nodes: &[Box<dyn IRNode>]) -> Ret<()> {
        if self.frame_bytes > 2040 {
            // GT-c: actionable hint (doc 66 §1.4 rated "medium": said "should suggest
                    // reducing locals / compacting" — say how)
                    return errf!(
                        "rv64 lower: locals frame {}B exceeds the addi range; hint: fewer \
                         simultaneously-live locals shrink the frame — reuse one name for \
                         values with disjoint lifetimes",
                        self.frame_bytes
                    );
        }
        // prologue: capture the ABI registers, init the arena and frame base.
        self.asm.mv(S2, 12); // a2 output_base
        self.asm.mv(S3, 13); // a3 output_max
        self.asm.mv(S4, 10); // a0 input_base
        self.asm.mv(S5, 11); // a1 input_len
        self.asm.li(S1, ARENA_BASE);
        self.asm.addi(S0, SP, -(self.frame_bytes as i32));
        if let Some((r, l)) = self.pool_base {
            self.asm.pcrel_addr(r, l);
        }
        // Zero only the slots a read could reach before the first write.
        // Banked (promoted) slots zero their register home instead — a
        // 2-byte c.li against a 4-byte sd of x0.
        // sd x0 never compresses (c.sd needs rs2 in the window): when two
        // or more stores need zeroing, a window register loaded with 0
        // makes each a 2-byte c.sd (LLVM's MakeCompressible trick). The
        // pool is empty here, so its first (window) register is free.
        let nz: Vec<usize> = (0..self.slot.len())
            .filter(|&i| {
                !matches!(self.slot[i], SlotTy::Input)
                    && self.plan.banked.get(i).copied().unwrap_or(BankKind::None) == BankKind::None
                    && self.needs_zero.get(i).copied().unwrap_or(true)
            })
            .collect();
        let zero_stores = nz
            .iter()
            .map(|&i| if matches!(self.slot[i], SlotTy::Bytes) { 2 } else { 1 })
            .sum::<usize>();
        let zreg = if zero_stores >= 2 { Some(self.vregs[0]) } else { None };
        if let Some(r) = zreg {
            self.asm.addi(r, ZERO, 0);
        }
        for (i, s) in self.slot.iter().enumerate() {
            if matches!(s, SlotTy::Input) {
                continue;
            }
            if self.plan.banked.get(i).copied().unwrap_or(BankKind::None) != BankKind::None {
                // a banked slot needs the entry zero only when its first
                // web segment is an init segment (first touch is a read or
                // a conditional write); a write-first slot re-defines its
                // value before any read, so the zero would be dead bytes
                if self.plan.init_zero.get(i).copied().unwrap_or(false) {
                    if let Some(regs) = self.plan.home_at(i, 0, true) {
                        match self.plan.banked[i] {
                            BankKind::Int => self.asm.addi(regs[0], ZERO, 0),
                            BankKind::Bytes => {
                                self.asm.addi(regs[0], ZERO, 0);
                                self.asm.addi(regs[1], ZERO, 0);
                            }
                            BankKind::None => unreachable!(),
                        }
                    }
                }
                continue;
            }
            if self.needs_zero.get(i).copied().unwrap_or(true) {
                let zr = match zreg {
                    Some(z) => z,
                    None => ZERO,
                };
                if let Some(o) = self.frame_off.get(i).filter(|o| **o >= 0) {
                    self.asm.sd(zr, S0, *o);
                    if matches!(s, SlotTy::Bytes) {
                        self.asm.sd(zr, S0, *o + 8);
                    }
                }
            }
        }
        // BUG-012: Nil-init flags start CLEARED (every tracked slot reads
        // as Nil until its first store). The tag stack needs no zeroing —
        // every tag byte is written before any read (write-on-produce /
        // read-on-consume at the same vstack depth). The region is word
        // aligned (see the constructor).
        if self.tag_base >= 0 {
            let end = self.tag_base + 8;
            let mut o = self.nil_flag_start;
            while o < end {
                self.asm.sd(ZERO, S0, o);
                o += 8;
            }
        }
        // body
        self.stmt_list(nodes, false)?;
        if !self.vstack.is_empty() {
            return errf!("rv64 lower: value stack not empty at function end");
        }
        // fallthrough: empty output — dead when the body cannot fall out
        // (ends in return/throw/abort/break/continue)
        let falls_out = nodes
            .last()
            .map(|n| !ends_in_unconditional_jump(&**n))
            .unwrap_or(true);
        if falls_out {
            self.asm.addi(A0, ZERO, 0);
            self.asm.ret();
        }
        Ok(())
    }

    /* ---- statements ---- */

    fn stmt_list(&mut self, nodes: &[Box<dyn IRNode>], keep_last: bool) -> Ret<()> {
        let last = nodes.len().saturating_sub(1);
        let mut i = 0usize;
        while i < nodes.len() {
            // top-level constant-select runs (the array arm covers nested
            // blocks; this covers the function body itself)
            if !keep_last || i != last {
                let run_o0 = self.asm.buf.len();
                if let Some(consumed) = self.try_const_select_run(nodes, i)? {
                    let cnt: usize = nodes[i..i + consumed]
                        .iter()
                        .map(|n| stmt_walk_count(&**n))
                        .sum::<usize>();
                    // one table replaces the whole arm run: attribute its
                    // bytes to the run's walk-index span ([base+1 .. base+cnt])
                    if let Some(v) = self.stmt_trace.as_mut() {
                        v.push(StmtRangeDbg {
                            w_lo: self.web_idx + 1,
                            w_hi: self.web_idx + cnt,
                            off_start: run_o0,
                            off_end: self.asm.buf.len(),
                        });
                    }
                    self.web_idx += cnt;
                    i += consumed;
                    continue;
                }
            }
            let n = &nodes[i];
            // P3-PAIRPKG: a recognized remove+insert pair — the fused
            // overwrite is emitted at the REMOVE root, the INSERT sibling
            // is subsumed (its dispatch is skipped, so the web_idx cursor
            // advances by both statements' counts explicitly).
            if let Some(ins) = self.pair_sibling(nodes, i) {
                self.web_idx += stmt_walk_count(&*nodes[i]) + stmt_walk_count(&*nodes[i + 1]);
                self.emit_pair(&*nodes[i], ins)?;
                i += 2;
                continue;
            }
            let keep = keep_last && i == last;
            self.stmt_one(&**n, keep)?;
            i += 1;
        }
        Ok(())
    }

    /// Lower one statement/expression node. `keep`: a trailing value must
    /// remain on the vstack (expression-position block).
    fn stmt_one(&mut self, node: &dyn IRNode, keep: bool) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.stmt_one(&*w.node, keep);
        }
        // Debug side-channel: attribute the emitted bytes to the statement's
        // walk-index range. Mirrors `WebWalk::walk_stmt` numbering (every
        // statement node counts exactly one idx, consumed by the inner call).
        if self.stmt_trace.is_some() {
            let w0 = self.web_idx;
            let o0 = self.asm.buf.len();
            let r = self.stmt_one_inner(node, keep);
            if r.is_ok() {
                if let Some(v) = self.stmt_trace.as_mut() {
                    v.push(StmtRangeDbg {
                        w_lo: w0 + 1,
                        w_hi: self.web_idx,
                        off_start: o0,
                        off_end: self.asm.buf.len(),
                    });
                }
            }
            return r;
        }
        self.stmt_one_inner(node, keep)
    }

    fn stmt_one_inner(&mut self, node: &dyn IRNode, keep: bool) -> Ret<()> {
        // statement-index cursor: +1 per statement node, mirroring
        // `WebWalk::walk_stmt` exactly (WRAP transparent, Empty counted)
        self.web_idx += 1;
        if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
            || node.as_any().downcast_ref::<IRNodeText>().is_some()
        {
            return Ok(());
        }
        // P3-IDIOMS: a chain-continuation PUT whose value is subsumed by
        // the fused wide load emitted at the chain head (the recognition
        // refuses keep-tail chains, so a skipped statement is never the
        // kept value; the PUT statement form has no value of its own).
        if let Some(FuseMark::Skip) = self.fuse.get(&node_addr(fuse_unwrap(node))) {
            return Ok(());
        }
        // P3-PAIRPKG: a recognized pair's INSERT is fully emitted at the
        // REMOVE root (one guarded sd). This early return happens after the
        // web_idx increment so the statement numbering stays aligned with
        // WebWalk::walk_stmt (plan refuses statement forms inside the pair
        // subtrees, so no deeper counts are skipped).
        if self.pair_skip.contains(&node_addr(fuse_unwrap(node))) {
            return Ok(());
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            IRLIST | IRBLOCK | IRBLOCKR => {
                let arr = node
                    .as_any()
                    .downcast_ref::<IRNodeArray>()
                    .ok_or("rv64 lower: block node shape unexpected")?;
                let keep_last = keep && matches!(op, IRBLOCKR | IRLIST);
                let last = arr.subs.len().saturating_sub(1);
                let mut i = 0usize;
                while i < arr.subs.len() {
                    // statement-position runs of `if x == K { slot = V }`
                    // collapse into one table lookup (arms are statements:
                    // no value-position interaction; a kept-last value is
                    // never an IRIF, so the run never swallows it)
                    if !keep_last || i != last {
                        if let Some(consumed) = self.try_const_select_run(&arr.subs, i)? {
                            // the caller already counted sub[i]'s own idx;
                            // skip the rest of the swallowed subtree counts
                            self.web_idx += arr.subs[i..i + consumed]
                                .iter()
                                .map(|n| stmt_walk_count(&**n))
                                .sum::<usize>();
                            i += consumed;
                            continue;
                        }
                    }
                    let n = &arr.subs[i];
                    // P3-PAIRPKG: a recognized pair inside the block — same
                    // sequence-level consumption as the body loop (the
                    // drop check below is bypassed: the fused emission is
                    // vstack-balanced, and plan refuses kept-tail pairs).
                    if let Some(ins) = self.pair_sibling(&arr.subs, i) {
                        self.web_idx += stmt_walk_count(&*arr.subs[i]) + stmt_walk_count(&*arr.subs[i + 1]);
                        self.emit_pair(&*arr.subs[i], ins)?;
                        i += 2;
                        continue;
                    }
                    let k = keep_last && i == last && n.hasretval();
                    self.stmt_one(&**n, k)?;
                    if n.hasretval() && !k {
                        let ty = node_res_ty(&**n, self.slot)?;
                        match ty {
                            Ty::Int => {
                                self.pop_vreg();
                            }
                            Ty::Bytes => {
                                self.pop_vreg();
                                self.pop_vreg();
                            }
                            // defensive: plan() rejects list-valued tails
                            Ty::List => {}
                        }
                    }
                    i += 1;
                }
                Ok(())
            }
            IRIF => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: if node shape unexpected")?;
                // `if <cmp> { slot = <operand> }` fuses to max/min
                if self.try_maxmin_update(node)? {
                    // the arm + else subtree mirror-counts were consumed
                    self.web_idx += stmt_walk_count(&*n.suby) + stmt_walk_count(&*n.subz);
                    return Ok(());
                }
                // An else arm that emits no bytes makes the then-arm's
                // jump-to-end dead; so does a then arm that cannot fall
                // through (return/throw/break/continue).
                let skip_join =
                    is_zero_byte_stmt(&*n.subz) || ends_in_unconditional_jump(&*n.suby);
                let l_else = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_else)?;
                self.stmt_one(&*n.suby, false)?;
                if !skip_join {
                    self.asm.jal_(ZERO, l_end);
                }
                self.place(l_else);
                self.stmt_one(&*n.subz, false)?;
                self.place(l_end);
                Ok(())
            }
            IRIFR => {
                // statement-position value-if: branch-lazy like the
                // bytecode channel (compile_if). The caller drops the
                // kept value. CHOOSE (ternary/choose()) no longer lands
                // here: it must evaluate ALL THREE operands eagerly (the
                // bytecode channel has no lazy path for it), so it falls
                // to the eval-and-drop default arm below.
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                let l_else = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_else)?;
                self.eval(&*n.suby)?;
                self.asm.jal_(ZERO, l_end);
                self.place(l_else);
                self.eval(&*n.subz)?;
                self.place(l_end);
                Ok(())
            }
            IRWHILE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: while node shape unexpected")?;
                if !self.vstack.is_empty() {
                    return errf!("rv64 lower: while loop in expression position is not supported");
                }
                let l_cond = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.place(l_cond);
                self.branch_cond(&*n.subx, false, l_end)?;
                self.break_labels.push(l_end);
                self.continue_labels.push(l_cond);
                self.stmt_one(&*n.suby, false)?;
                self.break_labels.pop();
                self.continue_labels.pop();
                if !self.vstack.is_empty() {
                    return errf!("rv64 lower: while body leaves values on the stack");
                }
                self.asm.jal_(ZERO, l_cond);
                self.place(l_end);
                Ok(())
            }
            IRBREAK => match self.break_labels.last() {
                Some(&l) => {
                    self.asm.jal_(ZERO, l);
                    Ok(())
                }
                None => errf!("rv64 lower: break outside a loop"),
            },
            IRCONTINUE => match self.continue_labels.last() {
                Some(&l) => {
                    self.asm.jal_(ZERO, l);
                    Ok(())
                }
                None => errf!("rv64 lower: continue outside a loop"),
            },
            REQ => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: require node shape unexpected")?;
                let mev = self.nil_val(&*n.subx);
                self.eval_int(&*n.subx)?; // error value
                let ok = self.asm.new_label();
                self.branch_cond(&*n.suby, true, ok)?;
                let ev = self.pop_reg();
                self.asm.mv(A0, ev);
                self.asm.jal_(ZERO, self.trap_label);
                self.place(ok);
                // validate_error_scalar runs on the interpreter AFTER the
                // condition: a Nil error value faults CastBeValueFail(76)
                // whether the condition passes or not. The value's tag sits
                // at the entry depth (it was popped; the tag byte stays).
                if mev {
                    self.nil_guard_tag(self.vstack.len(), 76);
                }
                Ok(())
            }
            AST => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: assert node shape unexpected")?;
                let ok = self.asm.new_label();
                self.branch_cond(&*n.subx, true, ok)?;
                self.trap(1);
                self.place(ok);
                Ok(())
            }
            ERR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: throw node shape unexpected")?;
                let mev = self.nil_val(&*n.subx);
                self.eval_int(&*n.subx)?;
                if mev {
                    // throw <nil> → validate_error_scalar → 76
                    self.nil_guard_tag(self.vstack.len() - 1, 76);
                }
                let ev = self.pop_reg();
                self.asm.mv(A0, ev);
                self.asm.jal_(ZERO, self.trap_label);
                Ok(())
            }
            ABT => {
                self.trap(1);
                Ok(())
            }
            RET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: return node shape unexpected")?;
                self.eval_bytes(&*n.subx)?;
                self.emit_return()?;
                Ok(())
            }
            END => {
                self.asm.addi(A0, ZERO, 0);
                self.asm.ret();
                Ok(())
            }
            NOP => Ok(()),
            // The surface compiler opens the local window and balances the
            // call frame with these; the proto pre-allocates a typed frame.
            ALLOC => Ok(()),
            POP => {
                if !self.vstack.is_empty() {
                    self.pop_vreg();
                }
                Ok(())
            }
            /* ---- P3-V2IMPL: the list mutation statements ---- */
            APPEND => {
                if keep {
                    return errf!(
                        "rv64 lower: opcode APPEND is outside the translatable kernel subset \
                         (the append result is a list value)"
                    );
                }
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: append node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                if !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
                    return errf!(
                        "rv64 lower: opcode APPEND is outside the translatable kernel subset \
                         (internal: append on a non-growable storage class)"
                    );
                }
                // BUG-029 (P3-PAIRPKG FIXUP): a may-Nil appended value is
                // compile-REJECTED. The old emission guarded it with a
                // 76-thunk — but the interpreter does NOT fault at the
                // mutation: check_scalar ADMITS Nil (canbe.rs
                // is_scalar_value contains Nil), the Nil element is
                // silently STORED, and only a LATER read of the hole
                // faults (CastFail 71 — probed, doc 98 §E4). The 76 trap
                // therefore fired exactly where the interpreter continued:
                // a code-and-timing divergence (six reproducing shapes,
                // e.g. `if q == 1u64 { var n = 5u64 } … append(l, n)`
                // outside the branch → interp Ok(length 1), rv64 Err(76)
                // in all three modes). Faithful storage would need
                // per-element Nil tags in the frame block — feature-level,
                // ruled out; the B-family fail-fast (may-Nil PACKLIST
                // element, may-Nil pair value precedents) is the fix.
                if self.nil_val(&*n.suby) {
                    return errf!(
                        "rv64 lower: opcode APPEND is outside the translatable kernel subset \
                         (the appended value may be Nil: the interpreter stores a Nil element \
                         and faults only when that element is later read, which a u64 frame \
                         block cannot represent — compute the value from definite assignments)"
                    );
                }
                // interpreter order (execute.rs APPEND → compo.rs append):
                // value faults during its evaluation, then the 128 cap
                // (40), then the store — a definite value has no further
                // fault face between its evaluation and the store
                // (check_scalar admits every scalar including Nil, and a
                // rejected may-Nil value never reaches this point)
                self.eval_int(&*n.suby)?;
                let vd = self.vstack.len() - 1;
                let cell = self.slot_offset(s as u8);
                let base = self.list_data_off[s];
                debug_assert!(base >= 0, "list data block missing from the frame plan");
                let sc1 = self.asm.sc1;
                let sc2 = self.asm.sc2;
                self.asm.ld(sc1, S0, cell); // len
                self.asm.slli_(sc2, sc1, 3); // len*8 (0..1016 iff len<128)
                self.asm.addi(sc1, ZERO, 1024);
                let l40 = self.nil_thunk(40);
                self.asm.bgeu(sc2, sc1, l40); // len ≥ 128 ⟺ len*8 ≥ 1024
                self.asm.addi(sc1, S0, base);
                self.asm.add(sc1, sc1, sc2); // addr = base + len*8
                let v = loc_reg(self.vstack[vd]);
                self.asm.sd(v, sc1, 0);
                self.asm.srli_(sc2, sc2, 3); // len back
                self.asm.addi(sc2, sc2, 1); // len + 1
                self.asm.sd(sc2, S0, cell);
                self.pop_vreg();
                Ok(())
            }
            CLEAR => {
                if keep {
                    return errf!(
                        "rv64 lower: opcode CLEAR is outside the translatable kernel subset \
                         (the clear result is a list value)"
                    );
                }
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: clear node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                // clear forces the growable class in plan(); the frame
                // contents stay — later appends overwrite (compo.rs:153-158)
                if !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
                    return errf!(
                        "rv64 lower: opcode CLEAR is outside the translatable kernel subset \
                         (internal: clear on a non-growable storage class)"
                    );
                }
                let cell = self.slot_offset(s as u8);
                self.asm.sd(ZERO, S0, cell);
                Ok(())
            }
            _ => {
                // value-position node in statement position: eval and drop
                self.eval(node)?;
                if node.hasretval() {
                    let ty = node_res_ty(node, self.slot)?;
                    match ty {
                        Ty::Int => {
                            self.pop_vreg();
                        }
                        Ty::Bytes => {
                            self.pop_vreg();
                            self.pop_vreg();
                        }
                        // defensive: plan() rejects list-valued statements
                        Ty::List => {}
                    }
                }
                Ok(())
            }
        }
    }

    /* ---- expressions ---- */

    /// Pop `vals` arm-result locations (LIFO) and return them bottom-first,
    /// with every Ref materialized into ITS OWN consecutive pool position
    /// (the `mv` lands inside the arm's own region). The stack stays at the
    /// entry level: the caller re-pushes the uniform locations once BOTH
    /// arms agree. Re-pushing position-by-position (not push/pop per value)
    /// is what keeps a bytes pair off the same register: with the old
    /// push/pop-per-value form the len materialization re-handed the PTR's
    /// register and clobbered it (BUG-011). A Pool value is already in its
    /// position's register — the arm pushed it there — so only a Ref moves.
    fn take_arm_values(&mut self, vals: usize) -> Vec<VLoc> {
        let mut popped: Vec<VLoc> = (0..vals).map(|_| self.pop_vreg()).collect();
        popped.reverse(); // bottom-first: mirror the push order
        let mut out = Vec::with_capacity(vals);
        for l in popped.drain(..) {
            let t = self.push_vreg(); // hands vregs[entry + i]
            if let VLoc::Ref(r) = l {
                if t != r {
                    self.asm.mv(t, r);
                }
            }
            out.push(VLoc::Pool(t));
        }
        for _ in 0..vals {
            self.pop_vreg();
        }
        out
    }

    fn eval(&mut self, node: &dyn IRNode) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.eval(&*w.node);
        }
        // P3-IDIOMS: a marked byte-assembly group root replaces the whole
        // subtree's lowering with one span-guarded wide load (doc 94).
        if let Some(FuseMark::Root { form, .. }) = self.fuse.get(&node_addr(node)) {
            return self.emit_fused_root(node, *form);
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            /* ---- literals ---- */
            P0 | P1 | P2 | P3 => {
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, (op as u8).wrapping_sub(P0 as u8) as i32);
            }
            PU8 => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1>()
                    .ok_or("rv64 lower: PU8 node shape unexpected")?;
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, p.para as i32);
            }
            PU16 => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam2>()
                    .ok_or("rv64 lower: PU16 node shape unexpected")?;
                let v = u16::from_be_bytes(p.para) as u64;
                let r = self.push_vreg();
                self.li_k(r, v);
            }
            PBUF | PBUFL => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParams>()
                    .ok_or("rv64 lower: PBUF node shape unexpected")?;
                let data = &p.para[if op == PBUF { 1 } else { 2 }..];
                let lbl = self.lit_label(data);
                let rp = self.push_vreg();
                let rl = self.push_vreg();
                self.lit_addr(rp, lbl);
                self.asm.li(rl, data.len() as u64);
            }
            PNBUF => {
                let rp = self.push_vreg();
                let rl = self.push_vreg();
                self.asm.mv(rp, ZERO);
                self.asm.addi(rl, ZERO, 0);
            }
            PTRUE | PFALSE => {
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, if op == PTRUE { 1 } else { 0 });
            }
            /* ---- casts ---- */
            CU8 | CU16 | CU32 | CU64 => {
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: cast node shape unexpected")?;
                // Int literals arrive as CU32/CU64 wrapping PBUF BE bytes
                // (or PU8/PU16/P0..P3): fold the whole cast node at compile
                // time — ir_literal_value sees through the cast chain.
                let limit: Option<u64> = match op {
                    CU8 => Some(0xFF),
                    CU16 => Some(0xFFFF),
                    CU32 => Some(0xFFFF_FFFF),
                    _ => None, // CU64 takes the full u64 range
                };
                if let Ok(Some(v)) = crate::lang::ir_literal_value(node) {
                    if let Ok(n) = v.extract_u128() {
                        if n <= u64::MAX as u128 {
                            if let Some(lim) = limit {
                                if n > lim as u128 {
                                    return errf!(
                                        "rv64 lower: literal {n} overflows the cast target"
                                    );
                                }
                            }
                            let r = self.push_vreg();
                            self.li_k(r, n as u64);
                            return Ok(());
                        }
                    }
                }
                self.eval_int(&*sub.subx)?;
                if let Some(limit) = limit {
                    let r = self.pop_own_replace();
                    self.li_k(self.a0, limit);
                    // trap iff r > limit (r == limit is IN range: 255 is a
                    // valid u8). The historical form was doubly wrong —
                    // bgeu(r, limit, ok) accepted overflows and trapped
                    // every valid value; nothing e2e-tested a runtime
                    // narrowing cast before the guard-semantics
                    // differential caught it.
                    self.asm.bltu(self.a0, r, self.fault_cast);
                }
            }
            CU128 => {
                // GT-c: actionable hint; the substring "u128 values are not in the
                // kernel subset" stays intact (kernel.rs annotate pass-through
                // test pins the family wording)
                return errf!(
                    "rv64 lower: u128 values are not in the kernel subset; hint: keep \
                     intermediates in u64 — the kernel subset computes 64-bit values"
                );
            }
            CBYTES => {
                // Integer literal → fold straight to a pool literal (8-byte
                // BE u64 view, matching the runtime path below).
                let lit = crate::lang::ir_literal_value(node)
                    .ok()
                    .and_then(|v| v)
                    .and_then(|v| v.extract_u128().ok())
                    .filter(|&n| n <= u64::MAX as u128);
                if let Some(n) = lit {
                    let be = (n as u64).to_be_bytes();
                    let lbl = self.lit_label(&be);
                    let rp = self.push_vreg();
                    let rl = self.push_vreg();
                    self.lit_addr(rp, lbl);
                    self.asm.li(rl, 8);
                } else {
                    let sub = node
                        .as_any()
                        .downcast_ref::<IRNodeSingle>()
                        .ok_or("rv64 lower: cast node shape unexpected")?;
                    let m = self.nil_val(&*sub.subx);
                    self.eval_int(&*sub.subx)?;
                    if m {
                        // `nil as bytes` → CastFail(71) on the interpreter
                        self.nil_guard_tag(self.vstack.len() - 1, 71);
                    }
                    self.emit_cbytes()?;
                }
            }
            /* ---- locals ---- */
            GET | GET0 | GET1 | GET2 | GET3 => {
                let idx = match op {
                    GET => node
                        .as_any()
                        .downcast_ref::<IRNodeParam1>()
                        .ok_or("rv64 lower: GET node shape unexpected")?
                        .para,
                    GET0 => 0,
                    GET1 => 1,
                    GET2 => 2,
                    _ => 3,
                };
                // BUG-012: a may-Nil read publishes the slot's init flag as
                // the value's tag (the consumer faults, never the GET)
                let mnil = self.nil_val(node);
                match self.slot.get(idx as usize) {
                    Some(SlotTy::Input) => {
                        // the kernel's `data: bytes` parameter: (input_base,
                        // len) — the ABI capture registers ARE the slot's
                        // home (read-only for the whole body), so reads are
                        // zero-cost references, exactly like a banked bytes
                        // local
                        self.push_ref(S4);
                        self.push_ref(S5);
                    }
                    // P3-V2IMPL defensive: a bare GET of a list slot never
                    // reaches lowering (plan() rejects every consumer but
                    // item_get/append/clear/length/has_key)
                    Some(SlotTy::List) => {
                        return errf!(
                            "rv64 lower: opcode GET is outside the translatable kernel subset \
                             (a list value cannot be used as a value)"
                        );
                    }
                    _ if self.bank_bytes(idx).is_some() => {
                        // promoted bytes local: two zero-cost references
                        let (rp, rl) = self.bank_bytes(idx).unwrap();
                        self.push_ref(rp);
                        self.push_ref(rl);
                        if mnil {
                            self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                        }
                    }
                    _ if self.bank_int_read(idx).is_some() => {
                        // promoted int local: zero-cost reference
                        self.push_ref(self.bank_int_read(idx).unwrap());
                        if mnil {
                            self.nil_wtag_from_flag(idx, self.vstack.len() - 1);
                        }
                    }
                    Some(SlotTy::Bytes) => {
                        // Forwarding soundness: a Ref may only ever name the
                        // BOTTOM pool register pair, pushed at an empty stack
                        // (position 0). Any later push hands vregs[2..] while
                        // the refs live, so nothing can re-hand — and then
                        // clobber — the watched registers mid-expression.
                        let hit = self
                            .fwd_enabled()
                            .then(|| {
                                if self.fwd_slot_ok(idx as usize) {
                                    self.fwd_bytes.get(idx as usize).copied().flatten()
                                } else {
                                    None
                                }
                            })
                            .flatten();
                        match hit {
                            Some((rp, rl))
                                if self.vstack.is_empty()
                                    && rp == self.vregs[0]
                                    && self.vregs.len() > 1
                                    && rl == self.vregs[1] =>
                            {
                                self.push_ref(rp);
                                self.push_ref(rl);
                                if mnil {
                                    self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                                }
                            }
                            _ => {
                                let off = self.slot_offset(idx);
                                let rp = self.push_vreg();
                                let rl = self.push_vreg();
                                self.asm.ld(rp, S0, off);
                                self.asm.ld(rl, S0, off + 8);
                                if mnil {
                                    self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                                }
                            }
                        }
                    }
                    _ => match self.fwd_int.get(idx as usize).copied().flatten() {
                        // same bottom-register rule (see the bytes arm)
                        Some(r)
                            if self.fwd_enabled()
                                && self.fwd_slot_ok(idx as usize)
                                && self.vstack.is_empty()
                                && r == self.vregs[0] =>
                        {
                            self.push_ref(r);
                            if mnil {
                                self.nil_wtag_from_flag(idx, self.vstack.len() - 1);
                            }
                        }
                        _ => {
                            let off = self.slot_offset(idx);
                            let r = self.push_vreg();
                            self.asm.ld(r, S0, off);
                            if mnil {
                                self.nil_wtag_from_flag(idx, self.vstack.len() - 1);
                            }
                        }
                    },
                }
            }
            PUT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: PUT node shape unexpected")?;
                if matches!(self.slot.get(n.para as usize), Some(SlotTy::Input)) {
                    if !self.param_bound {
                        // prelude binding: the value is stack machinery
                        // (ROLL0); the input registers already hold it.
                        self.param_bound = true;
                        return Ok(());
                    }
                    // GT-c: asserted core substring "input parameter cannot be assigned"
                        // stays contiguous (rv64_gen_corpus.rs P2 message pin);
                        // the hint appends after it
                        return errf!(
                            "rv64 lower: the input parameter cannot be assigned; the call-data \
                             argument is read-only — copy it into a local first \
                             (e.g. `var t = data`) and assign to that"
                        );
                }
                // P3-V2IMPL: the (single) list-creation PUT fills the frame
                // block / registers the rodata table; every other PUT into
                // a List slot was rejected by plan() (re-bind / alias).
                if matches!(self.slot.get(n.para as usize), Some(SlotTy::List)) {
                    return self.emit_list_create(n.para, &*n.subx);
                }
                let ty = node_res_ty(&*n.subx, self.slot)?;
                let msrc = self.nil_val(&*n.subx);
                let stracked =
                    self.tracked.get(n.para as usize).copied().unwrap_or(false);
                if ty == Ty::Bytes {
                    let (bp, bl) = match self.bank_bytes_write(n.para) {
                        Some(x) => x,
                        None => {
                            let off = self.slot_offset(n.para);
                            self.eval_bytes(&*n.subx)?;
                            // the source's tag sits under its len half
                            let tag_d = self.vstack.len() - 2;
                            let rl = self.pop_reg();
                            let rp = self.pop_reg();
                            let s = n.para as usize;
                            if self.store_is_dead(n.para) {
                                self.fwd_bytes[s] = None;
                            } else {
                                self.asm.sd(rp, S0, off);
                                self.asm.sd(rl, S0, off + 8);
                                self.fwd_bytes[s] = Some((rp, rl));
                                if stracked {
                                    if msrc {
                                        self.nil_set_flag_from_tag(n.para, tag_d);
                                    } else {
                                        self.nil_set_flag_one(n.para);
                                    }
                                }
                            }
                            return Ok(());
                        }
                    };
                    // literals bind straight into the bank registers
                    if let Some(b) = bytes_literal(&*n.subx) {
                        let lbl = self.lit_label(&b);
                        self.kill_fwd_reg(bp);
                        self.kill_fwd_reg(bl);
                        self.lit_addr(bp, lbl);
                        self.asm.li(bl, b.len() as u64);
                        if stracked {
                            self.nil_set_flag_one(n.para);
                        }
                        return Ok(());
                    }
                    self.eval_bytes(&*n.subx)?;
                    let tag_d = self.vstack.len() - 2;
                    let ll = self.pop_reg();
                    let lp = self.pop_reg();
                    self.kill_fwd_reg(bp);
                    self.kill_fwd_reg(bl);
                    if lp != bp {
                        self.asm.mv(bp, lp);
                    }
                    if ll != bl {
                        self.asm.mv(bl, ll);
                    }
                    if stracked {
                        if msrc {
                            self.nil_set_flag_from_tag(n.para, tag_d);
                        } else {
                            self.nil_set_flag_one(n.para);
                        }
                    }
                } else if let Some(r) = self.bank_int(n.para) {
                    // plain int literals (no cast wrapper — the cast arm
                    // keeps its range checks) load directly into the bank reg
                    if is_plain_int_literal(&*n.subx) {
                        let k = int_literal(&*n.subx).unwrap();
                        self.kill_fwd_reg(r);
                        self.li_k(r, k);
                        if stracked {
                            self.nil_set_flag_one(n.para);
                        }
                        return Ok(());
                    }
                    self.eval_int(&*n.subx)?;
                    let tag_d = self.vstack.len() - 1;
                    let v = self.pop_reg();
                    self.kill_fwd_reg(r);
                    if v != r {
                        self.asm.mv(r, v);
                    }
                    if stracked {
                        if msrc {
                            self.nil_set_flag_from_tag(n.para, tag_d);
                        } else {
                            self.nil_set_flag_one(n.para);
                        }
                    }
                } else if int_literal(&*n.subx) == Some(0)
                    && self.zero_init_skip.get(n.para as usize).copied().flatten()
                        == Some(self.web_idx)
                    && !stracked
                {
                    // `var x = 0` elided into the prologue frame zero (the
                    // timeline proved this store is the slot's first event
                    // and no backedge can resurrect a stale value). A
                    // TRACKED slot cannot take this path: its init flag
                    // must be set exactly when the store executes.
                    self.fwd_int[n.para as usize] = None;
                    return Ok(());
                } else {
                    let off = self.slot_offset(n.para);
                    self.eval_int(&*n.subx)?;
                    let tag_d = self.vstack.len() - 1;
                    let v = self.pop_reg();
                    let s = n.para as usize;
                    if self.store_is_dead(n.para) {
                        // dead store: the value is never read again on any
                        // path (the web timeline proves it); the expression
                        // (and its faults) still evaluate in full
                        self.fwd_int[s] = None;
                    } else {
                        self.asm.sd(v, S0, off);
                        self.fwd_int[s] = Some(v);
                        if stracked {
                            if msrc {
                                self.nil_set_flag_from_tag(n.para, tag_d);
                            } else {
                                self.nil_set_flag_one(n.para);
                            }
                        }
                    }
                }
            }
            XOP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: XOP node shape unexpected")?;
                let (lxop, idx) = decode_local_operand_mark(n.para);
                if matches!(self.slot.get(idx as usize), Some(SlotTy::Input)) {
                    // GT-c: asserted core substring "input parameter cannot be assigned"
                        // stays contiguous (rv64_gen_corpus.rs P2 message pin);
                        // the hint appends after it
                        return errf!(
                            "rv64 lower: the input parameter cannot be assigned; the call-data \
                             argument is read-only — copy it into a local first \
                             (e.g. `var t = data`) and assign to that"
                        );
                }
                // P3-V2IMPL defensive: compound assignment on a list slot is
                // plan-rejected (the interpreter faults CastFail(71) on the
                // Compo old value; the kernel subset rejects up front)
                if matches!(self.slot.get(idx as usize), Some(SlotTy::List)) {
                    return errf!(
                        "rv64 lower: opcode XOP is outside the translatable kernel subset \
                         (compound assignment on a list is not supported)"
                    );
                }
                // constant rhs: the guards/arithmetic absorb the constant
                // (`i += 1` → c.addi + c.beqz; `x /= 16` → c.srli …)
                let kop = lxop_inst(lxop);
                // BUG-012: the OLD value's Nil check runs before the op
                // (local_operand converts the old value first — 71)
                let xop_old_nil = self
                    .nil_occ
                    .get(&node_addr(node))
                    .map(|o| o.xop_old)
                    .unwrap_or(false);
                let rhs_nil = self.nil_val(&*n.subx);
                if let Some(k) = int_literal(&*n.subx) {
                    if Self::arith_k_handles(kop, k) {
                        if xop_old_nil {
                            self.nil_guard_flag(idx, 71);
                        }
                        if let Some(r) = self.bank_int(idx) {
                            self.kill_fwd_reg(r);
                            if self.emit_arith_k(kop, r, k) {
                                return Ok(());
                            }
                            unreachable!("arith_k_handles said yes");
                        }
                        let off = self.slot_offset(idx);
                        let sc1 = self.asm.sc1;
                        self.asm.ld(sc1, S0, off);
                        if !self.emit_arith_k(kop, sc1, k) {
                            unreachable!("arith_k_handles said yes");
                        }
                        if self.store_is_dead(idx) {
                            self.fwd_int[idx as usize] = None;
                        } else {
                            self.asm.sd(sc1, S0, off);
                            // BUG-018: the addi-path store went to memory
                            // through the dedicated scratch (sc1 is never a
                            // pool home), so there is nothing to cache — but
                            // a forward record left by an EARLIER store to
                            // this slot must die with it, or the next
                            // empty-stack GET reads the stale register
                            // (`a += 1u64` immediately read returned the
                            // pre-XOP value in Im/Imc).
                            self.fwd_int[idx as usize] = None;
                        }
                        return Ok(());
                    }
                }
                self.eval_int(&*n.subx)?;
                if xop_old_nil {
                    self.nil_guard_flag(idx, 71);
                }
                if rhs_nil {
                    self.nil_guard_tag(self.vstack.len() - 1, 71);
                }
                if let Some(rw) = self.bank_int(idx) {
                    // promoted: the arithmetic lands on the WRITE home; the
                    // old value may live in the previous segment's register
                    // (XOP is a segment boundary) — copy it across first.
                    // The rhs must not alias `rw` (it may be a Ref to the
                    // same slot: e.g. `i += i` — materialize it first).
                    let rr = self.bank_int_read(idx).unwrap_or(rw);
                    self.kill_fwd_reg(rw);
                    let rv = match self.pop_vreg() {
                        VLoc::Pool(v) => v,
                        VLoc::Ref(v) if v == rw => {
                            // the rhs aliases the write home: materialize it
                            // into a pool register. The push/pop pair keeps
                            // the vstack balanced (BUG-013: the push without
                            // the pop leaked one entry per self-ref XOP and
                            // rejected the whole ImcRemap body at function
                            // end); the value survives in `t`, consumed by
                            // the emit_arith below before any new push.
                            let t = self.push_vreg();
                            self.asm.mv(t, v);
                            let _ = self.pop_vreg();
                            t
                        }
                        VLoc::Ref(v) => v,
                    };
                    if rr != rw {
                        self.asm.mv(rw, rr);
                    }
                    return self.emit_arith(lxop_inst(lxop), rw, rv);
                }
                let rv = self.pop_reg();
                let off = self.slot_offset(idx);
                // Forwarded old value: a live fwd entry names the bottom
                // pool register (see the GET arm's soundness rule — XOP is
                // statement-level, so the stack is empty here and vregs[0]
                // is free-but-holding). Bank-register fwd entries are
                // excluded: the arithmetic destination must never be
                // another slot's bank home.
                let fr = self
                    .fwd_int
                    .get(idx as usize)
                    .copied()
                    .flatten()
                    .filter(|r| {
                        self.fwd_enabled()
                            && self.fwd_slot_ok(idx as usize)
                            && self.vregs.first() == Some(r)
                    });
                // at statement level (empty pool) the first pool registers
                // are free and window-resident, so the slot traffic
                // compresses (c.ld/c.sd) — pick one the rhs is not using;
                // nested inside an expression the pool is live, so fall
                // back to the dedicated scratch.
                let sc1 = match fr {
                    Some(r) => r,
                    None if self.vstack.is_empty() => {
                        if rv != self.vregs[0] {
                            self.vregs[0]
                        } else {
                            self.vregs[1]
                        }
                    }
                    None => self.asm.sc1,
                };
                if fr.is_none() {
                    self.asm.ld(sc1, S0, off);
                }
                self.emit_arith(lxop_inst(lxop), sc1, rv)?;
                if self.store_is_dead(idx) {
                    // dead write: the faulting arithmetic above is still
                    // fully emitted; only the store disappears
                    self.fwd_int[idx as usize] = None;
                } else {
                    self.asm.sd(sc1, S0, off);
                    if self.vregs.contains(&sc1) {
                        // sc1 is a pool register — cache the value
                        self.fwd_int[idx as usize] = Some(sc1);
                    } else {
                        self.fwd_int[idx as usize] = None;
                    }
                }
            }
            /* ---- binary ints (checked) ---- */
            ADD | SUB | MUL | DIV | MOD | MAX | MIN | ABSDIFF | DIVUP | DIVEXACT | BSHL
            | BSHR | BXOR | BOR | BAND | EQ | NEQ | LT | GT | LE | GE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: binary node shape unexpected")?;
                // constant-rhs absorption (`x / 16` → srli, `px >> 1` →
                // c.srli, `x % 16` → c.andi, checked guards folded away).
                // ADD/MUL also accept the constant on the left.
                if let Some(k) = int_literal(&*n.suby).filter(|k| Self::arith_k_handles(op, *k)) {
                    let lx = self.nil_val(&*n.subx);
                    self.eval_int(&*n.subx)?;
                    if lx {
                        // Nil operand → CastFail(71) before the op
                        self.nil_guard_tag(self.vstack.len() - 1, 71);
                    }
                    let d = self.pop_own_replace();
                    if !self.emit_arith_k(op, d, k) {
                        unreachable!("arith_k_handles said yes");
                    }
                    return Ok(());
                }
                if matches!(op, ADD | MUL) {
                    if let Some(k) =
                        int_literal(&*n.subx).filter(|k| Self::arith_k_handles(op, *k))
                    {
                        let ly = self.nil_val(&*n.suby);
                        self.eval_int(&*n.suby)?;
                        if ly {
                            self.nil_guard_tag(self.vstack.len() - 1, 71);
                        }
                        let d = self.pop_own_replace();
                        if !self.emit_arith_k(op, d, k) {
                            unreachable!("arith_k_handles said yes");
                        }
                        return Ok(());
                    }
                }
                let lx = self.nil_val(&*n.subx);
                let ly = self.nil_val(&*n.suby);
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                if lx || ly {
                    self.nil_guard_int_pair(op, lx, ly);
                }
                let (d, _ra, rb) = self.pop2_dest();
                self.emit_arith(op, d, rb)?;
            }
            POW => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: pow node shape unexpected")?;
                // Constant small exponent → unrolled checked muls (the
                // common `x ** 2` shape: 3 insts instead of a ~10-inst loop).
                let const_exp = crate::lang::ir_literal_value(&*n.suby)
                    .ok()
                    .and_then(|v| v)
                    .and_then(|v| v.extract_u128().ok())
                    .filter(|&e| e <= 64);
                if let Some(e) = const_exp {
                    let mb = self.nil_val(&*n.subx);
                    self.eval_int(&*n.subx)?; // base
                    if mb {
                        self.nil_guard_tag(self.vstack.len() - 1, 71);
                    }
                    let d = self.pop_own_replace();
                    match e {
                        0 => self.asm.addi(d, ZERO, 1),
                        1 => {}
                        _ => {
                            self.asm.mv(self.a0, d); // preserved base
                            for _ in 1..e {
                                self.asm.r(1, self.a0, d, 3, self.a1, 0b0110011); // mulhu
                                self.asm.bne(self.a1, ZERO, self.fault_arith);
                                self.asm.r(1, self.a0, d, 0, d, 0b0110011); // mul
                            }
                        }
                    }
                } else {
                    let (mb, me) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                    self.eval_int(&*n.subx)?; // base
                    self.eval_int(&*n.suby)?; // exp
                    // BUG-021 family hardening (main-agent re-review: the
                    // bounds check must PRECEDE the guard-index arithmetic —
                    // deferred depth_err pushes can leave the stack short,
                    // and a wrapped base would emit a guard reading a wrong
                    // frame byte). Same checked_sub form as CLAMP.
                    let pow_base = match self.vstack.len().checked_sub(2) {
                        Some(b) => b,
                        None => {
                            return errf!(
                                "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                            );
                        }
                    };
                    if mb {
                        self.nil_guard_tag(pow_base, 71);
                    }
                    if me {
                        self.nil_guard_tag(pow_base + 1, 71);
                    }
                    let rb = self.pop_reg();
                    let la = self.pop_vreg();
                    let d = self.push_vreg();
                    self.asm.mv(self.a0, loc_reg(la)); // preserve the base
                    self.asm.addi(d, ZERO, 1); // result
                    let l_loop = self.asm.new_label();
                    let l_end = self.asm.new_label();
                    self.place(l_loop);
                    self.asm.beq(rb, ZERO, l_end);
                    self.asm.r(1, self.a0, d, 3, self.a1, 0b0110011); // mulhu a1, d, base
                    self.asm.bne(self.a1, ZERO, self.fault_arith);
                    self.asm.r(1, self.a0, d, 0, d, 0b0110011); // mul d, d, base
                    self.asm.addi(rb, rb, -1);
                    self.asm.jal_(ZERO, l_loop);
                    self.place(l_end);
                }
            }
            CLAMP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: clamp node shape unexpected")?;
                self.eval_int(&*n.subx)?; // x
                self.eval_int(&*n.suby)?; // lo
                self.eval_int(&*n.subz)?; // hi
                // Nil operands → CastFail(71) (the interpreter converts
                // all three args before clamp_checked), x → lo → hi
                let (mx, mlo, mhi) = (
                    self.nil_val(&*n.subx),
                    self.nil_val(&*n.suby),
                    self.nil_val(&*n.subz),
                );
                // BUG-021 family hardening: under a shrunken pool the
                // deferred depth_err pushes leave the vstack short — bail
                // with the depth reject the campaign retry keys on (the
                // raw subtraction underflows in debug and wraps into a
                // wild tag offset in release)
                let base = match self.vstack.len().checked_sub(3) {
                    Some(b) => b,
                    None => {
                        return errf!(
                            "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                        );
                    }
                };
                if mx {
                    self.nil_guard_tag(base, 71);
                }
                if mlo {
                    self.nil_guard_tag(base + 1, 71);
                }
                if mhi {
                    self.nil_guard_tag(base + 2, 71);
                }
                let hi = self.pop_reg();
                let lo = self.pop_reg();
                let x = self.pop_vreg();
                let d = self.push_vreg();
                if let VLoc::Ref(r) = x {
                    if d != r {
                        self.asm.mv(d, r);
                    }
                }
                // clamp_checked rejects lo > hi BEFORE clamping
                // (interpreter Arithmetic 90, instruction.rs clamp_checked);
                // the unchecked min/max chain alone silently returned lo —
                // BUG-010. hi < lo <=> lo > hi.
                self.asm.bltu(hi, lo, self.fault_arith);
                self.emit_min(d, hi); // d = min(x, hi)
                self.emit_max(d, lo); // d = max(d, lo)
            }
            /* ---- unary / logical ---- */
            NOT => {
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: not node shape unexpected")?;
                self.eval_int(&*sub.subx)?;
                let r = self.pop_own_replace();
                self.asm.i(1, r, 3, r, 0b0010011); // sltiu r, r, 1
            }
            AND | OR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: logic node shape unexpected")?;
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                let (d, _ra, rb) = self.pop2_dest();
                if op == AND {
                    self.asm.and_(d, d, rb);
                } else {
                    self.asm.or_(d, d, rb);
                }
            }
            /* ---- bytes ---- */
            SIZE => {
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: size node shape unexpected")?;
                if let Some(b) = bytes_literal(&*sub.subx) {
                    let r = self.push_vreg();
                    self.asm.li(r, b.len() as u64);
                } else {
                    self.eval_bytes(&*sub.subx)?;
                    let rl = self.pop_reg();
                    let lp = self.pop_vreg();
                    let d = self.push_vreg();
                    if let VLoc::Ref(r) = lp {
                        let _ = r; // ptr dead: result is the length
                    }
                    if d != rl {
                        self.asm.mv(d, rl);
                    }
                }
            }
            CAT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: cat node shape unexpected")?;
                // Fold at compile time: lit ++ lit → one pool literal;
                // x ++ "" / "" ++ x → x (zero copy, no arena traffic).
                let lx = bytes_literal(&*n.subx);
                let ly = bytes_literal(&*n.suby);
                if lx.as_ref().is_some_and(|a| a.is_empty()) {
                    // concat still extracts the OTHER side: a Nil operand
                    // faults BytesHandle(91) even against the empty fold
                    let m = self.nil_val(&*n.suby);
                    self.eval_bytes(&*n.suby)?;
                    if m {
                        self.nil_guard_tag(self.vstack.len() - 2, 91);
                    }
                } else if ly.as_ref().is_some_and(|b| b.is_empty()) {
                    let m = self.nil_val(&*n.subx);
                    self.eval_bytes(&*n.subx)?;
                    if m {
                        self.nil_guard_tag(self.vstack.len() - 2, 91);
                    }
                } else if let (Some(a), Some(b)) = (lx, ly) {
                    let mut cat = a;
                    cat.extend_from_slice(&b);
                    let lbl = self.lit_label(&cat);
                    let rp = self.push_vreg();
                    let rl = self.push_vreg();
                    self.lit_addr(rp, lbl);
                    self.asm.li(rl, cat.len() as u64);
                } else {
                    let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                    self.eval_bytes(&*n.subx)?;
                    self.eval_bytes(&*n.suby)?;
                    // concat extracts x first, then y — Nil → 91 each
                    if mx {
                        self.nil_guard_tag(self.vstack.len() - 4, 91);
                    }
                    if my {
                        self.nil_guard_tag(self.vstack.len() - 2, 91);
                    }
                    self.emit_cat()?;
                }
            }
            BYTE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: byte node shape unexpected")?;
                let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_bytes(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                // interpreter order: the index's u16 cast fires first
                // (72), then the bytes extract (91)
                if my {
                    self.nil_guard_tag(self.vstack.len() - 1, 72);
                }
                if mx {
                    self.nil_guard_tag(self.vstack.len() - 3, 91);
                }
                self.emit_byte_at()?;
            }
            CUT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: cut node shape unexpected")?;
                let (mx, my, mz) =
                    (self.nil_val(&*n.subx), self.nil_val(&*n.suby), self.nil_val(&*n.subz));
                self.eval_bytes(&*n.subx)?;
                self.eval_int(&*n.suby)?; // ost
                self.eval_int(&*n.subz)?; // len
                // interpreter order: len's u16 cast, ost's, then the
                // bytes extract
                if mz {
                    self.nil_guard_tag(self.vstack.len() - 1, 72);
                }
                if my {
                    self.nil_guard_tag(self.vstack.len() - 2, 72);
                }
                if mx {
                    self.nil_guard_tag(self.vstack.len() - 4, 91);
                }
                self.emit_slice(SliceKind::Cut)?;
            }
            LEFT | RIGHT | LDROP | RDROP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: slice node shape unexpected")?;
                let m = self.nil_val(&*n.subx);
                self.eval_bytes(&*n.subx)?;
                if m {
                    // count is a ≤255 literal: only the bytes side can be Nil
                    self.nil_guard_tag(self.vstack.len() - 2, 91);
                }
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, n.para as i32);
                let kind = match op {
                    LEFT => SliceKind::Left,
                    RIGHT => SliceKind::Right,
                    LDROP => SliceKind::LDrop,
                    _ => SliceKind::RDrop,
                };
                self.emit_slice(kind)?;
            }
            /* ---- value-position control: blocks and choose ---- */
            IRLIST | IRBLOCK | IRBLOCKR => {
                let arr = node
                    .as_any()
                    .downcast_ref::<IRNodeArray>()
                    .ok_or("rv64 lower: block node shape unexpected")?;
                let keep_last = matches!(op, IRBLOCKR | IRLIST);
                let last = arr.subs.len().saturating_sub(1);
                for (i, sub) in arr.subs.iter().enumerate() {
                    let k = keep_last && i == last && sub.hasretval();
                    self.eval(&**sub)?;
                    if sub.hasretval() && !k {
                        let ty = node_res_ty(&**sub, self.slot)?;
                        self.pop_vreg();
                        if ty == Ty::Bytes {
                            self.pop_vreg();
                        }
                    }
                }
            }
            IRIFR => {
                // value-position if: branch-LAZY on BOTH engines — the
                // bytecode channel compiles IRIFR through compile_if
                // (compile.rs compile_triple handles IRIF|IRIFR), so the
                // untaken arm never evaluates on either side. Both arms
                // evaluate into the SAME pool registers (the vstack hands
                // out by depth, and each arm starts at the same depth), so
                // at the join the stack is rebased to exactly one value.
                // The compile-time vstack accumulates both arms' pushes;
                // pop each arm's results and re-push the (identical)
                // locations.
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                let ty = node_res_ty(&*n.suby, self.slot)?;
                let vals = if ty == Ty::Bytes { 2 } else { 1 };
                let l_else = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_else)?;
                self.eval(&*n.suby)?;
                // take (do NOT re-push): arm2 must evaluate at the same
                // stack level so its value lands in the same register
                let a_locs = self.take_arm_values(vals);
                // BUG-020: per-path tag materialization at the join. Both
                // arms evaluate at the SAME vstack depth and the join
                // re-push lands the value exactly where the taken arm's
                // own pushes sat — a may-Nil arm (bare GET / nested
                // choose / value-if / block tail: the only shapes
                // nil_plan marks may-Nil) already wrote its REAL tag
                // there at runtime, so there is nothing to copy (source
                // == result cell, unlike the CHOOSE select whose arms
                // sit at distinct depths). A MECHANISM-LESS arm (proven
                // non-Nil: a faulting op like `255 + t0`) wrote nothing,
                // leaving the prologue-zeroed cell — which reads as
                // "Nil" — under a consumer guard that fires whenever
                // my||mn. Each path therefore publishes non-Nil (1)
                // exactly when ITS arm is mechanism-less and the other
                // arm makes the join may-Nil; my=mn=false emits nothing
                // (no guard can read the tag — all-non-Nil census shapes
                // stay byte-identical).
                let my = self.nil_val(&*n.suby);
                let mn = self.nil_val(&*n.subz);
                if mn && !my {
                    let base = self.vstack.len();
                    for i in 0..vals {
                        self.nil_set_tag_one(base + i);
                    }
                }
                self.asm.jal_(ZERO, l_end);
                self.place(l_else);
                self.eval(&*n.subz)?;
                let b_locs = self.take_arm_values(vals);
                if my && !mn {
                    let base = self.vstack.len();
                    for i in 0..vals {
                        self.nil_set_tag_one(base + i);
                    }
                }
                self.place(l_end);
                // both arms now hold their value in the same pool
                // registers (refs were materialized inside their own arm
                // region); a future pool change cannot hide a divergence
                debug_assert_eq!(a_locs, b_locs, "value-if arms must share registers");
                // bottom-first re-push: the bytes pair goes back as
                // (ptr, len) — the old `.rev()` order swapped them (BUG-011)
                for l in a_locs.into_iter() {
                    self.span_open_at(self.vstack.len());
                    self.vstack.push(l);
                }
            }
            CHOOSE => {
                // ternary `? :` / choose(): EAGER — the bytecode channel
                // evaluates ALL THREE operands before selecting
                // (compile_triple lowers only IRIF|IRIFR lazily; CHOOSE
                // falls to the eager IRNodeTriple::codegen_into path, and
                // the interpreter's CHOOSE just picks among the three
                // already-evaluated stack values). Evaluate cond → yes →
                // no in the bytecode stack order, then branch-select into
                // the cond's positions: an arm's traps fire exactly where
                // the interpreter fires them (D9 fix).
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                let ty = node_res_ty(&*n.suby, self.slot)?;
                let ty_no = node_res_ty(&*n.subz, self.slot)?;
                if ty != ty_no {
                    // BUG-017 (facet c): the arm value count (1 int / 2
                    // bytes) is read off the YES arm; mismatched arms made
                    // the cond index underflow the vstack (bytes-first
                    // panicked) or mis-pop the stack (int-first leaked one
                    // entry and rejected late with an unrelated text). The
                    // vstack is statically typed per position — reject the
                    // shape up front, cleanly.
                    return errf!(
                        "rv64 lower: choose arms must have the same result type (yes {:?}, no {:?})",
                        ty, ty_no
                    );
                }
                let vals = if ty == Ty::Bytes { 2 } else { 1 };
                self.eval_int(&*n.subx)?; // cond
                self.eval(&*n.suby)?; // yes
                self.eval(&*n.subz)?; // no
                // BUG-021: the 8-value register pool may be exhausted
                // mid-arm — push_vreg's depth_err deferral then leaves the
                // vstack SHORT by design (emission continues so the final
                // reject carries one clean diagnostic), and this direct
                // index is the lowerer's only unchecked vstack read: on a
                // shrunk stack it must bail out with the depth_err reject
                // text instead of the release-profile wrapped-index OOB
                // panic that killed whole mass runs (the campaign's
                // "nesting exceeds" downsizing retry keys on the text).
                let cond_idx = self
                    .vstack
                    .len()
                    .checked_sub(1 + 2 * vals)
                    .and_then(|i| self.vstack.get(i).copied());
                let c_reg = match cond_idx {
                    Some(l) => loc_reg(l),
                    None => {
                        return errf!(
                            "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                        );
                    }
                };
                // Materialize each arm's values bottom-first into their own
                // consecutive pool positions (the BUG-011 take_arm_values
                // treatment). The selection arm kept the old LIFO pops and
                // indexed the reversed bytes (len, ptr) against the
                // bottom-first dests, so a bytes×bytes choose copied
                // (len, ptr) into (ptr, len) — the interpreter returned the
                // taken arm's bytes while all three rv64 modes swapped the
                // pair (BUG-017). The registers keep holding the values
                // after the take (the vstack pops are balanced), so the
                // select below still reads them.
                let no_locs = self.take_arm_values(vals);
                let yes_locs = self.take_arm_values(vals);
                let _ = self.pop_vreg(); // cond (dead after the select)
                let mut dests = Vec::with_capacity(vals);
                for _ in 0..vals {
                    dests.push(self.push_vreg());
                }
                // Nil tags ride along with the selection (BUG-012): the
                // result's tag = the taken arm's tag. Each path is gated on
                // ITS OWN arm's may-Nil verdict (BUG-019: the shared
                // `my || mn` gate made a mechanism-less arm's stale tag
                // byte — prologue zero reads as "Nil" — get copied over the
                // live result, so the consumer's guard invented a 71). A
                // mechanism-less arm (proven non-Nil ⇒ its evaluation never
                // writes tag bytes) publishes a non-Nil tag (1) instead:
                // whenever my||mn the consumer's guard WILL read the dest
                // tag, so the byte must be valid on every selectable path.
                // With my=mn=false nothing is emitted (no guard can read
                // the tag — byte-identical to the all-non-Nil census
                // shapes).
                let entry = self.vstack.len() - vals;
                let my = self.nil_val(&*n.suby);
                let mn = self.nil_val(&*n.subz);
                let l_yes = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.asm.bne(c_reg, ZERO, l_yes);
                for (i, &d) in dests.iter().enumerate() {
                    let s = loc_reg(no_locs[i]);
                    if d != s {
                        self.asm.mv(d, s);
                    }
                }
                if my || mn {
                    for i in 0..vals {
                        if mn {
                            self.nil_copy_tag(entry + 1 + vals + i, entry + i);
                        } else {
                            self.nil_set_tag_one(entry + i);
                        }
                    }
                }
                self.asm.jal_(ZERO, l_end);
                self.place(l_yes);
                for (i, &d) in dests.iter().enumerate() {
                    let s = loc_reg(yes_locs[i]);
                    if d != s {
                        self.asm.mv(d, s);
                    }
                }
                if my || mn {
                    for i in 0..vals {
                        if my {
                            self.nil_copy_tag(entry + 1 + i, entry + i);
                        } else {
                            self.nil_set_tag_one(entry + i);
                        }
                    }
                }
                self.place(l_end);
            }
            /* ---- P3-V2IMPL: the admitted list consumers ---- */
            ITEMGET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: itemget node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                let kind = self.lists.kind[s];
                let static_len = match kind {
                    ListKind::Empty => Some(0u64),
                    ListKind::Rodata { n } | ListKind::FrameStatic { n } => Some(n),
                    ListKind::Growable { .. } => None,
                    ListKind::NotList => {
                        return errf!(
                            "rv64 lower: opcode ITEMGET is outside the translatable kernel subset"
                        );
                    }
                };
                // constant index: in-range loads are guard-free (the frame/
                // table offsets are compile-time); out-of-range (including
                // the u32 index space) is a B-family compile reject — the
                // depth-9 / literal-cast-overflow fail-fast precedent
                let key_lit = crate::lang::ir_literal_value(&*n.suby)
                    .ok()
                    .flatten()
                    .and_then(|v| v.extract_u128().ok());
                if let Some(k) = key_lit {
                    if k >= 1u128 << 32 {
                        return errf!(
                            "rv64 lower: ITEMGET constant index {k} exceeds the u32 index space"
                        );
                    }
                    // Static-length classes (Empty/Rodata/FrameStatic) prove
                    // the bounds at compile time: in-range loads stay
                    // guard-free, out-of-range rejects above.
                    if let Some(sl) = static_len {
                        if k >= sl as u128 {
                            return errf!(
                                "rv64 lower: ITEMGET constant index {k} is out of bounds \
                                 for a list of length {sl}"
                            );
                        }
                    }
                    let r = self.push_vreg();
                    match kind {
                        ListKind::Rodata { .. } => {
                            let lbl = self.list_rodata_lbl(s)?;
                            self.lit_addr(self.asm.sc1, lbl);
                            if self.list_rodata_u32(s) {
                                self.asm.lwu(r, self.asm.sc1, (k as i32) * 4);
                            } else {
                                self.asm.ld(r, self.asm.sc1, (k as i32) * 8);
                            }
                        }
                        // Growable: the length is a RUNTIME value — the
                        // constant index needs the live `k < len` guard
                        // (BUG-028: the guard-less load read stale frame
                        // bytes past the list end and silently swallowed
                        // the interpreter's CompoNoFindItem(85) trap).
                        ListKind::Growable { .. } => {
                            let l85 = self.nil_thunk(85);
                            if k >= 128 {
                                // the cap invariant keeps len ≤ 128, so any
                                // k ≥ 128 faults unconditionally — no load
                                self.asm.jal_(ZERO, l85);
                            } else {
                                let sc1 = self.asm.sc1;
                                let sc2 = self.asm.sc2;
                                self.asm.li(sc2, k as u64); // li temp = sc1 (free here)
                                let cell = self.slot_offset(s as u8);
                                self.asm.ld(sc1, S0, cell); // len
                                self.asm.bgeu(sc2, sc1, l85); // k >= len → 85
                                let off = self.list_data_off[s] + (k as i32) * 8;
                                self.asm.ld(r, S0, off);
                            }
                        }
                        _ => {
                            let off = self.list_data_off[s] + (k as i32) * 8;
                            self.asm.ld(r, S0, off);
                        }
                    }
                    return Ok(());
                }
                // variable index: Nil → 72 (extract_u32), ≥ 2³² → 72,
                // ≥ len → 85 — in exactly that order (list.rs:36-40 reads
                // through extract_u32 first)
                self.eval_int(&*n.suby)?;
                let kd = self.vstack.len() - 1;
                if self.nil_val(&*n.suby) {
                    self.nil_guard_tag(kd, 72);
                }
                let kr = loc_reg(self.vstack[kd]);
                self.emit_list_bounds_guards(kr, s, static_len)?;
                if static_len != Some(0) {
                    // (an Empty list's always-85 guard exits above; there
                    // is no element block to load from) The result MUST go
                    // to a FRESH pool register: the key may have been
                    // pushed as a bank Ref (ImcRemap promoted locals), and
                    // overwriting its home register in place would clobber
                    // the promoted slot.
                    let rodata = match kind {
                        ListKind::Rodata { .. } => Some(self.list_rodata_lbl(s)?),
                        _ => None,
                    };
                    self.pop_vreg();
                    let r = self.push_vreg();
                    self.emit_list_load(r, kr, s, rodata)?;
                }
            }
            LENGTH => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: length node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                // folded to the u64-width shadow / constant (the pinned
                // D5/D7 width family: `as bytes` yields 8B, the interpreter
                // U32 4B — pinned divergence rows, not failures)
                let r = self.push_vreg();
                match self.lists.kind[s] {
                    ListKind::Empty => self.asm.addi(r, ZERO, 0),
                    ListKind::Rodata { n } | ListKind::FrameStatic { n } => {
                        self.asm.addi(r, ZERO, n as i32)
                    }
                    ListKind::Growable { .. } => {
                        let cell = self.slot_offset(s as u8);
                        self.asm.ld(r, S0, cell);
                    }
                    ListKind::NotList => {
                        return errf!(
                            "rv64 lower: opcode LENGTH is outside the translatable kernel subset"
                        );
                    }
                }
            }
            HASKEY => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: haskey node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                // key extract_u32 first (Nil → 72, ≥ 2³² → 72 — even
                // though an out-of-bounds key merely folds to false,
                // list.rs:30-33 runs through extract_u32)
                self.eval_int(&*n.suby)?;
                let kd = self.vstack.len() - 1;
                if self.nil_val(&*n.suby) {
                    self.nil_guard_tag(kd, 72);
                }
                let kr = loc_reg(self.vstack[kd]);
                let l72 = self.nil_thunk(72);
                let sc1 = self.asm.sc1;
                self.asm.li(sc1, 1u64 << 32);
                self.asm.bgeu(kr, sc1, l72);
                // the 0/1 result MUST go to a fresh pool register (the key
                // may be a bank Ref — see the ITEMGET note above)
                self.pop_vreg();
                let r = self.push_vreg();
                match self.lists.kind[s] {
                    ListKind::Empty => self.asm.addi(r, ZERO, 0),
                    ListKind::Rodata { n } | ListKind::FrameStatic { n } => {
                        self.asm.addi(sc1, ZERO, n as i32);
                        // sltu r, kr, sc1 (r = idx < len)
                        self.asm.r(0, sc1, kr, 3, r, 0b0110011);
                    }
                    ListKind::Growable { .. } => {
                        let cell = self.slot_offset(s as u8);
                        self.asm.ld(sc1, S0, cell);
                        self.asm.r(0, sc1, kr, 3, r, 0b0110011);
                    }
                    ListKind::NotList => {
                        return errf!(
                            "rv64 lower: opcode HASKEY is outside the translatable kernel subset"
                        );
                    }
                }
            }
            // defensive: these are plan-rejected before lowering ever runs
            NEWLIST | PACKLIST | APPEND | CLEAR => {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (a list value cannot be used as a value)"
                )
            }
            other => {
                return errf!(
                    "rv64 lower: opcode {:?} is outside the translatable kernel subset",
                    other
                )
            }
        }
        Ok(())
    }

    /// Branch to `label` when `node` evaluates nonzero (`when_true`) or zero.
    /// Fuses comparisons into single conditional branches (the 0/1
    /// materialization disappears), including the `addi rd, rd, -K; c.beqz`
    /// compare-with-small-constant form. Operand evaluation order and fault
    /// behavior are identical to `eval`.
    fn branch_cond(&mut self, node: &dyn IRNode, when_true: bool, label: usize) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.branch_cond(&*w.node, when_true, label);
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            NOT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: not node shape unexpected")?;
                self.branch_cond(&*n.subx, !when_true, label)
            }
            AND | OR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: logic node shape unexpected")?;
                if op == AND && !when_true && is_pure_cond(node, &self.tracked) {
                    // fault-free conjunction: any false operand jumps — a
                    // branch chain, no materialization at all
                    self.branch_cond(&*n.subx, false, label)?;
                    self.branch_cond(&*n.suby, false, label)?;
                    return Ok(());
                }
                // eager (identical fault behavior to `eval`)
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                let (d, _ra, rb) = self.pop2_dest();
                if op == AND {
                    self.asm.and_(d, d, rb);
                } else {
                    self.asm.or_(d, d, rb);
                }
                let _ = self.pop_vreg(); // d (dead after the branch)
                if when_true {
                    self.asm.bne(d, ZERO, label);
                } else {
                    self.asm.beq(d, ZERO, label);
                }
                Ok(())
            }
            EQ | NEQ | LT | GT | LE | GE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: binary node shape unexpected")?;
                // (f3, swapped): the branch to emit when the comparison is
                // TRUE, in terms of (a, b) = (subx, suby). f3 ^ 1 inverts.
                let (f3, swap) = match op {
                    EQ => (0u32, false),
                    NEQ => (1, false),
                    LT => (6, false), // bltu a, b
                    GT => (6, true),  // a > b  <=>  bltu b, a
                    LE => (7, true),  // a <= b <=>  bgeu b, a
                    _ => (7, false),  // a >= b <=>  bgeu a, b
                };
                let f3 = if when_true { f3 } else { f3 ^ 1 };
                let lx = int_literal(&*n.subx);
                let ly = int_literal(&*n.suby);
                let lit = if lx.is_some() && ly.is_none() {
                    lx.map(|k| (true, k)) // literal on the left
                } else {
                    ly.map(|k| (false, k)) // literal on the right (or both)
                };
                if let Some((lit_left, k)) = lit {
                    // comparisons never fault and literals are pure: skip the
                    // literal operand entirely
                    let sub: &dyn IRNode = if lit_left { &*n.suby } else { &*n.subx };
                    let eq_like = matches!(op, EQ | NEQ);
                    // == K / != K with a small constant folds into
                    // addi r, r, -K; c.beqz/c.bnez (wrapping equality)
                    if eq_like && (0..=31).contains(&k) {
                        let msub = self.nil_val(sub);
                        self.eval_int(sub)?;
                        if msub {
                            // Nil vs literal: exactly-one-Nil → 90
                            self.nil_guard_tag(self.vstack.len() - 1, 90);
                        }
                        let l = *self
                            .vstack
                            .last()
                            .ok_or("rv64 lower: eq-like branch on empty stack")?;
                        let f = if (op == EQ) == when_true { 0 } else { 1 };
                        match l {
                            VLoc::Pool(r) => {
                                self.pop_vreg();
                                // the temp is dead after the branch: fold K
                                // into it (addi -K; c.beqz/c.bnez)
                                if k != 0 {
                                    self.asm.addi(r, r, -(k as i32));
                                }
                                self.asm.branch(f, r, ZERO, label);
                            }
                            VLoc::Ref(r) if k == 0 => {
                                self.pop_vreg();
                                // compare with zero needs no constant at all
                                self.asm.branch(f, r, ZERO, label);
                            }
                            VLoc::Ref(r) => {
                                // ref registers are read-only here: li + full
                                // cmp. The temp is pushed WHILE the ref still
                                // occupies its stack slot, so it can never be
                                // the same register (a pop-first push at an
                                // empty stack re-hands vregs[0] and the li
                                // would clobber the ref's value — a real bug
                                // forwarding introduced).
                                let t = self.push_vreg();
                                self.li_k(t, k);
                                self.pop_vreg(); // the ref (value still in r)
                                let (a_reg, b_reg) =
                                    if lit_left { (t, r) } else { (r, t) };
                                if swap {
                                    self.asm.branch(f3, b_reg, a_reg, label);
                                } else {
                                    self.asm.branch(f3, a_reg, b_reg, label);
                                }
                                let _ = self.pop_vreg(); // t (dead after the branch)
                            }
                        }
                        return Ok(());
                    }
                    // general: evaluate the non-literal side, materialize K
                    let msub = self.nil_val(sub);
                    self.eval_int(sub)?;
                    if msub {
                        self.nil_guard_tag(self.vstack.len() - 1, 90);
                    }
                    let t = self.push_vreg();
                    self.li_k(t, k);
                    // pop both (the registers keep their values; nothing is
                    // emitted in between), then branch in (a, b) order
                    let kreg = self.pop_reg();
                    let v = self.pop_reg();
                    // emitted order: swap mirrors (a,b) -> (b,a) for GT/LE
                    let (a_reg, b_reg) = if lit_left { (kreg, v) } else { (v, kreg) };
                    if swap {
                        self.asm.branch(f3, b_reg, a_reg, label);
                    } else {
                        self.asm.branch(f3, a_reg, b_reg, label);
                    }
                    return Ok(());
                }
                let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                if mx || my {
                    self.nil_guard_int_pair(op, mx, my);
                }
                let rb = self.pop_reg();
                let ra = self.pop_reg();
                if swap {
                    self.asm.branch(f3, rb, ra, label);
                } else {
                    self.asm.branch(f3, ra, rb, label);
                }
                Ok(())
            }
            _ => {
                self.eval_int(node)?;
                let rc = self.pop_reg();
                if when_true {
                    self.asm.bne(rc, ZERO, label);
                } else {
                    self.asm.beq(rc, ZERO, label);
                }
                Ok(())
            }
        }
    }

    /// Store an int value register into a local slot (banked home or frame).
    fn store_int_slot(&mut self, slot: u8, v: u8) {
        if let Some(r) = self.bank_int(slot) {
            self.kill_fwd_reg(r);
            if r != v {
                self.asm.mv(r, v);
            }
            self.fwd_int[slot as usize] = None;
        } else if self.store_is_dead(slot) {
            self.fwd_int[slot as usize] = None;
        } else {
            let off = self.slot_offset(slot);
            self.asm.sd(v, S0, off);
            self.fwd_int[slot as usize] = Some(v);
        }
    }

    /// Lower a matched constant-select run as one bounds-checked byte-table
    /// lookup: `t = x - base; if t >= span skip; t = &table; v = t[x];
    /// slot = v`. Holes and the skipped path both leave the slot at D, which
    /// the preceding default store guarantees.
    fn emit_const_select(
        &mut self,
        read_slot: u8,
        write_slot: u8,
        arms: &[SelArm],
        d: u64,
    ) -> Ret<()> {
        let base = arms.iter().map(|a| a.key).min().unwrap();
        let top = arms.iter().map(|a| a.key).max().unwrap();
        let span = (top - base + 1) as usize;
        let mut table = vec![d as u8; span];
        for a in arms {
            table[(a.key - base) as usize] = a.val as u8;
        }
        let lbl = self.lit_label(&table);
        // operand: a pure slot read evaluated once (the chain re-read it
        // per arm; slot reads cannot fault, so hoisting is sound)
        let mut extra = 0usize;
        let xv = if let Some(r) = self.bank_int_read(read_slot) {
            r
        } else {
            let off = self.slot_offset(read_slot);
            let r = self.push_vreg();
            self.asm.ld(r, S0, off);
            extra += 1;
            r
        };
        let t = self.push_vreg();
        if base == 0 {
            self.asm.mv(t, xv);
        } else {
            self.asm.addi(t, xv, -(base as i32));
        }
        let sr = self.push_vreg();
        self.asm.addi(sr, ZERO, span as i32); // c.li (span ≤ 31)
        let skip = self.asm.new_label();
        self.asm.bgeu(t, sr, skip);
        let _ = self.pop_vreg(); // sr dead past the branch
        // the index stays in t; the table pointer needs its own register
        // (reusing t here would clobber the index and add the RAW x)
        let p = self.push_vreg();
        self.lit_addr(p, lbl);
        self.asm.add(p, p, t);
        let v = self.push_vreg();
        self.asm.lbu(v, p, 0);
        self.store_int_slot(write_slot, v);
        let _ = self.pop_vreg(); // v
        let _ = self.pop_vreg(); // p
        let _ = self.pop_vreg(); // t
        for _ in 0..extra {
            let _ = self.pop_vreg(); // xv (if loaded)
        }
        self.place(skip);
        Ok(())
    }

    /// The single `PUT slot ← expr` statement of a then-arm, or None.
    fn single_put_arm<'n>(&self, arm: &'n dyn IRNode) -> Option<&'n IRNodeParam1Single> {
        if let Some(w) = arm.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.single_put_arm(&*w.node);
        }
        match Bytecode::try_from_u8(arm.bytecode()) {
            Ok(Bytecode::PUT) => arm.as_any().downcast_ref::<IRNodeParam1Single>(),
            Ok(Bytecode::IRBLOCK) | Ok(Bytecode::IRBLOCKR) | Ok(Bytecode::IRLIST) => {
                let arr = arm.as_any().downcast_ref::<IRNodeArray>()?;
                if arr.subs.len() == 1 {
                    self.single_put_arm(&*arr.subs[0])
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Try to lower `if <cmp> { slot = <other> }` (else-less) as one fused
    /// max/min update: `slot = max(slot, other)` for `if other > slot` /
    /// `if slot < other`, min for the opposite direction. The compared
    /// operand is materialized ONCE (the generic path re-materializes it
    /// inside the arm). Returns true when the statement was consumed.
    fn try_maxmin_update(&mut self, node: &dyn IRNode) -> Ret<bool> {
        let n = match node.as_any().downcast_ref::<IRNodeTriple>() {
            Some(n)
                if Bytecode::try_from_u8(node.bytecode()).ok() == Some(Bytecode::IRIF)
                    && is_zero_byte_stmt(&*n.subz) =>
            {
                n
            }
            _ => return Ok(false),
        };
        let cond = Bytecode::try_from_u8(n.subx.bytecode()).ok();
        if !matches!(cond, Some(Bytecode::GT) | Some(Bytecode::LT)) {
            return Ok(false);
        }
        let c = match n.subx.as_any().downcast_ref::<IRNodeDouble>() {
            Some(c) => c,
            None => return Ok(false),
        };
        let put = match self.single_put_arm(&*n.suby) {
            Some(p) => p,
            None => return Ok(false),
        };
        let slot = put.para;
        if matches!(self.slot.get(slot as usize), Some(SlotTy::Input) | None) {
            return Ok(false);
        }
        // the arm operand must be the SAME value the slot was compared with:
        // a matching literal, or a read of the same slot-free expression is
        // too weak — require literal-on-both-sides or GET-of-slot on one
        // side only (the operand is the other side, literal or expr).
        // one side must read exactly this slot; the other is the operand
        let x_is_slot = slot_get_index(&*c.subx) == Some(slot);
        let y_is_slot = slot_get_index(&*c.suby) == Some(slot);
        if x_is_slot == y_is_slot {
            return Ok(false); // both or neither — not the update shape
        }
        let (slot_on_left, operand, cmp) = if x_is_slot {
            (true, &*c.suby, cond.unwrap())
        } else {
            (false, &*c.subx, cond.unwrap())
        };
        // the arm must store the very same operand (literal equal, or the
        // identical GET)
        let arm_matches = match int_literal(operand) {
            Some(k) => int_literal(&*put.subx) == Some(k),
            None => {
                slot_get_index(operand).is_some()
                    && slot_get_index(&*put.subx) == slot_get_index(operand)
                    && int_literal(&*put.subx).is_none()
            }
        };
        if !arm_matches {
            return Ok(false);
        }
        // direction: `slot < K` or `K > slot` → max(slot, K); `slot > K` or
        // `K < slot` → min(slot, K)
        let is_max = match (cmp, slot_on_left) {
            (Bytecode::LT, true) | (Bytecode::GT, false) => true,
            (Bytecode::GT, true) | (Bytecode::LT, false) => false,
            _ => return Ok(false),
        };
        // ---- emit ----
        // operand first (identical fault order to the original condition:
        // the IRIF evaluates subx, whose left child evaluates first). The
        // operand stays ON the stack while the slot's home is loaded — a
        // pop-first fresh push would re-hand (and the ld clobber) the very
        // register the operand lives in (a real bug the first cut shipped).
        // Nil guards (BUG-012): the ordered comparison faults 90 on ANY
        // Nil side — the slot side reads its init flag directly.
        let opnode: &dyn IRNode = if slot_on_left { &*c.suby } else { &*c.subx };
        let slot_get: &dyn IRNode = if slot_on_left { &*c.subx } else { &*c.suby };
        let (m_slot, m_op) = (self.nil_val(slot_get), self.nil_val(opnode));
        let slot_first = slot_on_left; // comparison evaluation order
        if slot_first && m_slot {
            self.nil_guard_flag(slot, 90);
        }
        self.eval_int(opnode)?;
        if m_op {
            self.nil_guard_tag(self.vstack.len() - 1, 90);
        }
        if !slot_first && m_slot {
            self.nil_guard_flag(slot, 90);
        }
        let t = loc_reg(
            *self.vstack.last().ok_or("rv64 lower: maxmin operand missing")?,
        );
        // slot's current value: bank write home (segment boundary = this
        // PUT), copying across from the read home when they differ
        if let Some(rw) = self.bank_int(slot) {
            let rr = self.bank_int_read(slot).unwrap_or(rw);
            self.kill_fwd_reg(rw);
            self.fwd_int[slot as usize] = None;
            if rr != rw {
                self.asm.mv(rw, rr);
            }
            if is_max {
                self.emit_max(rw, t);
            } else {
                self.emit_min(rw, t);
            }
            if self.tracked.get(slot as usize).copied() == Some(true) {
                // the guards proved both inputs non-Nil
                self.nil_set_flag_one(slot);
            }
        } else {
            let off = self.slot_offset(slot);
            // the operand is still pushed, so this fresh register can
            // never be the operand's
            let d = self.push_vreg();
            self.asm.ld(d, S0, off);
            if is_max {
                self.emit_max(d, t);
            } else {
                self.emit_min(d, t);
            }
            if self.store_is_dead(slot) {
                self.fwd_int[slot as usize] = None;
            } else {
                self.asm.sd(d, S0, off);
                self.fwd_int[slot as usize] = Some(d);
                if self.tracked.get(slot as usize).copied() == Some(true) {
                    self.nil_set_flag_one(slot);
                }
            }
            let _ = self.pop_vreg(); // d
        }
        let _ = self.pop_vreg(); // the operand
        Ok(true)
    }

    /// Try to lower a maximal constant-select run starting at `subs[start]`.
    /// Returns the number of subs consumed, or None (fall back to per-stmt
    /// lowering). All conditions are static on the frozen IR.
    fn try_const_select_run(
        &mut self,
        subs: &[Box<dyn IRNode>],
        start: usize,
    ) -> Ret<Option<usize>> {
        let mut arms: Vec<SelArm> = Vec::new();
        let mut rw: Option<(u8, u8)> = None;
        let mut consumed = 0usize;
        let mut j = start;
        while j < subs.len() {
            if is_zero_byte_stmt(&*subs[j]) {
                if arms.is_empty() {
                    return Ok(None); // runs start on a real arm
                }
                j += 1;
                consumed += 1;
                continue;
            }
            match match_sel_arm(&*subs[j]) {
                Some((g, w, a)) => {
                    match rw {
                        None => rw = Some((g, w)),
                        Some(x) if x != (g, w) => break,
                        _ => {}
                    }
                    arms.push(a);
                    j += 1;
                    consumed += 1;
                }
                None => break,
            }
        }
        if arms.len() < 4 {
            return Ok(None);
        }
        let mut keys: Vec<u64> = arms.iter().map(|a| a.key).collect();
        keys.sort_unstable();
        if keys.windows(2).any(|w| w[0] == w[1]) {
            return Ok(None); // first-hit-wins vs table conflict
        }
        let base = keys[0];
        let span = (keys[keys.len() - 1] - base + 1) as usize;
        if span > 31 || base > 2047 {
            return Ok(None);
        }
        let d = match default_predecessor(subs, start, rw.unwrap().1) {
            Some(d) => d,
            None => return Ok(None),
        };
        // Nil guard (BUG-012): every arm compares the read slot against a
        // literal — a Nil slot faults 90 at the FIRST comparison, before
        // any arm can store. The preceding default PUT already set the
        // write slot's flag.
        if self.tracked.get(rw.unwrap().0 as usize).copied() == Some(true) {
            self.nil_guard_flag(rw.unwrap().0, 90);
        }
        self.emit_const_select(rw.unwrap().0, rw.unwrap().1, &arms, d)?;
        Ok(Some(consumed))
    }

    fn eval_int(&mut self, node: &dyn IRNode) -> Ret<()> {
        let ty = node_res_ty(node, self.slot)?;
        if ty != Ty::Int {
            return errf!("rv64 lower: expected an int value, got bytes");
        }
        self.eval(node)
    }

    fn eval_bytes(&mut self, node: &dyn IRNode) -> Ret<()> {
        let ty = node_res_ty(node, self.slot)?;
        if ty != Ty::Bytes {
            return errf!("rv64 lower: expected a bytes value, got int");
        }
        self.eval(node)
    }

    /* ---- checked arithmetic: `d = d OP rhs` ---- */

    /// Size-optimized forms: fault paths are one `jal` to a shared thunk, so
    /// the straight-line cost is the minimum the semantics allow.
    fn emit_arith(&mut self, op: Bytecode, d: u8, rhs: u8) -> Ret<()> {
        use Bytecode::*;
        match op {
            ADD => {
                // overflow iff result < rhs (unsigned)
                self.asm.add(d, d, rhs);
                self.asm.bltu(d, rhs, self.fault_arith);
            }
            SUB => {
                // underflow iff old d < rhs — check BEFORE the sub
                self.asm.bltu(d, rhs, self.fault_arith);
                self.asm.sub(d, d, rhs);
            }
            MUL => {
                self.asm.r(1, rhs, d, 3, self.a1, 0b0110011); // mulhu
                self.asm.bne(self.a1, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 0, d, 0b0110011); // mul
            }
            DIV => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 5, d, 0b0110011); // divu
            }
            DIVEXACT => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 7, self.a1, 0b0110011); // remu
                self.asm.bne(self.a1, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 5, d, 0b0110011); // divu
            }
            DIVUP => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 7, self.a1, 0b0110011); // remu (dividend in d)
                self.asm.r(1, rhs, d, 5, d, 0b0110011); // divu
                let join = self.asm.new_label();
                self.asm.beq(self.a1, ZERO, join);
                self.asm.addi(d, d, 1);
                self.place(join);
            }
            MOD => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 7, d, 0b0110011); // remu
            }
            MAX => self.emit_max(d, rhs),
            MIN => self.emit_min(d, rhs),
            ABSDIFF => {
                let l_b = self.asm.new_label();
                let l_e = self.asm.new_label();
                self.asm.bgeu(d, rhs, l_b);
                self.asm.r(0x20, d, rhs, 0, d, 0b0110011); // d = rhs - d
                self.asm.jal_(ZERO, l_e);
                self.place(l_b);
                self.asm.r(0x20, rhs, d, 0, d, 0b0110011); // d = d - rhs
                self.place(l_e);
            }
            BXOR => self.asm.xor(d, d, rhs),
            BOR => self.asm.or_(d, d, rhs),
            BAND => self.asm.and_(d, d, rhs),
            BSHL => {
                self.asm.i(63, rhs, 7, self.a1, 0b0010011); // andi a1, rhs, 63
                self.asm.r(0, self.a1, d, 1, d, 0b0110011); // sll
            }
            BSHR => {
                self.asm.i(63, rhs, 7, self.a1, 0b0010011);
                self.asm.r(0, self.a1, d, 5, d, 0b0110011); // srl
            }
            EQ => {
                self.asm.xor(d, d, rhs); // xor (c.xor in the window)
                self.asm.i(1, d, 3, d, 0b0010011); // sltiu d, d, 1
            }
            NEQ => {
                self.asm.xor(d, d, rhs); // xor (c.xor in the window)
                self.asm.r(0, d, ZERO, 3, d, 0b0110011); // sltu d, x0, d
            }
            LT => {
                self.asm.r(0, rhs, d, 3, d, 0b0110011); // sltu d, d, rhs
            }
            GT => {
                self.asm.r(0, d, rhs, 3, d, 0b0110011); // sltu d, rhs, d
            }
            LE => {
                // d <= rhs  ⇔  !(rhs < d): sltu sc1, rhs, d — rs1=rhs,
                // rs2=d (the r() signature lists rs2 first). Two past bugs
                // lived here: xori funct3 written 12 (bit-spill into rs1)
                // and this operand swap, which made LE behave as GE. Both
                // passed naive tests by value coincidence; the HIP5 scorer
                // caught them (see module docs).
                let sc1 = self.asm.sc1;
                self.asm.r(0, d, rhs, 3, sc1, 0b0110011); // sltu sc1, rhs, d
                self.asm.i(1, sc1, 4, d, 0b0010011); // xori d, sc1, 1
            }
            GE => {
                // d >= rhs  ⇔  !(d < rhs)
                self.asm.r(0, rhs, d, 3, d, 0b0110011); // sltu d, d, rhs
                self.asm.i(1, d, 4, d, 0b0010011); // xori d, d, 1
            }
            other => {
                // GT-c: "prototype" wording retired (doc 66 §1.4) — the
                // kernel subset is a product contract, not a prototype
                // limitation; this arm is plan-should-have-rejected
                // territory, so it is an internal report, not a user error
                return errf!(
                    "rv64 lower: internal: arithmetic op {other:?} has no lowering (opcode \
                     outside the translatable kernel subset; plan should have rejected it)"
                )
            }
        }
        Ok(())
    }

    /// Whether `emit_arith_k` has a constant-specialized form for
    /// `d = d OP k` (checked before the operand is consumed).
    fn arith_k_handles(op: Bytecode, k: u64) -> bool {
        use Bytecode::*;
        let pow2 = k.count_ones() == 1 && k != 0;
        match op {
            ADD | SUB => k <= 1,
            MUL => k <= 1 || pow2,
            DIV => k == 1 || pow2 || k >= 2, // any const: no zero-guard
            DIVEXACT | DIVUP => k == 1 || (op == DIVEXACT && pow2),
            MOD => k == 1 || pow2 || k >= 2,
            BSHL | BSHR => k <= 63,
            BAND => k <= 2047,
            _ => false,
        }
    }

    /// `d = d OP k` with a constant rhs — the checked guards absorb what
    /// the constant proves (nonzero divisors, overflow-impossible forms)
    /// and the arithmetic folds into immediate/compressed shapes.
    /// Returns false when unhandled (caller falls back to the reg form).
    fn emit_arith_k(&mut self, op: Bytecode, d: u8, k: u64) -> bool {
        use Bytecode::*;
        let pow2j = |k: u64| -> Option<u32> {
            (k != 0 && k.count_ones() == 1).then(|| k.trailing_zeros())
        };
        match op {
            ADD if k == 0 => {}
            ADD if k == 1 => {
                // +1 overflows iff the result wrapped to 0
                self.asm.addi(d, d, 1);
                self.asm.beq(d, ZERO, self.fault_arith);
            }
            SUB if k == 0 => {}
            SUB if k == 1 => {
                // -1 underflows iff d was already 0
                self.asm.beq(d, ZERO, self.fault_arith);
                self.asm.addi(d, d, -1);
            }
            MUL if k == 0 => self.asm.addi(d, ZERO, 0),
            MUL if k == 1 => {}
            MUL if pow2j(k).is_some() => {
                // x * 2^j: overflow iff x's top j bits were set
                let j = pow2j(k).unwrap();
                let t = self.push_vreg();
                self.asm.srli_(t, d, 64 - j);
                self.asm.bne(t, ZERO, self.fault_arith);
                let _ = self.pop_vreg();
                self.asm.slli_(d, d, j);
            }
            DIV if k == 1 => {}
            DIV if pow2j(k).is_some() => {
                // const nonzero divisor: no zero-guard; unsigned / 2^j = srli
                self.asm.srli_(d, d, pow2j(k).unwrap());
            }
            DIV if k >= 2 => {
                // const nonzero: the beq rhs, ZERO guard is dead
                let t = self.push_vreg();
                self.asm.li(t, k);
                self.asm.r(1, t, d, 5, d, 0b0110011); // divu d, d, t
                let _ = self.pop_vreg();
            }
            DIVEXACT if k == 1 => {}
            DIVEXACT if pow2j(k).is_some() => {
                let j = pow2j(k).unwrap();
                let t = self.push_vreg();
                self.asm.andi(t, d, (k - 1) as i32);
                self.asm.bne(t, ZERO, self.fault_arith);
                let _ = self.pop_vreg();
                self.asm.srli_(d, d, j);
            }
            DIVUP if k == 1 => {}
            MOD if k == 1 => self.asm.addi(d, ZERO, 0),
            MOD if pow2j(k).is_some() => {
                self.asm.andi(d, d, (k - 1) as i32);
            }
            MOD if k >= 2 => {
                let t = self.push_vreg();
                self.asm.li(t, k);
                self.asm.r(1, t, d, 7, d, 0b0110011); // remu d, d, t
                let _ = self.pop_vreg();
            }
            BSHL if k <= 63 => {
                // constant shamt < 64: the mask-to-63 is a no-op
                self.asm.slli_(d, d, k as u32);
            }
            BSHR if k <= 63 => {
                self.asm.srli_(d, d, k as u32);
            }
            BAND if k <= 2047 => {
                self.asm.andi(d, d, k as i32);
            }
            _ => return false,
        }
        true
    }

    /// `d = min(d, rhs)` — branch form: 3 insts, 2 cycles (profile 0 has no
    /// Zbb min/max; the 5-inst branchless sequence loses on both axes).
    /// `d = min(d, rhs)` — take rhs only when rhs < d. Both directions
    /// were historically inverted (bltu skip forms) — a latent bug nothing
    /// e2e-tested (CLAMP/MAX/MIN never ran with distinguishing operands)
    /// until the max/min-update fusion exercised them.
    fn emit_min(&mut self, d: u8, rhs: u8) {
        let skip = self.asm.new_label();
        self.asm.bgeu(rhs, d, skip);
        self.asm.mv(d, rhs);
        self.place(skip);
    }

    /// `d = max(d, rhs)` — take rhs only when d < rhs.
    fn emit_max(&mut self, d: u8, rhs: u8) {
        let skip = self.asm.new_label();
        self.asm.bgeu(d, rhs, skip);
        self.asm.mv(d, rhs);
        self.place(skip);
    }

    /* ---- arena / bytes helpers ---- */

    /// `for i in 0..len: *S1++ = src[i]` — clobbers the scratch pair, bumps S1.
    fn emit_memcpy(&mut self, src: u8, len: u8) {
        self.asm.addi(self.a0, ZERO, 0);
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.bgeu(self.a0, len, l_done);
        self.asm.add(self.a1, src, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.sb(self.a1, S1, 0);
        self.asm.addi(S1, S1, 1);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
    }

    /// Arena-exhaustion check for `S1 + len_reg` against the declared end.
    fn emit_arena_check(&mut self, len_reg: u8) {
        self.asm.li(self.a0, self.arena_end);
        self.asm.add(self.a1, S1, len_reg);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast); // arena exhausted
    }

    /// Top of stack: `a ++ b` → one bytes descriptor.
    fn emit_cat(&mut self) -> Ret<()> {
        // operands are read-only (memcpy sources, lengths); the two result
        // slots are fresh pool registers.
        let lb = self.pop_reg();
        let pb = self.pop_reg();
        let la = self.pop_reg();
        let pa = self.pop_reg();
        // total = la + lb (checked)
        self.asm.r(0, lb, la, 0, self.a1, 0b0110011); // add a1 = la + lb
        let ok = self.asm.new_label();
        self.asm.bgeu(self.a1, la, ok);
        self.trap(EXIT_ARITH);
        self.place(ok);
        self.emit_arena_check(self.a1);
        // copies bump S1 by la then lb
        self.emit_memcpy(pa, la);
        self.emit_memcpy(pb, lb);
        let dstp = self.push_vreg();
        let dstl = self.push_vreg();
        self.asm.r(0, lb, la, 0, dstl, 0b0110011); // add dstl, la + lb
        self.asm.r(0x20, dstl, S1, 0, dstp, 0b0110011); // sub dstp, S1 - dstl
        Ok(())
    }

    /// Top of stack: bytes, idx → u8 at idx (traps out of range).
    fn emit_byte_at(&mut self) -> Ret<()> {
        let idx = self.pop_reg();
        let rl = self.pop_reg();
        let lp = self.pop_vreg();
        self.asm.bgeu(idx, rl, self.fault_cast);
        let rp = loc_reg(lp);
        let d = self.push_vreg();
        self.asm.add(d, rp, idx);
        self.asm.lbu(d, d, 0);
        Ok(())
    }

    /* ---- P3-IDIOMS: fused byte-assembly groups (doc 94) ---- */

    /// Wide load at ptr + off (imm when it fits the 12-bit I-immediate,
    /// otherwise a scratch-materialized address). f3: 3=ld, 6=lwu, 1=lhu.
    fn emit_wide_at(&mut self, d: u8, rp: u8, off: u64, f3: u8) {
        if off <= 2047 {
            match f3 {
                3 => self.asm.ld(d, rp, off as i32),
                6 => self.asm.lwu(d, rp, off as i32),
                _ => self.asm.lhu(d, rp, off as i32),
            }
        } else {
            let sc1 = self.asm.sc1;
            self.li_k(sc1, off);
            self.asm.add(sc1, rp, sc1);
            match f3 {
                3 => self.asm.ld(d, sc1, 0),
                6 => self.asm.lwu(d, sc1, 0),
                _ => self.asm.lhu(d, sc1, 0),
            }
        }
    }

    /// LE group: one span guard (lemma 1: c0+n-1 ≥ len covers the union
    /// of the N per-byte guards, same 71 thunk) + one zero-extending
    /// wide load (lemma 2: bit-identical to the |/^ tree value).
    fn emit_fused_le(&mut self, recv: &dyn IRNode, c0: u64, n: u64) -> Ret<()> {
        self.eval_bytes(recv)?;
        let rl = self.pop_reg();
        let lp = self.pop_vreg();
        let rp = loc_reg(lp);
        let sc1 = self.asm.sc1;
        self.li_k(sc1, c0 + n - 1);
        self.asm.bgeu(sc1, rl, self.fault_cast);
        let d = self.push_vreg();
        let (f3, w) = match n {
            8 => (3u8, 8u64),
            4 => (6u8, 4u64),
            _ => (1u8, 2u64),
        };
        self.emit_wide_at(d, rp, c0, f3);
        debug_assert!(w == n);
        Ok(())
    }

    /// BE mul-add group (lemma 3 + 4). `base`: None = const offsets
    /// (single guard on c0+n-1); Some = var base (guard on the base like
    /// the original leaf-0 path, then the overflow-free subtraction span
    /// guard `len - t0 < n → 71`), then the wide load and the shift
    /// recombination.
    fn emit_fused_be(
        &mut self,
        recv: &dyn IRNode,
        base: Option<&dyn IRNode>,
        c0: u64,
        n: u64,
    ) -> Ret<()> {
        self.eval_bytes(recv)?;
        let (rp, rl, t0) = match base {
            None => {
                let rl = self.pop_reg();
                let lp = self.pop_vreg();
                (loc_reg(lp), rl, None)
            }
            Some(b) => {
                self.eval_int(b)?;
                let t0 = self.pop_reg();
                let rl = self.pop_reg();
                let lp = self.pop_vreg();
                (loc_reg(lp), rl, Some(t0))
            }
        };
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        match t0 {
            None => {
                // const offsets: one guard covers all leaves (lemma 1)
                self.li_k(sc1, c0 + n - 1);
                self.asm.bgeu(sc1, rl, self.fault_cast);
            }
            Some(t0) => {
                // var base: leaf-0 guard first (identical to the original
                // first byte()'s guard), then len - t0 ≥ n via subtraction
                // (safe: t0 < len proven by the first guard — no wrap)
                self.asm.bgeu(t0, rl, self.fault_cast);
                self.li_k(sc1, n);
                self.asm.sub(sc2, rl, t0);
                self.asm.bltu(sc2, sc1, self.fault_cast);
            }
        }
        let d = self.push_vreg();
        match t0 {
            None => {
                let (f3, off) = match n {
                    4 => (6u8, c0),
                    _ => (1u8, c0),
                };
                self.emit_wide_at(d, rp, off, f3);
            }
            Some(t0) => {
                self.asm.add(sc2, rp, t0);
                match n {
                    4 => self.asm.lwu(d, sc2, 0),
                    _ => self.asm.lhu(d, sc2, 0),
                }
            }
        }
        // shift recombination (lemma 3): x = the zero-extended LE load.
        // 64-bit shifts do NOT clear bytes above bit 32 — every byte is
        // masked out explicitly (only sc1/sc2, never a pool register).
        match n {
            4 => {
                // y = b0<<24 | b1<<16 | b2<<8 | b3
                self.asm.srli_(sc1, d, 8); // b1 | b2<<8 | b3<<16
                self.asm.andi(sc1, sc1, 0xFF); // b1
                self.asm.slli_(sc1, sc1, 16); // b1<<16
                self.asm.srli_(sc2, d, 16); // b2 | b3<<8
                self.asm.andi(sc2, sc2, 0xFF); // b2
                self.asm.slli_(sc2, sc2, 8); // b2<<8
                self.asm.or_(sc1, sc1, sc2); // b1<<16 | b2<<8
                self.asm.srli_(sc2, d, 24); // b3 (32-bit value: top byte)
                self.asm.or_(sc1, sc1, sc2); // | b3
                self.asm.andi(sc2, d, 0xFF); // b0
                self.asm.slli_(sc2, sc2, 24); // b0<<24
                self.asm.or_(d, sc1, sc2); // full swap into d
            }
            2 => {
                // y = ((x & 0xFF) << 8) | (x >> 8)  (x>>8 < 2^8 here)
                self.asm.andi(sc1, d, 0xFF);
                self.asm.srli_(sc2, d, 8);
                self.asm.slli_(sc1, sc1, 8);
                self.asm.or_(d, sc1, sc2);
            }
            _ => unreachable!("BE groups are N in {{2,4}} only"),
        }
        Ok(())
    }

    /// Dispatch for a marked group root: re-derive the group from the
    /// tree (same deterministic recognition as fuse_scan — the mark
    /// guarantees it succeeds) and emit.
    fn emit_fused_root(&mut self, node: &dyn IRNode, form: FuseForm) -> Ret<()> {
        let root = fuse_unwrap(node);
        match form {
            FuseForm::LeChain | FuseForm::LeExpr => {
                let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64)> = Vec::new();
                fuse_flatten_le(root, &mut terms)
                    .ok_or("rv64 lower: internal: fused LE group shape lost")?;
                let (recv, _, _) = terms[0];
                let (c0, n) = match self.fuse.get(&node_addr(node)) {
                    Some(FuseMark::Root { c0, n, .. }) => (*c0, *n),
                    _ => return errf!("rv64 lower: internal: fused group mark lost"),
                };
                self.emit_fused_le(recv, c0, n)
            }
            FuseForm::BeConst | FuseForm::BeVar => {
                let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64)> = Vec::new();
                fuse_flatten_be(root, &mut terms)
                    .ok_or("rv64 lower: internal: fused BE group shape lost")?;
                let (recv, idx0, _) = terms[0];
                let (c0, n) = match self.fuse.get(&node_addr(node)) {
                    Some(FuseMark::Root { c0, n, .. }) => (*c0, *n),
                    _ => return errf!("rv64 lower: internal: fused group mark lost"),
                };
                let base = match form {
                    FuseForm::BeVar => Some(idx0),
                    _ => None,
                };
                self.emit_fused_be(recv, base, c0, n)
            }
        }
    }

    /// Top of stack: descriptor + count operand(s).
    fn emit_slice(&mut self, kind: SliceKind) -> Ret<()> {
        // Stack layout: Cut = [ptr, len, ost, n] (4 deep), others =
        // [ptr, len, n] (3 deep). Pop order must mirror the pushes.
        let n = self.pop_reg(); // len (Cut) / count (top)
        let ost = match kind {
            SliceKind::Cut => self.pop_reg(),
            _ => n,
        };
        let rl = self.pop_reg();
        let lp = self.pop_vreg();
        let rp = loc_reg(lp);
        match kind {
            SliceKind::Cut => {
                // ost + n must not wrap and ost+n must be <= rl
                self.asm.r(0, n, ost, 0, self.a0, 0b0110011); // add scratch = ost + n
                self.asm.bltu(self.a0, ost, self.fault_cast); // wrapped
                self.asm.bltu(rl, self.a0, self.fault_cast); // rl < ost+n
            }
            _ => {
                self.asm.bltu(rl, n, self.fault_cast); // n > rl
            }
        }
        let dstp = self.push_vreg();
        let dstl = self.push_vreg();
        match kind {
            SliceKind::Cut => {
                self.asm.add(dstp, rp, ost);
                self.asm.mv(dstl, n);
            }
            SliceKind::Left => {
                if dstp != rp {
                    // the ptr survives: a bank-ref source needs the copy
                    self.asm.mv(dstp, rp);
                }
                self.asm.mv(dstl, n);
            }
            SliceKind::Right => {
                self.asm.add(dstp, rp, rl);
                self.asm.r(0x20, n, dstp, 0, dstp, 0b0110011); // sub dstp, dstp, n
                self.asm.mv(dstl, n);
            }
            SliceKind::LDrop => {
                self.asm.add(dstp, rp, n);
                self.asm.r(0x20, n, rl, 0, dstl, 0b0110011); // sub dstl, rl, n
            }
            SliceKind::RDrop => {
                self.asm.r(0x20, n, rl, 0, dstl, 0b0110011); // sub dstl, rl, n
                if dstp != rp {
                    // the prefix ptr survives: a bank-ref source needs the copy
                    // (same shape as SliceKind::Left; BUG-009 fix — dstp was
                    // never written, so Im/Imc read a stale register)
                    self.asm.mv(dstp, rp);
                }
            }
        }
        Ok(())
    }

    /// Top of stack: u64 → 8-byte big-endian bytes descriptor in the arena.
    fn emit_cbytes(&mut self) -> Ret<()> {
        let rp = self.pop_own_replace(); // holds the value, becomes the ptr
        let rl = self.push_vreg();
        self.asm.addi(rl, ZERO, 8);
        // arena room for 8 bytes
        self.asm.li(self.a0, self.arena_end);
        self.asm.addi(self.a1, S1, 8);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast);
        // for i in 0..8: *(S1 + i) = (v >> 8*(7-i)) & 0xff
        let sc = self.asm.sc1;
        self.asm.addi(self.a1, ZERO, 0); // i
        let l_st = self.asm.new_label();
        let l_st_end = self.asm.new_label();
        self.place(l_st);
        self.asm.bgeu(self.a1, rl, l_st_end);
        self.asm.r(0x20, self.a1, rl, 0, sc, 0b0110011); // sub sc, rl, i
        self.asm.addi(sc, sc, -1);
        self.asm.slli_(sc, sc, 3);
        self.asm.r(0, sc, rp, 5, sc, 0b0110011); // srl sc, v, sc
        self.asm.add(self.a0, S1, self.a1);
        self.asm.sb(sc, self.a0, 0);
        self.asm.addi(self.a1, self.a1, 1);
        self.asm.jal_(ZERO, l_st);
        self.place(l_st_end);
        self.asm.add(S1, S1, rl); // bump
        self.asm.r(0x20, rl, S1, 0, rp, 0b0110011); // sub rp, S1, rl (old bump)
        Ok(())
    }

    /// Top of stack: bytes descriptor → copy to output, `a0 = len`, `ret`.
    fn emit_return(&mut self) -> Ret<()> {
        let rl = self.pop_reg();
        let rp = self.pop_reg();
        // fault iff len > output_max (len == output_max is legal)
        self.asm.bltu(S3, rl, self.fault_cast);
        self.asm.addi(self.a0, ZERO, 0); // i
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.bgeu(self.a0, rl, l_done);
        self.asm.add(self.a1, rp, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        let sc1 = self.asm.sc1;
        self.asm.add(sc1, S2, self.a0);
        self.asm.sb(self.a1, sc1, 0);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        self.asm.mv(A0, rl);
        self.asm.ret();
        Ok(())
    }
}

/* ------------------------------------------------------------------ */
/* P0 debug exports. Pure additions supporting the compiler experiments */
/* (docs.loc/rv64im/10-a0-maxlive-and-licm.md): the WebWalk statement    */
/* timeline, the per-statement code-offset trace, and a mirror of the    */
/* ImcRemap promotion plan. Nothing in the lowering reads these; the     */
/* production entry point discards the side-channel, and equality of    */
/* the emitted code is pinned by the `dbg_engine_matches_production`    */
/* test.                                                                */
/* ------------------------------------------------------------------ */

/// `WebEvDbg::kind`: the slot's value is read.
pub const WEB_EV_READ: u8 = 0;
/// `WebEvDbg::kind`: unconditional write (dominates later reads on its path).
pub const WEB_EV_WRITE_U: u8 = 1;
/// `WebEvDbg::kind`: write under if/choose arms (conditional).
pub const WEB_EV_WRITE_C: u8 = 2;

/// `LowerDbg::slot_tys` element mirrors (`SlotTy`).
pub const SLOT_TY_NONE: u8 = 0;
pub const SLOT_TY_INT: u8 = 1;
pub const SLOT_TY_BYTES: u8 = 2;
/// The kernel's `data: bytes` parameter (lives in the ABI input registers).
pub const SLOT_TY_INPUT: u8 = 3;
/// P3-V2IMPL: compo-list slot (the admitted v2 shape set).
pub const SLOT_TY_LIST: u8 = 4;

/// `LowerDbg::banked` element mirrors (`BankKind`).
pub const BANK_KIND_NONE: u8 = 0;
pub const BANK_KIND_INT: u8 = 1;
pub const BANK_KIND_BYTES: u8 = 2;

/// One timeline event of a slot (mirror of `WebWalk::Ev`).
#[derive(Clone, Copy, Debug)]
pub struct WebEvDbg {
    /// statement index (mirrors `WebWalk::walk_stmt` numbering)
    pub idx: usize,
    /// one of `WEB_EV_*`
    pub kind: u8,
    /// region id (while bodies get their own; see `region_parent`)
    pub region: usize,
}

/// The WebWalk statement timeline (the def-use / liveness infrastructure the
/// dead-store pass and the bank plan are built on), exported verbatim.
#[derive(Clone, Debug)]
pub struct WebTimelineDbg {
    /// per-slot type (one of `SLOT_TY_*`); index = slot id
    pub slot_tys: Vec<u8>,
    /// per-slot event list in statement order
    pub ev: Vec<Vec<WebEvDbg>>,
    /// per while statement: (loop-head idx — the condition's reads —, last
    /// body idx); the backedge folds [a, b] circularly
    pub while_spans: Vec<(usize, usize)>,
    /// per slot: read by some while condition (loop-carried)
    pub cond_read: Vec<bool>,
    /// region id → parent region id (`usize::MAX` for the root)
    pub region_parent: Vec<usize>,
    /// final statement index = number of statement nodes walked
    pub stmt_total: usize,
}

/// One lowered statement (or const-select table run): the emitted bytes
/// `[off_start, off_end)` belong to walk-index range `[w_lo, w_hi]`.
#[derive(Clone, Copy, Debug)]
pub struct StmtRangeDbg {
    pub w_lo: usize,
    pub w_hi: usize,
    pub off_start: usize,
    pub off_end: usize,
}

/// Debug side-channel of one lowering.
#[derive(Clone, Debug)]
pub struct LowerDbg {
    pub timeline: WebTimelineDbg,
    /// per emitted unit, in emission order (nested statement ranges overlap)
    pub stmt_ranges: Vec<StmtRangeDbg>,
    /// per-slot bank kind (one of `BANK_KIND_*`)
    pub banked: Vec<u8>,
    /// the register vector handed to the web plan (pool-carved; the literal
    /// pool base is popped off first when active)
    pub bank_regs: Vec<u8>,
    /// value-pool register count (peak expression depth, min 1 cap 8)
    pub pool_size: usize,
    /// literal-pool base register, when the pool-base scheme is active
    pub pool_base_reg: Option<u8>,
    /// frame byte offset per slot (-1 = not in the frame: banked/input)
    pub frame_off: Vec<i32>,
    /// literal sites found by `count_literal_sites` (drives the pool base)
    pub lit_sites: usize,
    /// A-19 batch 2 applied dominated constant-mul check merges (the
    /// mechanical-proof pin set: base register × constant pair × interval)
    pub mul_merges: Vec<MulMergeRecord>,
    /// P3-GUARDTAIL GT-a applied guard-copy clusters (thunk code × copy
    /// position × form × converted site offsets)
    pub guard_copies: Vec<GuardCopyRecord>,
}

/// Literal value of an int IR node (debug wrapper over the crate-internal
/// `int_literal` helper — same extraction the const-select/max-min fusion
/// matches use).
pub fn dbg_int_literal(node: &dyn IRNode) -> Option<u64> {
    int_literal(node)
}

/* ------------------------------------------------------------------ */
/* Tests: encoder round-trips, subset pinning, e2e execution, and the  */
/* HVM-vs-RV64 per-iteration comparison report.                        */
/* ------------------------------------------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::decode::{BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
    use crate::calc::{CalcFnObj, CalcRunLimits};
    use crate::fitshc::compile_body;
    use crate::lang::Tokenizer;

    fn compile_with_input_pub(body: &str) -> (IRNodeArray, Vec<u8>) {
        compile(body)
    }

    fn compile(body: &str) -> (IRNodeArray, Vec<u8>) {
        let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
        let (ir, compiled, _) = compile_body(tokens, vec![], &[], &[], false).unwrap();
        match compiled {
            crate::fitshc::CompiledCode::Bytecode(b) => (ir, b),
            _ => unreachable!(),
        }
    }

    fn lower(body: &str) -> Rv64Lowered {
        let (ir, _) = compile(body);
        lower_body(&ir).unwrap()
    }

    fn lower_mode(body: &str, mode: Rv64Mode) -> Rv64Lowered {
        let (ir, _) = compile(body);
        lower_body_mode(&ir, false, mode).unwrap()
    }

    /// The debug engine must stay byte-identical to the production entry
    /// (the P0 experiment tooling reads the side-channel; see
    /// docs.loc/rv64im/10-a0-maxlive-and-licm.md).
    #[test]
    fn dbg_engine_matches_production() {
        let bodies: Vec<(&str, bool)> = vec![
            ("var s = 0u64\nvar i = 0u64\nwhile i < 10 { s += i\ni += 1 }\nreturn s as bytes", false),
            (
                "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nreturn a as bytes",
                true,
            ),
            (
                "var v = byte(data, 0)\nvar t = 7u64\nif v == 2 { t = 20 }\nif v == 17 { t = 30 }\nif v == 39 { t = 40 }\nreturn t as bytes",
                true,
            ),
            (HIP5_FITSH, true),
        ];
        for (bi, (body, has_input)) in bodies.iter().enumerate() {
            let (ir, _) = if *has_input {
                compile_with_input(body)
            } else {
                compile(body)
            };
            for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                let prod = lower_body_mode_arena(&ir, *has_input, mode, ARENA_SIZE_MAX).unwrap();
                let (dbg_lo, dbg) =
                    lower_body_mode_arena_dbg(&ir, *has_input, mode, ARENA_SIZE_MAX).unwrap();
                assert_eq!(
                    prod.code, dbg_lo.code,
                    "body #{bi} {mode:?}: dbg engine code diverged"
                );
                assert_eq!(prod.pool, dbg_lo.pool, "body #{bi} {mode:?}: pool diverged");
                // timeline sanity: every slot's events are idx-ordered; the
                // walk consumed the same node count the lowering cursor did
                for evs in &dbg.timeline.ev {
                    assert!(
                        evs.windows(2).all(|w| w[0].idx <= w[1].idx),
                        "body #{bi}: event order"
                    );
                }
                assert!(dbg.timeline.stmt_total >= 1);
            }
        }
    }

    fn decode_stream(code: &[u8]) -> Vec<Inst> {
        let mut out = Vec::new();
        let mut off = 0;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, 0).unwrap();
            out.push(inst);
            off += len as usize;
        }
        out
    }

    fn decode_stream_p(code: &[u8], profile: u8) -> Vec<Inst> {
        let mut out = Vec::new();
        let mut off = 0;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, profile).unwrap();
            out.push(inst);
            off += len as usize;
        }
        out
    }

    fn run_e2e(body: &str, input: &[u8]) -> Vec<u8> {
        run_e2e_mode(body, input, Rv64Mode::Im)
    }

    fn run_e2e_mode(body: &str, input: &[u8], mode: Rv64Mode) -> Vec<u8> {
        let lo = lower_mode(body, mode);
        let cap = SpaceCap::new(0);
        let container = match mode {
            Rv64Mode::Im => lo.pack_segments(&cap, 64, 5120, 1280),
            _ => lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC),
        }
        .unwrap_or_else(|e| panic!("pack failed: {e}"));
        let fno = CalcFnObj::build_v1(container).unwrap();
        let exec =
            fno.run(input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
        match exec.result {
            Ok(out) => out,
            Err(e) => panic!("kernel aborted: {e:?} exit={} cycles={}", exec.exit_code, exec.cycles_used),
        }
    }

    /* ---- encoder round-trips (consensus decoder as the oracle) ---- */

    #[test]
    fn assembler_encodings_decode() {
        let mut a = Asm::new();
        let l = a.new_label();
        a.addi(5, 0, -1);
        a.li(6, 2047);
        a.li(7, 2048);
        a.li(28, 0x1234_5678);
        a.li(29, 0xFFFF_8000);
        a.li(30, 0xFFFF_FFFF);
        a.li(16, 0x7FFF_FFFF);
        a.r(0, 6, 5, 0, 7, 0b0110011); // add
        a.r(1, 6, 5, 3, 7, 0b0110011); // mulhu
        a.r(1, 6, 5, 5, 7, 0b0110011); // divu
        a.r(0x20, 6, 5, 0, 7, 0b0110011); // sub
        a.r(0, 6, 5, 3, 7, 0b0110011); // sltu (f3=3!)
        a.ld(5, 8, -16);
        a.sd(5, 8, 2047);
        a.lbu(5, 8, 0);
        a.sb(5, 8, 0);
        a.srli_(5, 6, 31);
        a.slli_(5, 6, 63);
        a.add(5, 6, 7);
        a.bne(5, 6, l);
        a.bgeu(5, 6, l);
        a.bltu(5, 6, l);
        a.jal_(0, l);
        a.pcrel_addr(5, l);
        a.place(l);
        a.ret();
        a.ecall();
        let buf = a.finish().unwrap();
        let insts = decode_stream(&buf);
        assert!(insts.len() >= 26);
        assert!(matches!(insts[0], Inst::ImmAlu { op: ImmAluOp::Addi, rd: 5, imm: -1, .. }));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Add, rd: 7, rs1: 5, rs2: 6 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Mulhu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Divu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Sub, .. })));
        // sltu must decode with the CORRECT rs1 (guards the f3=3 vs 11 bug)
        assert!(insts.iter().any(|i| matches!(i, Inst::RegAlu { op: RegAluOp::Sltu, rd: 7, rs1: 5, rs2: 6 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Load { op: LoadOp::Ld, rd: 5, rs1: 8, imm: -16 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Store { op: StoreOp::Sd, rs1: 8, rs2: 5, imm: 2047 })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Load { op: LoadOp::Lbu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Store { op: StoreOp::Sb, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Branch { kind: BrKind::Ne, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Branch { kind: BrKind::Geu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Branch { kind: BrKind::Ltu, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Auipc { rd: 5, .. })));
        assert!(insts.iter().any(|i| matches!(i, Inst::Ecall)));
    }

    /// RVC encoders round-tripped through the consensus `decode_profile_c`.
    /// The encoder bit layouts were derived by inverting the decoder; this
    /// test pins them against it directly.
    #[test]
    fn rvc_encodings_decode() {
        let mut a = Asm::new_rvc();
        // backward c.j (label placed before the jump)
        let back = a.new_label();
        a.place(back);
        a.addi(5, 5, 4);
        a.jal_(ZERO, back);
        // forward jump: optimistic 2-byte c.j slot, patched by `finish`
        let fwd = a.new_label();
        a.jal_(ZERO, fwd);
        a.place(fwd);
        // c.li / c.addi / c.mv / c.add (full 5-bit rd forms, any register)
        a.addi(10, ZERO, 7); // c.li a0, 7
        a.addi(30, 30, -3); // c.addi t5, -3
        a.mv(14, 10); // c.mv a4, a0
        a.add(14, 14, 10); // c.add a4, a0
        // window-restricted forms
        a.ld(12, 8, 16); // c.ld a2, 16(s0)
        a.sd(12, 8, 24); // c.sd a2, 24(s0)
        a.slli_(12, 12, 3); // c.slli
        a.srli_(12, 12, 2); // c.srli (window)
        a.sub(12, 12, 10); // c.sub (window rr)
        a.xor(12, 12, 11); // c.xor
        a.or_(12, 12, 10); // c.or
        a.and_(12, 12, 11); // c.and
        // c.beqz / c.bnez (backward, window rs1)
        let lb = a.new_label();
        a.place(lb);
        a.beq(12, ZERO, lb);
        a.bne(12, ZERO, lb);
        // non-compressible stays 4-byte: forward beq, non-window regs
        let lf = a.new_label();
        a.beq(5, ZERO, lf);
        a.place(lf);
        a.beq(5, 6, lb);
        a.slli_(28, 28, 2); // c.slli works on any rd
        a.ret(); // c.jr ra
        a.li(10, 100000); // lui+addi -> c.lui + c.addi? (100000 hi = 24)
        let buf = a.finish().unwrap();
        let insts = decode_stream_p(&buf, crate::calc::container::CALC_PROFILE_RV64IMC);
        let find = |f: &dyn Fn(&Inst) -> bool| insts.iter().filter(|i| f(i)).count();
        // c.j backward + forward (both 2-byte now)
        assert_eq!(find(&|i| matches!(i, Inst::Jal { rd: 0, .. })), 2, "jal forms");
        // c.li/c.mv/c.add/c.addi decode to their uncompressed equivalents
        assert!(find(&|i| matches!(i, Inst::ImmAlu { op: ImmAluOp::Addi, rd: 10, rs1: 0, imm: 7 })) >= 1);
        assert!(find(&|i| matches!(i, Inst::ImmAlu { op: ImmAluOp::Addi, rd: 30, imm: -3, .. })) >= 1);
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Add, rd: 14, rs1: 0, rs2: 10 })) >= 1, "c.mv");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Add, rd: 14, rs1: 14, rs2: 10 })) >= 1, "c.add");
        assert!(find(&|i| matches!(i, Inst::Load { op: LoadOp::Ld, rd: 12, rs1: 8, imm: 16 })) == 1, "c.ld");
        assert!(find(&|i| matches!(i, Inst::Store { op: StoreOp::Sd, rs1: 8, rs2: 12, imm: 24 })) == 1, "c.sd");
        assert!(find(&|i| matches!(i, Inst::ShiftImm { kind: ShiftKind::Sll, rd: 12, shamt: 3, word: false, .. })) == 1, "c.slli");
        assert!(find(&|i| matches!(i, Inst::ShiftImm { kind: ShiftKind::Srl, rd: 12, shamt: 2, word: false, .. })) == 1, "c.srli");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Sub, rd: 12, rs1: 12, rs2: 10 })) == 1, "c.sub");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Xor, rd: 12, rs1: 12, rs2: 11 })) == 1, "c.xor");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::Or, rd: 12, rs1: 12, rs2: 10 })) == 1, "c.or");
        assert!(find(&|i| matches!(i, Inst::RegAlu { op: RegAluOp::And, rd: 12, rs1: 12, rs2: 11 })) == 1, "c.and");
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Eq, rs1: 12, rs2: 0, .. })) == 1, "c.beqz");
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Ne, rs1: 12, rs2: 0, .. })) == 1, "c.bnez");
        // forward beq + beq rs1,rs2 stay full-width
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Eq, rs1: 5, rs2: 0, .. })) == 1);
        assert!(find(&|i| matches!(i, Inst::Branch { kind: BrKind::Eq, rs1: 5, rs2: 6, .. })) == 1);
        assert!(find(&|i| matches!(i, Inst::Jalr { rd: 0, rs1: 1, imm: 0 })) == 1, "c.jr");
        // c.slli on non-window rd (x28) still compresses
        assert!(find(&|i| matches!(i, Inst::ShiftImm { kind: ShiftKind::Sll, rd: 28, shamt: 2, word: false, .. })) == 1);
        // 100000 = hi 0x19<<12 + 0x610 -> c.lui + addi(0x610 > 31 -> full-width)
        assert!(find(&|i| matches!(i, Inst::Lui { rd: 10, val: 0x18000, .. })) == 1, "c.lui");
        // verify the stream is 2-byte aligned overall
        assert_eq!(buf.len() % 2, 0);
    }

    /// The three lowering modes must produce identical results through the
    /// consensus engine (RVC path decode + execute end to end).
    #[test]
    fn rvc_modes_agree_e2e() {
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            // li ladder
            for v in [0u64, 2047, 65535, 100000, 0x7FFF_FFFF, 1000000007, u64::MAX] {
                let body = format!("return {v}u64 as bytes");
                let out = run_e2e_mode(&body, &[], mode);
                assert_eq!(out, v.to_be_bytes().to_vec(), "li({v}) mode {mode:?}");
            }
            // checked arithmetic + faults
            let out = run_e2e_mode("return ((3000000000u64 * 3) % 1000000007) as bytes", &[], mode);
            assert_eq!(out, (3000000000u64 * 3 % 1000000007).to_be_bytes().to_vec());
            // loop with break/continue
            let body = "var s = 0u64\nvar i = 0u64\nwhile i < 100 { i += 1\nif i % 3 == 0 { continue }\nif i > 50 { break }\ns += i }\nreturn s as bytes";
            let out = run_e2e_mode(body, &[], mode);
            let want: u64 = (1..=50u64).filter(|i| i % 3 != 0).sum();
            assert_eq!(out, want.to_be_bytes().to_vec(), "loop mode {mode:?}");
            // bytes chain
            let body = "return size(\"abcdef\" ++ \"1234\") as bytes";
            let out = run_e2e_mode(body, &[], mode);
            assert_eq!(out, 10u64.to_be_bytes().to_vec());
        }
        // deep-input remap mode: input bytes plumbing
        let body = "var s = 0u64\nvar i = 0u64\nwhile i < size(data) { s += byte(data, i)\ni += 1 }\nreturn s as bytes";
        let input: Vec<u8> = (0u8..=39).collect();
        let out = run_input_mode(body, &input, Rv64Mode::ImcRemap);
        assert_eq!(out, input.iter().map(|&b| b as u64).sum::<u64>().to_be_bytes().to_vec());
    }

    #[test]
    fn li_values_execute_via_e2e() {
        // li correctness is pinned by returning each constant as bytes.
        for v in [
            0u64, 1, 2047, 2048, 4095, 65535, 0x12345, 0xFFFFFF, 0x7FFF_FFFF, 0xFFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFF, 0x8000_0000_0000_0000, 1000000007,
        ] {
            let body = format!("return {v}u64 as bytes");
            let out = run_e2e(&body, &[]);
            assert_eq!(out, v.to_be_bytes().to_vec(), "li({v}) round trip");
        }
    }

    /// HIP5 Diamond Analyzer v0.1.1 scoring, the fitsh version of
    /// `doc/hacdsdefi4000.loc/score_sum.rs`, as one inlined kernel body (the
    /// contract keeps the multi-function form as comments; `self.` calls are
    /// outside the kernel subset). Wire = names[6n] || numbers[8n] ||
    /// visual_genes[10n]; n = size/24. Output = 8-byte BE u64 sum of
    /// score(record). Deviation vs score_sum.rs: malformed sizes are not
    /// trapped (the loop just iterates floor(size/24)).
    const HIP5_FITSH: &str = r#"// five-dimension score, Diamond Analyzer v0.1.1
var n = size(data) / 24
var ne = n * 6
var nue = ne + n * 8
var sum = 0u64
var i = 0u64
while i < n {
    var nb = i * 6
    var xb = ne + i * 8
    var gb = nue + i * 10

    // ---- shape: nibbles 0/1 of the visual gene ----
    var b0 = byte(data, gb)
    var sa = b0 / 16
    var sb = b0 % 16
    var shape = 0u64
    if sa == 0 {
        if sb == 1 { shape = 30 }
        if sb == 2 { shape = 30 }
        if sb == 3 { shape = 50 }
        if sb == 4 { shape = 20 }
        if sb == 5 { shape = 30 }
        if sb == 6 { shape = 30 }
        if sb == 7 { shape = 20 }
        if sb == 8 { shape = 20 }
    }

    var colorish = 0u64
    if shape == 0 {
        // ---- style (Common shape): nibbles 14..17 = bytes 7/8 ----
        var h7 = byte(data, gb + 7)
        var h8 = byte(data, gb + 8)
        var c13 = h7 / 16
        var c14 = h7 % 16
        var c15 = h8 / 16
        var c16 = h8 % 16
        // positional rules, DA order, first hit wins (mutually exclusive)
        var pos = 0u64
        if c13 == c14 && c14 == c15 && c15 == c16 { pos = 250 } else {
        if c13 == c14 && c14 == c15 && c14 != c16 { pos = 20 } else {
        if c13 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c13 == c14 && c14 == c16 && c15 != c13 { pos = 20 } else {
        if c14 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c16 == c15 && c13 == c14 { pos = 20 } else {
        if c16 == c14 && c13 == c15 { pos = 20 } else {
        if c15 * 16 + c13 == c14 * 16 + c16 { pos = 20 } else {
        if c13 == c14 && c15 != c16 { pos = 3 } else {
        if c15 == c16 && c14 != c13 { pos = 3 }
        }}}}}}}}}
        // bitmap + dark/light flags over nibbles 2..18 in one pass
        var bitmap = 0u64
        var alldark = 1u64
        var alllight = 1u64
        var k = 2u64
        while k < 18 {
            var hv = byte(data, gb + k / 2)
            if k % 2 == 0 { hv = hv / 16 } else { hv = hv % 16 }
            bitmap = bitmap | (1 << hv)
            if hv > 4 { alldark = 0 }
            if hv <= 4 { alllight = 0 }
            k += 1
        }
        // popcount16(bitmap)
        var px = bitmap & 65535
        px = px - ((px >> 1) & 21845)
        px = (px & 13107) + ((px >> 2) & 13107)
        px = (px + (px >> 4)) & 3855
        px = px + (px >> 8)
        var distinct = px & 31
        var cnt = 0u64
        if distinct == 4 { if 200 > cnt { cnt = 200 } }
        if distinct == 5 { if 100 > cnt { cnt = 100 } }
        if distinct == 6 { if 50 > cnt { cnt = 50 } }
        if distinct == 7 { if 20 > cnt { cnt = 20 } }
        if distinct == 14 { if 30 > cnt { cnt = 30 } }
        if distinct == 15 { if 100 > cnt { cnt = 100 } }
        if distinct == 16 { if 200 > cnt { cnt = 200 } }
        var dl = 0u64
        if alldark == 1 { if 20 > dl { dl = 20 } }
        if alllight == 1 { if 20 > dl { dl = 20 } }
        colorish = pos
        if cnt > colorish { colorish = cnt }
        if dl > colorish { colorish = dl }
    } else {
        // ---- color (non-Common shape): name[0] table + 17-nibble light/dark ----
        var nc = byte(data, nb)
        var cs = 0u64
        if nc == 65 { cs = 7 }
        if nc == 66 { cs = 6 }
        if nc == 69 { cs = 1 }
        if nc == 72 { cs = 8 }
        if nc == 73 { cs = 8 }
        if nc == 75 { cs = 8 }
        if nc == 77 { cs = 2 }
        if nc == 78 { cs = 6 }
        if nc == 83 { cs = 6 }
        if nc == 84 { cs = 8 }
        if nc == 85 { cs = 3 }
        if nc == 86 { cs = 8 }
        if nc == 87 { cs = 3 }
        if nc == 88 { cs = 5 }
        if nc == 89 { cs = 3 }
        if nc == 90 { cs = 10 }
        var alldark2 = 1u64
        var alllight2 = 1u64
        var k2 = 2u64
        while k2 < 19 {
            var hv2 = byte(data, gb + k2 / 2)
            if k2 % 2 == 0 { hv2 = hv2 / 16 } else { hv2 = hv2 % 16 }
            if hv2 > 4 { alldark2 = 0 }
            if hv2 <= 4 { alllight2 = 0 }
            k2 += 1
        }
        if alldark2 == 1 { if 20 > cs { cs = 20 } }
        if alllight2 == 1 { if 20 > cs { cs = 20 } }
        colorish = cs
    }

    // ---- nameliteral (6 name bytes) ----
    var maxrun = 1u64
    var run = 1u64
    var q = 1u64
    while q < 6 {
        var c1 = byte(data, nb + q)
        var c0 = byte(data, nb + q - 1)
        if c1 == c0 {
            run += 1
            if run > maxrun { maxrun = run }
        } else {
            run = 1
        }
        q += 1
    }
    var bm2 = 0u64
    q = 0
    while q < 6 {
        var lc = byte(data, nb + q)
        var li = 16u64
        if lc == 87 { li = 0 }
        if lc == 84 { li = 1 }
        if lc == 89 { li = 2 }
        if lc == 85 { li = 3 }
        if lc == 73 { li = 4 }
        if lc == 65 { li = 5 }
        if lc == 72 { li = 6 }
        if lc == 88 { li = 7 }
        if lc == 86 { li = 8 }
        if lc == 77 { li = 9 }
        if lc == 69 { li = 10 }
        if lc == 75 { li = 11 }
        if lc == 66 { li = 12 }
        if lc == 83 { li = 13 }
        if lc == 90 { li = 14 }
        if lc == 78 { li = 15 }
        bm2 = bm2 | (1 << li)
        q += 1
    }
    var px2 = bm2 & 65535
    px2 = px2 - ((px2 >> 1) & 21845)
    px2 = (px2 & 13107) + ((px2 >> 2) & 13107)
    px2 = (px2 + (px2 >> 4)) & 3855
    px2 = px2 + (px2 >> 8)
    var distinct2 = px2 & 31
    var nl = 0u64
    if maxrun >= 3 {
        if maxrun == 6 { nl = 200 }
        if maxrun == 5 { if 50 > nl { nl = 50 } }
        if maxrun == 4 { if 10 > nl { nl = 10 } }
        if maxrun < 4 { if 2 > nl { nl = 2 } }
    }
    if distinct2 == 2 { if 10 > nl { nl = 10 } }
    if distinct2 == 3 { if 3 > nl { nl = 3 } }
    // ABCABC: name[0..3] == name[3..6]
    if byte(data, nb) == byte(data, nb + 3) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 5) {
        if 20 > nl { nl = 20 }
    }
    // Symmetric letter: palindrome
    if byte(data, nb) == byte(data, nb + 5) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 3) {
        if 10 > nl { nl = 10 }
    }
    // AABCCD: 1=2 && 4=5
    if byte(data, nb) == byte(data, nb + 1) && byte(data, nb + 3) == byte(data, nb + 4) {
        if 10 > nl { nl = 10 }
    }
    // ABBCDD: 2=3 && 5=6
    if byte(data, nb + 1) == byte(data, nb + 2) && byte(data, nb + 4) == byte(data, nb + 5) {
        if 10 > nl { nl = 10 }
    }

    // ---- numberliteral: BE u64 fold, then the decimal digit string ----
    var num = 0u64
    var j = 0u64
    while j < 8 {
        num = num * 256 + byte(data, xb + j)
        j += 1
    }
    var nu = 0u64
    if num < 10 { nu = 30 }
    // decimal digits, ASCII, built by prepending
    var digs = ""
    var x = num
    if x == 0 { digs = "0" }
    while x > 0 {
        var d10 = x % 10
        var ch = "9"
        if d10 == 0 { ch = "0" }
        if d10 == 1 { ch = "1" }
        if d10 == 2 { ch = "2" }
        if d10 == 3 { ch = "3" }
        if d10 == 4 { ch = "4" }
        if d10 == 5 { ch = "5" }
        if d10 == 6 { ch = "6" }
        if d10 == 7 { ch = "7" }
        if d10 == 8 { ch = "8" }
        digs = ch ++ digs
        x = x / 10
    }
    var dn = size(digs)
    var bm3 = 0u64
    j = 0
    while j < dn {
        bm3 = bm3 | (1 << (byte(digs, j) - 48))
        j += 1
    }
    var px3 = bm3 & 65535
    px3 = px3 - ((px3 >> 1) & 21845)
    px3 = (px3 & 13107) + ((px3 >> 2) & 13107)
    px3 = (px3 + (px3 >> 4)) & 3855
    px3 = px3 + (px3 >> 8)
    var distinct3 = px3 & 31
    if distinct3 == 1 { if 50 > nu { nu = 50 } }
    // Serial num: strictly +1 or strictly -1 across the whole string
    var up = 1u64
    var down = 1u64
    j = 1
    while j < dn {
        var cur = byte(digs, j)
        var pre = byte(digs, j - 1)
        if cur <= pre { up = 0 } else { if cur - pre != 1 { up = 0 } }
        if cur >= pre { down = 0 } else { if pre - cur != 1 { down = 0 } }
        j += 1
    }
    if up == 1 || down == 1 { if 25 > nu { nu = 25 } }
    // Number Symmetry: palindrome
    var sym = 1u64
    j = 0
    while j < dn / 2 {
        if byte(digs, j) != byte(digs, dn - 1 - j) { sym = 0 }
        j += 1
    }
    if sym == 1 { if 10 > nu { nu = 10 } }
    // Tail runs (>= 4)
    var mrun = 1u64
    run = 1
    j = 1
    while j < dn {
        if byte(digs, j) == byte(digs, j - 1) {
            run += 1
            if run > mrun { mrun = run }
        } else {
            run = 1
        }
        j += 1
    }
    if mrun >= 4 {
        // DA: 7 -> 200, 6 -> 100, 5 -> 50, anything else (4 or >= 8) -> 20.
        if mrun == 7 { nu = 200 }
        if mrun == 6 { if 100 > nu { nu = 100 } }
        if mrun == 5 { if 50 > nu { nu = 50 } }
        if mrun < 5 { if 20 > nu { nu = 20 } }
        if mrun > 7 { if 20 > nu { nu = 20 } }
    }

    // ---- total = max(1, shape + colorish + nl + nu) ----
    var total = shape + colorish + nl + nu
    if total < 1 { total = 1 }
    sum += total
    i += 1
}
return sum as bytes"#;

    /* ---- HIP5 scorer: fitsh vs rust, Im vs Imc (RVC) ---- */

    /// Deterministic pseudo-random HIP5 wire: names[6n] || numbers[8n] ||
    /// visual_genes[10n], letters drawn from the 16-letter name alphabet.
    fn hip5_wire(n: usize) -> Vec<u8> {
        const LETTERS: &[u8] = b"WTYUIAHXVMEKBSZN";
        let mut st = 0x1234_5678_9abc_def0u64;
        let mut input = Vec::with_capacity(n * 24);
        for _ in 0..n {
            for _ in 0..6 {
                input.push(LETTERS[((st >> 8) % 16) as usize]);
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            }
            let num = st;
            input.extend_from_slice(&num.to_be_bytes());
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            for k in 0..10 {
                input.push(((st >> (k * 5)) & 0xFF) as u8);
            }
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        input
    }

    #[test]
    fn hip5_rvc_report() {
        let (ir, hvm) = compile_with_input(HIP5_FITSH);
        let (hvm_bytes, hvm_gas) = hvm_bytecode_stats(&hvm).unwrap();
        println!("HIP5 fitsh: hvm bytecode = {hvm_bytes} B, static gas = {hvm_gas}");

        let input = hip5_wire(200);
        assert_eq!(input.len(), 4800);
        let mut outs: Vec<Vec<u8>> = Vec::new();
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).unwrap();
            // The 40 KiB arena (ARENA_SIZE_MAX, the L1-layout maximum with
            // io 4800+8 under the 64 KiB cap) absorbs the digit-string churn
            // the bump allocator never reclaims — ~199 B per record for
            // u64-range numbers; the deployed rust kernel manages with
            // 14 KiB via a fixed digits buffer. A production backend wants
            // GC or slot regions instead of a bigger box.
            let cap = SpaceCap::new(0);
            let container = match mode {
                Rv64Mode::Im => lo.pack_segments(&cap, 64, 4800, 8),
                _ => lo.pack_segments_profile(
                    &cap,
                    64,
                    4800,
                    8,
                    crate::calc::container::CALC_PROFILE_RV64IMC,
                ),
            }
            .unwrap_or_else(|e| panic!("pack {mode:?}: {e}"));
            let fno = CalcFnObj::build_v1(container).unwrap();
            let exec =
                fno.run(&input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
            let out = match exec.result {
                Ok(o) => o,
                Err(e) => panic!("run {mode:?}: {e:?} exit={}", exec.exit_code),
            };
            println!(
                "HIP5 fitsh {mode:?}: code+pool = {} B (code {}), insts = {}, static cycles = {}, exec cycles = {} ({:.2}/record)",
                lo.code.len() + lo.pool.len(),
                lo.code.len(),
                lo.stats.insts,
                lo.stats.cycles,
                exec.cycles_used,
                exec.cycles_used as f64 / 200.0,
            );
            println!("  sum[{mode:?}] = {}", u64::from_be_bytes(out.as_slice().try_into().unwrap()));
            outs.push(out);
        }
        assert_eq!(outs[0], outs[1], "Imc output must equal Im");
        assert_eq!(outs[0], outs[2], "ImcRemap output must equal Im");
        println!("HIP5 fitsh sum (Im) = {}", u64::from_be_bytes(outs[0].as_slice().try_into().unwrap()));

        // Diff against the deployed rust kernel container (local-only fixture;
        // skipped when the .loc dir is absent so clean checkouts stay green).
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let rel = std::path::Path::new("doc/hacdsdefi4000.loc/out/score_sum.hcf1");
        let hcf1 = [base.join(rel), base.join("../..").join(rel), base.join("..").join(rel)]
            .into_iter()
            .find(|p| p.exists());
        let bytes = match hcf1 {
            Some(p) => std::fs::read(p).ok(),
            None => {
                println!("rust container fixture not present, diff skipped");
                None
            }
        };
        if let Some(bytes) = bytes {
            let (c, _) = crate::calc::container::CalcContainer::decode(&bytes).unwrap();
            let fno = CalcFnObj::build_v1(c).unwrap();
            let exec =
                fno.run(&input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
            let rust_out = match exec.result {
                Ok(o) => o,
                Err(e) => panic!("rust kernel: {e:?} exit={}", exec.exit_code),
            };
            println!(
                "HIP5 rust kernel (deployed, profile 0): out = {}, exec cycles = {} ({:.2}/record)",
                u64::from_be_bytes(rust_out.as_slice().try_into().unwrap()),
                exec.cycles_used,
                exec.cycles_used as f64 / 200.0,
            );
            assert_eq!(outs[0], rust_out, "fitsh scorer must reproduce the rust kernel output");

            // The rust kernel rebuilt at profile 2 (riscv64imac): must
            // decode under RVC and produce the same sum.
            let rel2 = std::path::Path::new("doc/hacdsdefi4000.loc/out_rvc/score_sum.hcf1");
            let rvc_path = [
                base.join(rel2),
                base.join("..").join(rel2),
                base.join("../..").join(rel2),
            ]
            .into_iter()
            .find(|p| p.exists());
            if let Some(p) = rvc_path {
                let (c, _) =
                    crate::calc::container::CalcContainer::decode(&std::fs::read(p).unwrap())
                        .unwrap();
                assert_eq!(c.profile, crate::calc::container::CALC_PROFILE_RV64IMC);
                let fno = CalcFnObj::build_v1(c).unwrap();
                let exec = fno
                    .run(&input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
                let rvc_out = match exec.result {
                    Ok(o) => o,
                    Err(e) => panic!("rust rvc kernel: {e:?} exit={}", exec.exit_code),
                };
                assert_eq!(rvc_out, rust_out, "profile-2 rust kernel must match profile-0");
                println!(
                    "HIP5 rust kernel (profile 2, rvc): out = {}, exec cycles = {} ({:.2}/record)",
                    u64::from_be_bytes(rvc_out.as_slice().try_into().unwrap()),
                    exec.cycles_used,
                    exec.cycles_used as f64 / 200.0,
                );
            } else {
                println!("rust profile-2 fixture not present, rvc diff skipped");
            }
        }
    }

    /// Residue diagnosis: every 4-byte br-zero / j site in the fitsh
    /// ImcRemap stream, with the offending register, plus the slot ld/sd
    /// register census (window vs scratch vs full-width).
    #[test]
    fn fwd_mode_differential_hunt() {
        // bodies exercising PUT→GET forwarding, XOP, banking shapes
        #[allow(clippy::type_complexity)]
        let bodies: Vec<(&str, u64)> = vec![
            ("var s = 0u64\nvar i = 0u64\nwhile i < 10 { s += i\ni += 1 }\nreturn s as bytes", 45),
            ("var s = 0u64\nvar t = 5u64\nvar i = 0u64\nwhile i < 8 { t = t + i\ns += t\ni += 1 }\nreturn (s + t) as bytes", 157),
            ("var a = 3u64\nvar b = 4u64\nvar c = a + b\nvar d = c * 2\nreturn (c + d) as bytes", 21),
            ("var x = 10u64\nvar y = x + 5\nx = y * 2\nreturn (x + y) as bytes", 45),
            ("var s = 0u64\nvar i = 0u64\nwhile i < 6 { var t = i * i\ns += t\ni += 1 }\nreturn s as bytes", 55),
            ("var s = 0u64\nvar i = 0u64\nwhile i < 5 { if i % 2 == 0 { s += i }\ni += 1 }\nreturn s as bytes", 6),
            ("var p = 0x0011223344556677\nvar q = p\nvar r = byte(q, 2)\nreturn (r + size(p)) as bytes", 0x22 + 8),
            ("var unused = 42u64\nvar keep = 7u64\nreturn keep as bytes", 7),
            ("var s = 0u64\nvar i = 0u64\nwhile i < 4 { var a = i + 1\nvar b = a * 3\nvar c = a + b\ns += c\ni += 1 }\nreturn s as bytes", 40),
            ("var m = 0u64\nvar i = 0u64\nwhile i < 5 { if i > m { m = i }\ni += 1 }\nreturn m as bytes", 4),
            ("var m = 100u64\nvar i = 0u64\nwhile i < 5 { if 200 > m { m = 200 }\nif m < 150 { m = 150 }\ni += 1 }\nreturn m as bytes", 200),
            ("var m = 0u64\nvar n = 7u64\nif n > m { m = n }\nreturn m as bytes", 7),
            ("var a = 3u64\nvar b = 9u64\nvar m = 0u64\nwhile a < b { if a > m { m = a }\na += 2 }\nreturn m as bytes", 7),
            ("var lo = 1000u64\nvar i = 0u64\nwhile i < 4 { if i < lo { lo = i }\ni += 1 }\nreturn lo as bytes", 0),
        ];
        // the oracle is the source semantics (pinned constants), not a
        // sibling mode — a shared bug would hide from a mode-vs-mode diff
        for (bi, (body, want)) in bodies.iter().enumerate() {
            let out_im = run_e2e_mode(body, &[], Rv64Mode::Im);
            assert_eq!(
                out_im, want.to_be_bytes(),
                "body #{bi} Im wrong: {body}"
            );
            for mode in [Rv64Mode::Imc, Rv64Mode::ImcRemap] {
                let out = run_e2e_mode(body, &[], mode);
                assert_eq!(
                    out, want.to_be_bytes(),
                    "body #{bi} mode {mode:?} wrong: {body}"
                );
            }
            println!("body #{bi} ok = {out_im:x?}");
        }
    }

    /// Disassemble body #9 of the differential hunt (the failing max/min
    /// shape) for manual inspection.
    /// Value-position if/choose executes correctly end to end (the
    /// historically-unimplemented eval path).
    #[test]
    fn value_if_e2e() {
        let body = "var a = byte(data, 0)\nvar r = if a != 0 { 1 } else { 0 }\nreturn r as bytes";
        let out = run_input(body, &[5]);
        assert_eq!(out, 1u64.to_be_bytes());
        let out0 = run_input(body, &[0]);
        assert_eq!(out0, 0u64.to_be_bytes());
        // nested value-if and a bytes-typed arm
        let nested = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = if a > b { if a > 100 { 2 } else { 3 } } else { 4 }\nreturn r as bytes";
        assert_eq!(run_input(nested, &[200, 5]), 2u64.to_be_bytes());
        assert_eq!(run_input(nested, &[50, 5]), 3u64.to_be_bytes());
        assert_eq!(run_input(nested, &[1, 5]), 4u64.to_be_bytes());
        // a Ref-valued arm (bare variable) materializes into the shared
        // join register
        let refarm = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = if a > b { a } else { b }\nreturn r as bytes";
        assert_eq!(run_input(refarm, &[9, 4]), 9u64.to_be_bytes());
        assert_eq!(run_input(refarm, &[4, 9]), 9u64.to_be_bytes());
    }

    #[test]
    fn pool_const_repro() {
        for v in [2048u64, 13107, 21845, 1000000007, 1_000_000_000_000] {
            // ≥4 literal sites activate the pool-base register
            let body = format!(
                "var a = 0x11\nvar b = 0x22\nvar c = a ++ b\nvar d = c ++ a\nvar x = {v}u64\nvar y = x % 65537\nreturn (x + y + size(c) + size(d)) as bytes"
            );
            let body = body.replace("\\n", "\n");
            let (ir, _) = compile(&body);
            let lo = lower_body_mode_arena(&ir, false, Rv64Mode::ImcRemap, ARENA_SIZE).unwrap();
            let cap = SpaceCap::new(0);
            let fno = CalcFnObj::build_v1(
                lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC)
                    .unwrap(),
            )
            .unwrap();
            let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
            match exec.result {
                Ok(o) => {
                    let got = u64::from_be_bytes(o.as_slice().try_into().unwrap());
                    println!("pool const {v}: got {got} {}",
                        if got == v + (v % 65537) + 2 + 3 { "OK" } else { "WRONG" });
                    assert_eq!(got, v + (v % 65537) + 2 + 3, "pool const {v} wrong");
                }
                Err(e) => panic!("pool const {v}: {e:?} exit={}", exec.exit_code),
            }
        }
    }

    #[test]
    fn hip5_residue_diagnosis() {
        let (ir, _) = compile_with_input(HIP5_FITSH);
        let lo = lower_body_mode_arena(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
        let profile = crate::calc::container::CALC_PROFILE_RV64IMC;
        let mut off = 0usize;
        let mut brz4 = 0;
        let mut j4 = 0;
        while off < lo.code.len() {
            let (inst, len) = decode_at(&lo.code, off, profile).unwrap();
            let l = len as usize;
            match &inst {
                Inst::Branch { rs2: 0, rs1, .. } if l == 4 => {
                    brz4 += 1;
                    println!("brz4 @{off:04x}: {inst:?}");
                }
                Inst::Jal { rd: 0, .. } if l == 4 => {
                    j4 += 1;
                    println!("j4   @{off:04x}: {inst:?}");
                }
                _ => {}
            }
            off += l;
        }
        println!("total 4B br-zero = {brz4}, 4B j = {j4}");

        // slot ld/sd census by (rs1 base, offset) → approximate slot identity
        let mut mem: std::collections::BTreeMap<(u8, i64), (usize, usize)> = Default::default();
        let mut off = 0usize;
        while off < lo.code.len() {
            let (inst, len) = decode_at(&lo.code, off, profile).unwrap();
            let l = len as usize;
            match &inst {
                Inst::Load { op: LoadOp::Ld, rs1, imm, .. } => {
                    let e = mem.entry((*rs1, *imm)).or_default();
                    e.0 += 1;
                    e.1 += l;
                }
                Inst::Store { op: StoreOp::Sd, rs1, imm, .. } => {
                    let e = mem.entry((*rs1, *imm)).or_default();
                    e.0 += 1;
                    e.1 += l;
                }
                _ => {}
            }
            off += l;
        }
        println!("-- slot ld/sd by (base,off) --");
        for ((b, o), (n, by)) in &mem {
            println!("  x{b} {o:>4}: {n:>3} inst, {by} B");
        }
    }

    /* ---- per-instruction census: fitsh stream vs rust profile-2 stream ---- */

    /// Machine category of one decoded instruction (stream-agnostic).
    fn census_cat(i: &Inst) -> &'static str {
        use crate::calc::decode::{ImmAluOp::*, LoadOp::*, RegAluOp::*, StoreOp::*};
        match i {
            Inst::Lui { .. } => "const/lui",
            Inst::Auipc { .. } => "auipc",
            Inst::ImmAlu { op: Addi, rs1: 0, .. } => "const/li",
            Inst::ImmAlu { op: Addi, imm: 0, rs1, rd } if rd == rs1 && *rs1 != 0 => "mv",
            Inst::RegAlu { op: Add, rs1: 0, .. } => "mv", // c.mv
            Inst::ImmAlu { .. } => "alu-imm",
            Inst::RegAlu { op, .. }
                if matches!(op, Mul | Mulh | Mulhsu | Mulhu | Div | Divu | Rem | Remu) =>
            {
                "mul-div"
            }
            Inst::RegAlu { op, .. } if matches!(op, Slt | Sltu) => "setbb",
            Inst::RegAlu { .. } => "alu-reg",
            Inst::ShiftImm { .. } => "shift",
            Inst::Load { op: Ld, .. } => "ld",
            Inst::Load { op: Lbu, .. } => "lbu",
            Inst::Load { .. } => "load",
            Inst::Store { op: Sd, .. } => "sd",
            Inst::Store { op: Sb, .. } => "sb",
            Inst::Store { .. } => "store",
            Inst::Branch { rs2: 0, .. } => "br-zero",
            Inst::Branch { .. } => "br",
            Inst::Jal { rd: 0, .. } => "j",
            Inst::Jal { .. } => "jal-link",
            Inst::Jalr { .. } => "jr",
            Inst::Ecall => "ecall",
            _ => "other",
        }
    }

    /// Decode a stream and aggregate (count, bytes, compressed-2B count)
    /// per category. Returns rows sorted by fitsh-bycategory order of first
    /// appearance; caller prints.
    fn census_stream(
        code: &[u8],
        profile: u8,
    ) -> Vec<(&'static str, usize, usize, usize)> {
        let mut map: std::collections::BTreeMap<&'static str, (usize, usize, usize)> =
            Default::default();
        let mut off = 0usize;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, profile).unwrap();
            let e = map.entry(census_cat(&inst)).or_default();
            e.0 += 1;
            e.1 += len as usize;
            e.2 += (len == 2) as usize;
            off += len as usize;
        }
        map.into_iter().map(|(k, (n, b, c))| (k, n, b, c)).collect()
    }

    /// The rust kernel's RX code bytes (profile-2 fixture), for the census.
    fn rust_rvc_code() -> Option<Vec<u8>> {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let rel = std::path::Path::new("doc/hacdsdefi4000.loc/out_rvc/score_sum.hcf1");
        let p = [base.join(rel), base.join("../..").join(rel), base.join("..").join(rel)]
            .into_iter()
            .find(|p| p.exists())?;
        let bytes = std::fs::read(p).ok()?;
        let (c, _) = crate::calc::container::CalcContainer::decode(&bytes).ok()?;
        let seg = c
            .segs
            .iter()
            .find(|s| s.vaddr == crate::calc::container::CODE_BASE as u32)?;
        Some(seg.data.clone())
    }

    /// Static per-slot access counts + peak expression depth for HIP5
    /// (drives local-promotion bank sizing / prioritization).
    #[test]
    fn and_ir_shape_probe() {
        let body = "var a = 1u64\nvar b = 2u64\nvar c = 3u64\nvar x = 0u64\nif a == b && b == c { x = 1 }\nreturn x as bytes";
        let (ir, _) = compile(body);
        fn dump(n: &dyn IRNode, depth: usize) {
            let op = Bytecode::try_from_u8(n.bytecode()).unwrap();
            println!("{:indent$}{op:?}", "", indent = depth * 2);
            for c in children(n) {
                dump(c, depth + 1);
            }
        }
        for n in &ir.subs {
            dump(&**n, 0);
        }
    }

    #[test]
    fn hip5_depth_and_slots() {
        let (ir, _) = compile_with_input(HIP5_FITSH);
        let fp = plan(&ir.subs, true).unwrap();
        let mut uses = vec![0usize; fp.slot.len()];
        let mut writes = vec![0usize; fp.slot.len()];
        fn walkc(n: &dyn IRNode, uses: &mut Vec<usize>, writes: &mut Vec<usize>) {
            if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
                return walkc(&*w.node, uses, writes);
            }
            let op = Bytecode::try_from_u8(n.bytecode()).unwrap();
            match op {
                Bytecode::GET | Bytecode::GET0 | Bytecode::GET1 | Bytecode::GET2 | Bytecode::GET3 => {
                    let idx = match op {
                        Bytecode::GET => {
                            n.as_any().downcast_ref::<IRNodeParam1>().unwrap().para as usize
                        }
                        Bytecode::GET0 => 0,
                        Bytecode::GET1 => 1,
                        Bytecode::GET2 => 2,
                        _ => 3,
                    };
                    uses[idx] += 1;
                }
                Bytecode::PUT => {
                    let p = n.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    writes[p.para as usize] += 1;
                }
                Bytecode::XOP => {
                    let p = n.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    let (_, idx) = decode_local_operand_mark(p.para);
                    uses[idx as usize] += 1;
                    writes[idx as usize] += 1;
                }
                _ => {}
            }
            for c in children(n) {
                walkc(c, uses, writes);
            }
        }
        for n in &ir.subs {
            walkc(&**n, &mut uses, &mut writes);
        }
        println!("slots = {}", fp.slot.len());
        for (i, ((s, u), w)) in fp.slot.iter().zip(&uses).zip(&writes).enumerate() {
            if *u + *w > 0 {
                println!("  slot {i:>2} {:?}: reads+writes = {}", s, u + w);
            }
        }
        let mut lw = Lower::new_mode_arena(
            &fp.slot,
            &fp.needs_zero,
            Rv64Mode::ImcRemap,
            ARENA_SIZE_MAX,
        );
        lw.emit_prologue_body_epilogue(&ir.subs).unwrap();
        println!("HIP5 peak vstack depth = {}", lw.max_depth);
    }

    #[test]
    fn hip5_census_report() {
        let (ir, _) = compile_with_input(HIP5_FITSH);
        let lo = lower_body_mode_arena(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).unwrap();
        let fitsh = census_stream(&lo.code, crate::calc::container::CALC_PROFILE_RV64IMC);
        println!("\n== fitsh ImcRemap code = {} B ==", lo.code.len());
        println!("{:<12} {:>5} {:>7} {:>5}", "cat", "n", "bytes", "2B");
        let mut tn = 0;
        let mut tb = 0;
        for (k, n, b, c) in &fitsh {
            println!("{:<12} {:>5} {:>7} {:>5}", k, n, b, c);
            tn += n;
            tb += b;
        }
        println!("{:<12} {:>5} {:>7}", "TOTAL", tn, tb);
        assert_eq!(tb, lo.code.len());

        if let Some(rcode) = rust_rvc_code() {
            let rust = census_stream(&rcode, crate::calc::container::CALC_PROFILE_RV64IMC);
            println!("\n== rust profile-2 code = {} B ==", rcode.len());
            let mut tn = 0;
            let mut tb = 0;
            for (k, n, b, c) in &rust {
                println!("{:<12} {:>5} {:>7} {:>5}", k, n, b, c);
                tn += n;
                tb += b;
            }
            println!("{:<12} {:>5} {:>7}", "TOTAL", tn, tb);
            assert_eq!(tb, rcode.len());

            // side-by-side delta
            println!("\n== delta (fitsh - rust) by category ==");
            println!("{:<12} {:>7} {:>7} {:>8}", "cat", "fitsh", "rust", "deltaB");
            let mut keys: Vec<&str> = fitsh.iter().map(|r| r.0).collect();
            for (k, _, _, _) in &rust {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            keys.sort_unstable();
            for k in keys {
                let f = fitsh.iter().find(|r| r.0 == k).map(|r| r.2).unwrap_or(0);
                let r = rust.iter().find(|r| r.0 == k).map(|r| r.2).unwrap_or(0);
                println!("{:<12} {:>7} {:>7} {:>8}", k, f, r, f as i64 - r as i64);
            }
        } else {
            println!("rust fixture missing; fitsh-only census");
        }

        // fitsh-side cause refinements (register/target heuristics), v2
        // (P1-G): guard control transfer into the shared fault thunks is
        // peeled out of the br / br-zero / j form buckets into its own
        // semantic bucket (it used to be split by match-arm order: rs2==0
        // guard branches landed in "forward 4B br-zero" or were dropped
        // entirely, and the r==limit c.j fast path landed in "j"). The
        // thunk entries are located by their tail `li a0, <exit code>`
        // immediates inside the final 32 B — the old fixed 7-instruction
        // walkback landed 2 B early because the compressed tail has 8
        // instructions. Pure measurement: nothing here feeds the emitter.
        let profile = crate::calc::container::CALC_PROFILE_RV64IMC;
        let code = &lo.code;
        let mut offs = Vec::new();
        let mut insts = Vec::new();
        let mut lens = Vec::new();
        {
            let mut p = 0usize;
            while p < code.len() {
                let (i, l) = decode_at(code, p, profile).unwrap();
                offs.push(p);
                insts.push(i);
                lens.push(l as usize);
                p += l as usize;
            }
        }
        assert_eq!(
            insts.last(),
            Some(&Inst::Ecall),
            "ecall must close the fitsh stream"
        );
        let tail_lo = code.len().saturating_sub(32);
        let thunk_li = |imm_want: i64| -> usize {
            let mut found = None;
            for i in 0..insts.len() {
                if offs[i] < tail_lo {
                    continue;
                }
                if let Inst::ImmAlu { op: ImmAluOp::Addi, rd: 10, rs1: 0, imm } = &insts[i] {
                    if *imm == imm_want {
                        found = Some(offs[i]);
                    }
                }
            }
            found.unwrap_or_else(|| panic!("thunk li a0, {imm_want} not found in the final 32 B"))
        };
        let thunk_arith = thunk_li(EXIT_ARITH as i64);
        let thunk_cast = thunk_li(EXIT_CAST as i64);
        assert!(thunk_arith < thunk_cast, "thunk tail layout changed");
        // GT-a copy thunks: `li a0,{90,71}` strictly below the shared-thunk
        // region, followed by a jump into a shared thunk (c.j form) or into
        // the trap tail (jal form). No other stream shape emits that pair.
        let mut copy_of: std::collections::BTreeMap<usize, i64> = Default::default();
        for i in 0..insts.len() {
            if offs[i] >= thunk_arith {
                break;
            }
            let code = match &insts[i] {
                Inst::ImmAlu { op: ImmAluOp::Addi, rd: 10, rs1: 0, imm }
                    if *imm == EXIT_ARITH as i64 || *imm == EXIT_CAST as i64 =>
                {
                    *imm
                }
                _ => continue,
            };
            let jump_ok = i + 1 < insts.len()
                && match &insts[i + 1] {
                    Inst::Jal { rd: 0, target, .. } => {
                        let abs = offs[i + 1] as i64 + target;
                        abs == thunk_arith as i64
                            || abs == thunk_cast as i64
                            || (lens[i + 1] == 4 && abs > thunk_cast as i64)
                    }
                    _ => false,
                };
            if jump_ok {
                copy_of.insert(offs[i], code);
            }
        }
        let mut slot_n = 0usize;
        let mut slot_b = 0usize;
        let mut arena_sb_n = 0usize;
        let mut arena_sb_b = 0usize;
        let mut guard_n = 0usize;
        let mut guard_b = 0usize;
        let mut guard_arith = (0usize, 0usize);
        let mut guard_cast = (0usize, 0usize);
        let mut guard_br_rs2nz = (0usize, 0usize);
        let mut guard_br_rs2z = (0usize, 0usize);
        let mut guard_jal = (0usize, 0usize);
        let mut guard_2b = (0usize, 0usize);
        // P3-GUARDTAIL GT-a: copy thunks (`li a0,<code>` + jump) inserted so
        // clustered zero-form guards reach a 2-byte c.bz; the copies ARE
        // guard machinery and the converted sites still guard — both are
        // counted in the guard totals above, this line reports the split.
        let mut guard_copy = (0usize, 0usize); // copy thunks: n, bytes
        let mut guard_copy_cj = 0usize; // copies taking the c.j tail form
        let mut guard_conv = (0usize, 0usize); // converted sites: n, bytes
        let mut brfwd4_z_all = (0usize, 0usize); // forward br-zero, 4B, incl. guards
        let mut brfwd4_z_guard = (0usize, 0usize);
        let mut brfwd4_z_win = 0usize; // forward non-guard 4B br-zero in c.beqz window
        let mut jfwd4_n = 0usize; // forward j, 4B (could be c.j)
        let mut jfwd4_b = 0usize;
        let mut ng_br = (0usize, 0usize); // non-guard br (rs2 != 0, not thunk-bound)
        let mut ng_brz = (0usize, 0usize); // non-guard br-zero
        let mut ng_brz_2b = 0usize;
        let mut li_hist: std::collections::BTreeMap<i64, usize> = Default::default();
        for i in 0..insts.len() {
            let off_i = offs[i];
            let len = lens[i];
            let abs_t =
                |t: i64| (off_i as i64 + t) as usize;
            match &insts[i] {
                Inst::Load { op: LoadOp::Ld, rs1: 8, .. }
                | Inst::Store { op: StoreOp::Sd, rs1: 8, .. } => {
                    slot_n += 1;
                    slot_b += len;
                }
                Inst::Store { op: StoreOp::Sb, rs1: 9, .. } => {
                    arena_sb_n += 1;
                    arena_sb_b += len;
                }
                Inst::Branch { target, rs2, .. } => {
                    let abs = abs_t(*target);
                    let conv_code = copy_of.get(&(abs as usize)).copied();
                    let guard = abs >= thunk_arith || conv_code.is_some();
                    if guard {
                        guard_n += 1;
                        guard_b += len;
                        if conv_code == Some(EXIT_ARITH as i64)
                            || (conv_code.is_none() && abs < thunk_cast)
                        {
                            guard_arith.0 += 1;
                            guard_arith.1 += len;
                        } else {
                            guard_cast.0 += 1;
                            guard_cast.1 += len;
                        }
                        if conv_code.is_some() {
                            guard_conv.0 += 1;
                            guard_conv.1 += len;
                        }
                        if *rs2 == 0 {
                            guard_br_rs2z.0 += 1;
                            guard_br_rs2z.1 += len;
                        } else {
                            guard_br_rs2nz.0 += 1;
                            guard_br_rs2nz.1 += len;
                        }
                        if len == 2 {
                            guard_2b.0 += 1;
                            guard_2b.1 += len;
                        }
                    } else if *rs2 == 0 {
                        ng_brz.0 += 1;
                        ng_brz.1 += len;
                        if len == 2 {
                            ng_brz_2b += 1;
                        }
                        if len == 4 && *target > 4 {
                            brfwd4_z_all.0 += 1;
                            brfwd4_z_all.1 += 4;
                            if (-256..=254).contains(target) {
                                brfwd4_z_win += 1;
                            }
                        }
                    } else {
                        ng_br.0 += 1;
                        ng_br.1 += len;
                    }
                    if len == 4 && *target > 4 && abs >= thunk_arith && *rs2 == 0 {
                        brfwd4_z_guard.0 += 1;
                        brfwd4_z_guard.1 += 4;
                    }
                }
                Inst::Jal { rd: 0, target, .. } => {
                    let abs = abs_t(*target);
                    if copy_of.contains_key(&(off_i.saturating_sub(4))) {
                        // GT-a copy thunk tail: the li counted the copy's
                        // head; count the jump half here
                        let code = copy_of[&(off_i - 4)];
                        guard_n += 1;
                        guard_b += len;
                        guard_copy.0 += 1;
                        guard_copy.1 += len;
                        if code == EXIT_ARITH as i64 {
                            guard_arith.0 += 1;
                            guard_arith.1 += len;
                        } else {
                            guard_cast.0 += 1;
                            guard_cast.1 += len;
                        }
                        if len == 2 {
                            guard_2b.0 += 1;
                            guard_2b.1 += len;
                            guard_copy_cj += 1;
                        }
                    } else if off_i < thunk_arith && abs >= thunk_arith {
                        // external jump into the trap tail: the r == limit
                        // cast fast path (the site preloads a0 itself)
                        guard_n += 1;
                        guard_b += len;
                        guard_cast.0 += 1;
                        guard_cast.1 += len;
                        guard_jal.0 += 1;
                        guard_jal.1 += len;
                        guard_2b.0 += 1;
                        guard_2b.1 += len;
                    } else if *target > 4 && len == 4 {
                        jfwd4_n += 1;
                        jfwd4_b += 4;
                    }
                }
                Inst::ImmAlu { op: ImmAluOp::Addi, rs1: 0, imm, .. }
                | Inst::Lui { val: imm, .. } => {
                    *li_hist.entry(*imm).or_insert(0) += 1;
                    if let Some(&code) = copy_of.get(&off_i) {
                        // GT-a copy head: the jump half is counted in the
                        // Jal arm; the head bytes land in the guard total
                        guard_n += 1;
                        guard_b += len;
                        guard_copy.0 += 1;
                        guard_copy.1 += len;
                        if code == EXIT_ARITH as i64 {
                            guard_arith.0 += 1;
                            guard_arith.1 += len;
                        } else {
                            guard_cast.0 += 1;
                            guard_cast.1 += len;
                        }
                    }
                }
                _ => {}
            }
        }
        // cross-checks against the raw form table (guards must be exactly
        // the difference between the raw and peeled br/br-zero buckets)
        let (raw_br, raw_brz) = {
            let t = census_stream(code, profile);
            (
                t.iter().find(|r| r.0 == "br").map(|r| (r.1, r.2)).unwrap_or((0, 0)),
                t.iter().find(|r| r.0 == "br-zero").map(|r| (r.1, r.2)).unwrap_or((0, 0)),
            )
        };
        assert_eq!(
            ng_br.0 + guard_br_rs2nz.0,
            raw_br.0,
            "guard peel must cover all br"
        );
        assert_eq!(
            ng_brz.0 + guard_br_rs2z.0,
            raw_brz.0,
            "guard peel must cover all br-zero"
        );
        println!("\n== fitsh causes (v2, guard-corrected) ==");
        println!("slot ld/sd (base s0):        {slot_n:>4} inst, {slot_b:>5} B");
        println!("arena sb (base s1):          {arena_sb_n:>4} inst, {arena_sb_b:>5} B");
        println!(
            "guard → thunks:              {guard_n:>4} inst, {guard_b:>5} B \
             (arith→{EXIT_ARITH} {} inst/{} B, cast→{EXIT_CAST} {} inst/{} B)",
            guard_arith.0, guard_arith.1, guard_cast.0, guard_cast.1
        );
        println!(
            "  guard copies (GT-a):       {:>4} inst, {:>5} B \
             ({} copy thunks, {} c.j-form; {} converted 2B c.bz sites {}/{} B)",
            guard_copy.0 + guard_conv.0,
            guard_copy.1 + guard_conv.1,
            guard_copy.0 / 2,
            guard_copy_cj,
            guard_conv.0,
            guard_conv.0,
            guard_conv.1
        );
        println!(
            "  branch-form guards:        {:>4} inst, {:>5} B \
             (rs2≠0 {} inst/{} B, rs2==0 {} inst/{} B)",
            guard_br_rs2nz.0 + guard_br_rs2z.0,
            guard_br_rs2nz.1 + guard_br_rs2z.1,
            guard_br_rs2nz.0,
            guard_br_rs2nz.1,
            guard_br_rs2z.0,
            guard_br_rs2z.1
        );
        println!(
            "  jal fast-path guards:      {:>4} inst, {:>5} B \
             (r==limit cast, site preloads a0)",
            guard_jal.0, guard_jal.1
        );
        println!(
            "  2B-encoded guards:         {:>4} inst, {:>5} B \
             (c.beqz-in-window + c.j fast path)",
            guard_2b.0, guard_2b.1
        );
        println!(
            "forward 4B br-zero:          {:>4} inst, {:>5} B total \
             (guard {}/{} B, non-guard {}/{} B, non-guard in c.beqz window {})",
            brfwd4_z_all.0 + brfwd4_z_guard.0,
            brfwd4_z_all.1 + brfwd4_z_guard.1,
            brfwd4_z_guard.0,
            brfwd4_z_guard.1,
            brfwd4_z_all.0,
            brfwd4_z_all.1,
            brfwd4_z_win
        );
        println!(
            "forward 4B j (non-guard):    {:>4} inst, {:>5} B (c.j → 2B each)",
            jfwd4_n, jfwd4_b
        );
        println!(
            "non-guard branch forms:      br {} inst/{} B; br-zero {} inst/{} B ({} × 2B)",
            ng_br.0, ng_br.1, ng_brz.0, ng_brz.1, ng_brz_2b
        );
        let mut hists: Vec<(i64, usize)> = li_hist.into_iter().collect();
        hists.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("top const/li immediates (remat candidates):");
        for (v, n) in hists.iter().take(12) {
            println!("  li {v:>6} × {n}");
        }

        // ---- rust-side denominators for the P1-G attribution table v2 ----
        // checked-profile / checked-source .text bins from the b1 emit
        // build (gitignored artifacts, regenerated by
        // hacdtestnet-b1:b1-scratch/build_emit.sh; byte-identical to the
        // B1 baselines per 60-b2 §Method). Printed when present so the v2
        // table is reproducible from this census alone; skipped otherwise.
        for name in ["prof", "src"] {
            let dir = std::env::var("RV64_B1_EMIT_DIR").unwrap_or_else(|_| {
                "/media/jji/data/repo/hacashdiamonds/hacdtestnet-b1/b1-scratch/out-emit"
                    .to_string()
            });
            let Ok(rcode) = std::fs::read(format!("{dir}/{name}.text.bin")) else {
                println!(
                    "\n(rust {name} .text bin not found under {dir}; \
                     b1 emit artifacts missing — rust-side v2 columns skipped)"
                );
                continue;
            };
            let rust = census_stream(&rcode, crate::calc::container::CALC_PROFILE_RV64IMC);
            println!("\n== rust {name} .text = {} B ==", rcode.len());
            let mut tn = 0;
            let mut tb = 0;
            for (k, n, b, c) in &rust {
                println!("{:<12} {:>5} {:>7} {:>5}", k, n, b, c);
                tn += n;
                tb += b;
            }
            println!("{:<12} {:>5} {:>7}", "TOTAL", tn, tb);
            assert_eq!(tb, rcode.len());
            // branch audit: far-forward branches (disp >= 100) — the
            // trap-stub-bound guard set clusters here — plus multi-inbound
            // branch targets (stub blocks have many inbound edges).
            let mut p = 0usize;
            let mut rows: Vec<(i64, usize, bool, usize, usize, usize, usize)> = Vec::new();
            let mut targets: std::collections::BTreeMap<usize, (usize, usize)> = Default::default();
            while p < rcode.len() {
                let (inst, len) =
                    decode_at(&rcode, p, crate::calc::container::CALC_PROFILE_RV64IMC).unwrap();
                if let Inst::Branch { rs2, target, .. } = &inst {
                    let abs = (p as i64 + *target) as usize;
                    let e = targets.entry(abs).or_default();
                    e.0 += 1;
                    e.1 += len as usize;
                    if *target >= 100 {
                        rows.push((
                            *target,
                            p,
                            *rs2 == 0,
                            len as usize,
                            abs,
                            e.0,
                            0,
                        ));
                    }
                }
                p += len as usize;
            }
            rows.sort_unstable();
            println!("far-forward branches (disp >= 100):");
            for (disp, off, z, len, abs, _, _) in &rows {
                println!(
                    "  @{off:04x} {} {len}B disp {disp:+>5} -> {abs:#05x}",
                    if *z { "br-zero" } else { "br" }
                );
            }
            println!("branch targets with >= 2 inbound edges:");
            for (t, (n, b)) in &targets {
                if *n >= 2 {
                    println!("  -> {t:#05x}: {n} inbound, {b} B");
                }
            }
        }
    }

    #[test]
    fn pa_repro() {
        // exact HIP5 nc-table shape (16 arms, keys 65..90, holes = default 0)
        const LETTERS: [(u64, u64); 16] = [
            (65, 7), (66, 6), (69, 1), (72, 8), (73, 8), (75, 8), (77, 2), (78, 6),
            (83, 6), (84, 8), (85, 3), (86, 8), (87, 3), (88, 5), (89, 3), (90, 10),
        ];
        let mut chain = String::new();
        for (k, v) in LETTERS {
            chain.push_str(&format!("if x == {k}u64 {{ sel = {v}u64 }}\n"));
        }
        for (x, want) in [(65u64, 7u64), (90, 10), (66, 6), (67, 0), (64, 0), (91, 0), (80, 0)] {
            // x declared FIRST so `var sel = 0` directly precedes the run
            // and the table path (not the chain fallback) is exercised
            let body = format!(
                "var x = {x}u64\nvar sel = 0u64\n{chain}return sel as bytes"
            );
            let out = run_e2e_mode(&body, &[], Rv64Mode::Im);
            println!("x={x} want={want} got={:?}", u64::from_be_bytes(out.as_slice().try_into().unwrap()));
            assert_eq!(out, want.to_be_bytes().to_vec(), "x={x}");
        }
    }

    #[test]
    fn deep_expr_probe() {
        // The register pool is a hard subset limit: depth <= 8 computes
        // correctly; deeper nesting is rejected at compile time (the old
        // spill path silently miscomputed at depth >= 10 — caught here).
        for depth in [7usize, 8] {
            let (body, want) = deep_add_body(depth);
            let out = run_e2e(&body, &[]);
            assert_eq!(out, want.to_be_bytes().to_vec(), "depth {depth}");
        }
        for depth in [9usize, 16] {
            let (body, _) = deep_add_body(depth);
            let (ir, _) = compile(&body);
            let err = lower_body(&ir).unwrap_err().to_string();
            assert!(err.contains("8-value register pool"), "depth {depth}: {err}");
        }
    }

    fn deep_add_body(depth: usize) -> (String, u64) {
        let mut body = String::new();
        let mut want = 0u64;
        for i in 0..depth {
            body.push_str(&format!("var v{i} = {}u64\n", i + 1));
            want += (i + 1) as u64;
        }
        let mut expr = String::new();
        for i in (0..depth).rev() {
            expr = if i == depth - 1 {
                format!("v{i}")
            } else {
                format!("v{i} + ({expr})")
            };
        }
        body.push_str(&format!("return ({expr}) as bytes"));
        (body, want)
    }

    #[test]
    fn rejects_outside_subset() {
        let (ir, _) = compile("return 1 as u128");
        let err = lower_body(&ir).unwrap_err().to_string();
        assert!(err.contains("CU128"), "{err}");

        let (ir, _) = compile("log(1, 2)");
        let err = lower_body(&ir).unwrap_err().to_string();
        assert!(err.contains("outside the translatable kernel subset"), "{err}");

        // P3-V2IMPL: in-shape list literals lower; the ESCAPE (returning
        // the list) still rejects, naming the consuming opcode.
        let (ir, _) = compile("var b = [1, 2]\nreturn b");
        let err = lower_body(&ir).unwrap_err().to_string();
        assert!(err.contains("RET"), "{err}");
        assert!(err.contains("outside the translatable kernel subset"), "{err}");
    }

    #[test]
    fn lowers_core_constructs() {
        lower("var s = 0u64\nvar i = 0u64\nwhile i < 100 { s += i\ni += 1 }\nreturn s as bytes");
        lower("var b = 0x00112233445566778899aabbccddeeff\nreturn b ++ buf_cut(b, 1, 4) ++ buf_right(4, b)");
        lower("var b = 0x01020304\nvar i = 0u64\nvar s = 0u64\nwhile i < size(b) { s = s * 256 + byte(b, i)\ni += 1 }\nreturn s as bytes");
        lower("if 3 > 2 { return 0x01 } else { return 0x02 }");
        lower("require 1001, 1 < 2\nreturn 0xff");
        lower("throw 42");
        lower("var x = min(3, 5) + max(2, 9) + clamp(11, 1, 10)\nreturn x as bytes");
        lower("var x = (2 ** 10) % 1000\nreturn x as bytes");
    }

    /* ---- end-to-end execution through the consensus engine ---- */

    #[test]
    fn e2e_arithmetic_bodies() {
        // sum of 0..=99 with compound assignment
        let out = run_e2e(
            "var s = 0u64\nvar i = 0u64\nwhile i < 100 { s += i\ni += 1 }\nreturn s as bytes",
            &[],
        );
        assert_eq!(out, 4950u64.to_be_bytes().to_vec());

        // mul-acc mod prime (score_sum shaped)
        let out = run_e2e(
            "var s = 0u64\nvar i = 0u64\nwhile i < 64 { s = (s * 31 + i * 7) % 1000000007\ni += 1 }\nreturn s as bytes",
            &[],
        );
        let mut expect = 0u64;
        for i in 0u64..64 {
            expect = (expect.wrapping_mul(31).wrapping_add(i * 7)) % 1_000_000_007;
        }
        assert_eq!(out, expect.to_be_bytes().to_vec());

        // bytes: concat + cut + right
        let out = run_e2e(
            "var b = 0x0011223344556677\nreturn b ++ buf_cut(b, 1, 2) ++ buf_right(2, b)",
            &[],
        );
        assert_eq!(out, hex::decode("001122334455667711226677").unwrap());

        // bytes → int fold (big-endian)
        let out = run_e2e(
            "var b = 0x01020304\nvar i = 0u64\nvar s = 0u64\nwhile i < size(b) { s = s * 256 + byte(b, i)\ni += 1 }\nreturn s as bytes",
            &[],
        );
        assert_eq!(out, 0x01020304u64.to_be_bytes().to_vec());

        // branches: break/continue
        let out = run_e2e(
            "var s = 0u64\nvar i = 0u64\nwhile i < 100 { i += 1\nif i % 2 == 0 { continue }\nif i > 20 { break }\ns += i }\nreturn s as bytes",
            &[],
        );
        assert_eq!(out, 100u64.to_be_bytes().to_vec()); // 1+3+...+19 = 100

        // comparison semantics (all three modes): LE/GE carried two latent
        // encoder bugs that value-coincidence hid — pin all directions with
        // operand values chosen to flip on the bug.
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let pairs: [(u64, u64, &str); 6] = [
                (5, 5, "eq"), (5, 9, "lt"), (9, 5, "gt"),
                (0, 1, "lt01"), (1, 0, "gt10"), (6100, 6100, "eqbig"),
            ];
            for (a, b, tag) in pairs {
                let body = format!(
                    "var x = {a}u64\nvar y = {b}u64\nvar lt = x < y\nvar gt = x > y\nvar le = x <= y\nvar ge = x >= y\nreturn (lt * 1 + gt * 10 + le * 100 + ge * 1000) as bytes"
                );
                let want = (a < b) as u64 * 1
                    + (a > b) as u64 * 10
                    + (a <= b) as u64 * 100
                    + (a >= b) as u64 * 1000;
                let out = run_e2e_mode(&body, &[], mode);
                assert_eq!(
                    out,
                    want.to_be_bytes().to_vec(),
                    "cmp {tag} mode {mode:?}"
                );
            }
        }

        // checked overflow traps with exit code 90
        let lo = lower("var a = 18446744073709551615u64\nreturn (a + 1) as bytes");
        let cap = SpaceCap::new(0);
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
        assert!(exec.result.is_err());
        assert_eq!(exec.exit_code, 90);

        // division by zero traps with 90
        let lo = lower("var z = 0u64\nreturn (5 / z) as bytes");
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
        assert!(exec.result.is_err());
        assert_eq!(exec.exit_code, 90);

        // require error value maps to the exit code
        let lo = lower("require 1001, false\nreturn 0x01");
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
        assert!(exec.result.is_err());
        assert_eq!(exec.exit_code, 1001);
    }

    /* ---- input-parameter kernel (the `function rv64im k(data: bytes)` ABI) ---- */

    fn compile_with_input(body: &str) -> (IRNodeArray, Vec<u8>) {
        let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
        let (ir, compiled, _) = compile_body(
            tokens,
            vec![("data".to_string(), crate::value::ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .unwrap();
        match compiled {
            crate::fitshc::CompiledCode::Bytecode(b) => (ir, b),
            _ => unreachable!(),
        }
    }

    fn run_input(body: &str, input: &[u8]) -> Vec<u8> {
        run_input_mode(body, input, Rv64Mode::Im)
    }

    fn run_input_mode(body: &str, input: &[u8], mode: Rv64Mode) -> Vec<u8> {
        let (ir, _) = compile_with_input(body);
        let lo = lower_body_mode(&ir, true, mode).unwrap();
        let cap = SpaceCap::new(0);
        let container = match mode {
            Rv64Mode::Im => lo.pack_segments(&cap, 64, 5120, 1280),
            _ => lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC),
        };
        let fno = CalcFnObj::build_v1(container.unwrap_or_else(|e| panic!("pack: {e}"))).unwrap();
        let exec =
            fno.run(input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
        match exec.result {
            Ok(out) => out,
            Err(e) => panic!("kernel aborted: {e:?} exit={}", exec.exit_code),
        }
    }

    /// score_sum shape: byte-weighted mod-prime accumulation and a
    /// record-wise BE u64 fold — the HacdsDeFi `score_sum` wire format.
    #[test]
    fn e2e_input_score_sum_shape() {
        let body = "var s = 0u64\nvar i = 0u64\nvar n = size(data)\nwhile i < n { s = (s * 31 + byte(data, i) * 7) % 1000000007\ni += 1 }\nreturn s as bytes";
        let input: Vec<u8> = (0u8..=39).collect(); // 40 bytes
        let out = run_input(body, &input);
        let mut want: u64 = 0;
        for &b in &input {
            want = (want.wrapping_mul(31).wrapping_add(b as u64 * 7)) % 1_000_000_007;
        }
        assert_eq!(out, want.to_be_bytes().to_vec());

        // record-wise BE u64 fold: 5 × 8-byte records summed
        let body2 = "var s = 0u64\nvar off = 0u64\nvar n = size(data)\nwhile off < n { var v = 0u64\nvar j = 0u64\nwhile j < 8 { v = v * 256 + byte(data, off + j)\nj += 1 }\ns += v\noff += 8 }\nreturn s as bytes";
        let mut input2 = Vec::new();
        let mut want2 = 0u64;
        for k in 0u64..5 {
            let r = (k * 0x0102_0304_0506_0708) % 1_000_000_007;
            want2 = want2.wrapping_add(r);
            input2.extend_from_slice(&r.to_be_bytes());
        }
        let out2 = run_input(body2, &input2);
        assert_eq!(out2, want2.to_be_bytes().to_vec());
    }

    /* ---- comparison report (same IR, both channels) ---- */

    /// cycles→compute uses the frozen CYCLES_PER_COMPUTE = 56 (SPEC §8).
    const N: f64 = 56.0;

    /// Find the first while-loop body in the IR and price its HVM bytecode.
    fn hvm_per_iter_gas(ir: &IRNodeArray) -> Option<i64> {
        fn find_while(node: &dyn IRNode) -> Option<i64> {
            if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
                return find_while(&*w.node);
            }
            if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                for c in &arr.subs {
                    if let Some(r) = find_while(&**c) {
                        return Some(r);
                    }
                }
            }
            if let Some(d) = node.as_any().downcast_ref::<IRNodeDouble>() {
                if d.inst == Bytecode::IRWHILE {
                    if let Ok(bts) = d.suby.codegen() {
                        return hvm_bytecode_stats(&bts).ok().map(|(_, g)| g);
                    }
                }
                for c in [&d.subx, &d.suby] {
                    if let Some(r) = find_while(&**c) {
                        return Some(r);
                    }
                }
            }
            None
        }
        find_while(ir)
    }

    /// Executed rv64 cycles for a body run through the consensus engine.
    fn rv_exec(body: &str) -> (u64, Vec<u8>) {
        let lo = lower(body);
        let cap = SpaceCap::new(0);
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec =
            fno.run(&[], CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
        match exec.result {
            Ok(o) => (exec.cycles_used, o),
            Err(e) => panic!("abort {e:?}"),
        }
    }

    #[test]
    fn comparison_report() {
        let bodies: Vec<(&str, &str, u64)> = vec![
            (
                "sum loop",
                "var s = 0u64\nvar i = 0u64\nwhile i < 1000 { s += i\ni += 1 }\nreturn s as bytes",
                1000,
            ),
            (
                "mul-acc mod prime (score_sum shape)",
                "var s = 0u64\nvar i = 0u64\nwhile i < 1000 { s = (s * 31 + i * 7) % 1000000007\ni += 1 }\nreturn s as bytes",
                1000,
            ),
            (
                "bytes fold (byte/size BE u64)",
                "var b = 0x0102030405060708\nvar i = 0u64\nvar s = 0u64\nwhile i < 1000 { s = (s * 31 + byte(b, i % 8)) % 1000000007\ni += 1 }\nreturn s as bytes",
                1000,
            ),
            (
                "branchy loop (continue)",
                "var s = 0u64\nvar i = 0u64\nwhile i < 1000 { i += 1\nif i % 2 == 0 { continue }\ns += i }\nreturn s as bytes",
                1000,
            ),
        ];
        println!(
            "{:<34} {:>6} {:>6} {:>8} {:>10} {:>11} {:>9} {:>8}",
            "workload", "rv_B", "hvm_B", "rv_inst", "cyc/iter", "comp/iter", "hvm_gas_i", "win"
        );
        for (name, body, iters) in &bodies {
            let (ir, hvm) = compile(body);
            let lo = lower_body(&ir).unwrap();
            let (hb, _) = hvm_bytecode_stats(&hvm).unwrap();
            let (exec, _out) = rv_exec(body);
            let per_iter = exec / *iters;
            let comp_iter = per_iter as f64 / N;
            let hvm_gas = hvm_per_iter_gas(&ir).unwrap_or(0);
            let win = hvm_gas as f64 / comp_iter;
            println!(
                "{:<34} {:>6} {:>6} {:>8} {:>10} {:>11.2} {:>9} {:>7.0}x",
                name,
                lo.stats.code_bytes,
                hb,
                lo.stats.insts,
                per_iter,
                comp_iter,
                hvm_gas,
                win
            );
            assert_eq!(lo.stats.code_bytes % 4, 0);
        }
    }

    /* ---- A-19 B2-FIXUP: mul_check_base_const copy->check rescan pins ---- */

    fn li_i(rd: u8, imm: i64) -> Inst {
        Inst::ImmAlu { op: ImmAluOp::Addi, rd, rs1: 0, imm }
    }
    fn mv_r(rd: u8, rs2: u8) -> Inst {
        Inst::RegAlu { op: RegAluOp::Add, rd, rs1: 0, rs2 }
    }

    /// Review counter-example (acceptance review of 03374a6): a base
    /// register write BETWEEN the copy and the check used to escape the
    /// backward-scan veto (the region is visited while `base` is still
    /// None). The check then tests the pre-write value while a later
    /// same-base site reads the post-write one — trap-set inclusion
    /// breaks, traps can be swallowed. The copy->check forward rescan
    /// must veto: None.
    #[test]
    fn a19_mulchk_base_write_between_copy_and_check_vetoes() {
        // li k6,6; mv d,r; li r,v2; mulhu sc,d,k6; bne sc -> 90
        let insts = [
            li_i(11, 6),
            mv_r(10, 25),
            li_i(25, 99), // base redefined after the copy, before the check
            Inst::RegAlu { op: RegAluOp::Mulhu, rd: 31, rs1: 10, rs2: 11 },
            Inst::Branch { kind: BrKind::Ne, rs1: 31, rs2: 0, target: 2880 },
        ];
        assert_eq!(mul_check_base_const(&insts, 3, 10, Some(11)), None);
    }

    /// Positive control (guards against over-tightening): same shape
    /// without the in-region base write recognizes (r, 6).
    #[test]
    fn a19_mulchk_clean_site_recognized() {
        let insts = [
            li_i(11, 6),
            mv_r(10, 25),
            Inst::RegAlu { op: RegAluOp::Mulhu, rd: 31, rs1: 10, rs2: 11 },
        ];
        assert_eq!(mul_check_base_const(&insts, 2, 10, Some(11)), Some((25, 6)));
    }

    /// The pow2/srl form (t = None) shares the same hole and the same
    /// fix: an in-region base write must veto.
    #[test]
    fn a19_mulchk_srl_form_base_write_vetoes() {
        // mv d,r; li r,v2; srli sc,d,61; bne sc -> 90
        let insts = [
            mv_r(10, 25),
            li_i(25, 3), // base redefined between copy and check
            Inst::ShiftImm { kind: ShiftKind::Srl, rd: 31, rs1: 10, shamt: 61, word: false },
            Inst::Branch { kind: BrKind::Ne, rs1: 31, rs2: 0, target: 2880 },
        ];
        assert_eq!(mul_check_base_const(&insts, 2, 10, None), None);
        // control without the rewrite: recognized, k comes from the caller
        let clean = [
            mv_r(10, 25),
            Inst::ShiftImm { kind: ShiftKind::Srl, rd: 31, rs1: 10, shamt: 61, word: false },
        ];
        assert_eq!(mul_check_base_const(&clean, 1, 10, None), Some((25, 0)));
    }

    /// A base write BEFORE the copy is harmless (both sites' copies read
    /// the post-write value) and must NOT veto.
    #[test]
    fn a19_mulchk_base_write_before_copy_allowed() {
        let insts = [
            li_i(25, 7), // base defined earlier — fine
            li_i(11, 10),
            mv_r(10, 25),
            Inst::RegAlu { op: RegAluOp::Mulhu, rd: 31, rs1: 10, rs2: 11 },
        ];
        assert_eq!(mul_check_base_const(&insts, 3, 10, Some(11)), Some((25, 10)));
    }

}
