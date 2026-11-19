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
//!   CU8, CU16, CU32, CU64, CU128, CBYTES`. A u128 value lowers as a
//!   (lo, hi) u64 pair — two vstack entries, 16-byte slots, arithmetic in
//!   the pair domain with the interpreter's same-width overflow faces.
//! * casts (T13): `CTO` for the `as bool` / `as address` targets — bool is
//!   `extract_bool` truthiness, address is the 21-byte + version decode
//!   (the address literal is exactly `CTO(Address, PBUF 21B)`); an Address
//!   value lowers as its 21 content bytes with a static Address stain
//!   (EQ/NEQ compare raw bytes, a non-21 Bytes against an Address faults
//!   90); `CU8..CU128`/`CBYTES` also accept bytes/address operands (the
//!   BE-trimmed fit / the scalar_bytes identity).
//! * ints: `ADD SUB MUL DIV MOD POW MAX MIN CLAMP ABSDIFF DIVUP DIVEXACT`
//!   (checked), `BSHL BSHR BXOR BOR BAND` (shift amounts are checked:
//!   `>= width` faults 90 like the interpreter's `checked_shl/shr`),
//!   `EQ..GE`, `NOT, AND, OR, CHOOSE`. At u128 width: add/sub/mul/div/mod,
//!   the bitwise family and comparisons (mixed-width compares stay legal —
//!   numeric order; mixed-width ARITHMETIC rejects, as do shifts/pow/
//!   clamp/min/max/absdiff/divup/divexact — the shared soft-divmod routine
//!   covers 128-bit division).
//! * bytes: `CAT BYTE CUT LEFT RIGHT LDROP RDROP SIZE`.
//! * compo lists (v2 shape set): creation `[]`/`[..]` (≤ 128 items),
//!   `ITEMGET`/`HASKEY`/`LENGTH`/`APPEND`/`CLEAR` on list VARIABLES,
//!   `ITEMSET` (`l[i] = v` / `item_set`, in-place store on a local list
//!   slot — the may-Nil value rejects at compile time like APPEND's), the
//!   compound `l[i] += -= *= /=` (native lowering of the parser's raw
//!   DUPN/ROLL fragment, RHS-first), and `ITEMCAT` (`item_concat` — every
//!   element spliced by its `extract_bytes`, big-endian, 39 against the
//!   kernel's output budget).
//! * bytes writes: `BUFSET` (`buf_set` on a local bytes slot — the patch
//!   lands in a fresh arena copy that IS the result; the slot and every
//!   other holder keep their bytes, premise 24).
//! * locals: `GET GET0..GET3 PUT XOP(+= -= *= /=)`, plus the pure-compute
//!   subset extension: constant-index `GETX`/`PUTX` (local_x/local_x_put
//!   fold to GET/PUT; dynamic indices, out-of-frame indices and >u16
//!   literals reject), and `XLG` (local[idx] OP rhs — truthiness
//!   And/Or, content Eq/Ne with the exactly-one-Nil 90, uint-only ordered
//!   compares; bool/uint mixed pairs and bytes operands reject).
//! * type tests (pure-compute subset extension): `TNIL`/`TIS`/`TID`/
//!   `TLIST`/`TMAP` — statically folded from the operand's type model; a
//!   may-Nil operand reads its Nil tag (TNIL ≡ the init flag).
//! * `SQRT`/`SQRTUP` (pure-compute subset extension): software bitwise
//!   isqrt on the u64 value or the u128 (lo, hi) pair, result kept at the
//!   operand's width; non-uint static types reject (the interpreter's
//!   strict extract_u128 faults 72).
//! * control: `IRLIST IRBLOCK IRBLOCKR IRIF IRIFR IRWHILE IRBREAK IRCONTINUE,
//!   REQ AST ERR ABT RET END NOP`, plus the frame prelude `ALLOC`/`POP`.
//! * rejected with a compile error: mixed-width u128 arithmetic (and the
//!   u128 shift/pow/clamp/min/max/absdiff/divup/divexact faces, u128 list
//!   elements, slots mixing u128 with narrower flows), compo/address/nil
//!   values, maps
//!   and nested/tuple/list-element containers, every
//!   native/action/storage/status/global/log/call opcode, `bytecode {}`
//!   splices (other than the parser's own compound-assignment fragment,
//!   which lowers natively), dynamic `GETX`/`PUTX` slots, the
//!   row-C mul-pipeline ops, stack shuffles.
//! * heap (task 7): exactly ONE constant `heap_grow` per body — its u8
//!   immediate segment count must fit `SpaceCap::heap_grow_segment` (passed
//!   in through the lowering's parameter channel) and the reserved
//!   `seg × Heap::SEGLEN` bytes must fit the 2040B frame memory budget; a
//!   grow inside a loop, or a second grow anywhere (even on an exclusive
//!   path), rejects. The frame also carries a grown-flag word, so the heap
//!   length is 0 until the grow executes (reads/writes before it, and any
//!   access past the current length, trap HeapError(44) — the interpreter's
//!   `checked_right` faces, including the `read(0, 0)` empty-read edge).
//!   The six read/write opcodes share the one zero-filled block: `HREAD` /
//!   `HWRITE` take a dynamic u32/u16-checked offset, `HWRITEX` /
//!   `HWRITEXL` immediate offsets, `HREADU` / `HREADUL` their marker's
//!   uint width (u8..u128, the u128 read is a (lo, hi) pair); writes
//!   extract the value's canonical big-endian bytes (`extract_bytes`:
//!   width-exact uint scalars, a bool's 0/1 byte, u128's 16 bytes, bytes
//!   descriptors copied whole). A heap bytes read is a descriptor pointing
//!   INTO the frame block — S0 stays live for the whole body.
//!
//! ## Documented deviations from exact HVM semantics
//!
//! Shift amounts are checked against the operand width exactly like the
//! interpreter's `checked_shl`/`checked_shr` (amount >= width faults 90;
//! the shifted-out value bits truncate silently, which `sll`/`srl` match).
//! `x as bytes` is width-exact (the interpreter's `scalar_bytes`: u8 → 1 B,
//! u16 → 2, u32 → 4, u64 → 8, bool → 1 canonical byte) through a static
//! width model (`node_int_width` + the plan-time slot-width join); operands
//! with no static uint width (a bool mixed with ints) reject at compile
//! time. Overflow-face arithmetic (add/sub/mul/shift/pow) emits the
//! interpreter's narrow-width checks at the statically known width
//! (`emit_range_check`/`emit_truncate`: inputs < 2^w make the u64 op
//! exact, so a result-range check IS the interpreter's checked op);
//! div/mod/bitops/comparisons are width-blind and need no guard. A slot
//! whose writes mix bool-ish values with ints keeps the legacy uniform-u64
//! behavior (the interpreter faults 71 on the bool side — the pinned
//! D5BoolUint family). Comparisons are unsigned 64-bit (kernel ints are
//! non-negative).
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

pub(crate) use std::collections::{HashMap, HashSet};

pub(crate) use super::{
    IRNode, IRNodeArray, IRNodeBytecodes, IRNodeDouble, IRNodeEmpty, IRNodeLeaf,
    IRNodeParam1, IRNodeParam1Double, IRNodeParam1Quad, IRNodeParam1Single,
    IRNodeParam1Triple, IRNodeParam2, IRNodeParam2Single, IRNodeParams,
    IRNodeParamsSingle, IRNodeSingle, IRNodeText, IRNodeTriple, IRNodeWrapOne,
};
pub(crate) use crate::calc::container::{self, CalcSegment, CODE_BASE};
pub(crate) use crate::calc::decode::{decode_at, inst_cycles, Inst};
pub(crate) use crate::rt::{
    decode_local_logic_mark, decode_local_operand_mark, fin_spec_lookup,
    Bytecode, FinKernel, FinRoundPolicy, GasTable, LxOp, LxLg, SpaceCap,
    VmrtRes,
};
pub(crate) use crate::value::Value;
pub(crate) use sys::{errf, Ret};

mod asm;
mod classify;
mod dbg;
mod emit;
mod finish;
mod lib_fin;
mod lib_u128;
mod lib_u256;
mod plan;
mod plan_kinds;
mod types;
mod web;
mod widths;

pub use dbg::{
    dbg_int_literal, BANK_KIND_BYTES, BANK_KIND_INT, BANK_KIND_NONE, LowerDbg,
    StmtRangeDbg, SLOT_TY_ADDRESS, SLOT_TY_BYTES, SLOT_TY_INPUT, SLOT_TY_INT,
    SLOT_TY_LIST, SLOT_TY_MAP, SLOT_TY_NONE, WebEvDbg, WebTimelineDbg,
    WEB_EV_READ, WEB_EV_WRITE_C, WEB_EV_WRITE_U,
};
pub use finish::{
    hvm_bytecode_stats, GuardCopyRecord, MulMergeIvKind, MulMergeRecord,
    Rv64Lowered, Rv64Stats,
};
pub use types::{ARENA_BASE, ARENA_SIZE, ARENA_SIZE_MAX, Rv64Mode};

pub(crate) use asm::*;
pub(crate) use classify::*;
pub(crate) use emit::*;
pub(crate) use finish::*;
pub(crate) use plan::*;
pub(crate) use plan_kinds::*;
pub(crate) use types::*;
pub(crate) use web::*;
pub(crate) use widths::*;

#[cfg(test)]
mod tests;

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
///
/// The heap per-grow segment cap is the `SpaceCap` default
/// (`HEAP_GROW_SEGMENT_DEFAULT`); production callers pass the live cap
/// field through [`lower_body_mode_arena_heap`].
pub fn lower_body_mode_arena(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
) -> Ret<Rv64Lowered> {
    lower_body_mode_arena_impl(
        ir,
        has_input,
        mode,
        arena_size,
        crate::rt::HEAP_GROW_SEGMENT_DEFAULT,
        false,
        true,
    )
    .map(|(lo, _)| lo)
}

/// Task 7: the lowering entry carrying the caller's
/// `SpaceCap::heap_grow_segment` (the B2-style parameter channel — the
/// kernel never hardcodes a segment cap). The check is compile-time: a
/// constant HGROW above the field rejects.
pub fn lower_body_mode_arena_heap(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
    heap_seg_max: usize,
) -> Ret<Rv64Lowered> {
    lower_body_mode_arena_impl(ir, has_input, mode, arena_size, heap_seg_max, false, true)
        .map(|(lo, _)| lo)
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
    lower_body_mode_arena_impl(
        ir,
        has_input,
        mode,
        arena_size,
        crate::rt::HEAP_GROW_SEGMENT_DEFAULT,
        true,
        true,
    )
    .map(|(lo, dbg)| (lo, dbg.expect("dbg requested")))
}

/// P2 (§3 shared prework) — the ARENA CONTAINER MODEL entry.
///
/// T17 wired the container model into every lowering entry: this function
/// is now an explicit alias of [`lower_body_mode_arena_heap`] (kept for
/// the existing differential harnesses, which name the model entry). The
/// admitted forms: an alias PUT (`var m = l` — the descriptor word is
/// copied, both slots share one arena object, in-place mutation is
/// visible through every alias), a re-bind (`l = <list>` — a fresh
/// object; other aliases keep the old), a CLONE-bind (the interpreter's
/// only deep copy), a receiver-yielding mutation bind (`var m =
/// append(l, v)` / insert / remove / merge / clear), a CHOOSE between two
/// list slots, TAKEFIRST/TAKELAST, MERGE, the true shifting INSERT/
/// REMOVE, KEYS/VALUES(list) (the 83 face), list EQ/NEQ, the list cast /
/// ordered-compare / XOP fault faces and nested pack elements (76). The
/// involved list slots take the `ListKind::Arena` storage class: their
/// frame cell holds the arena object descriptor.
///
/// What still rejects: returning a container (the kernel ABI always
/// pushes Value::Bytes), an aliased item_concat, the compound item
/// assignment on an arena list, a list declared under a conditional, and
/// the unrepresentable width/Nil element stores. T16's map family (map
/// slots, the `ctr_map_*` helpers, KEYS/VALUES snapshots) is admitted on
/// every entry as well.
pub fn lower_body_mode_arena_containers(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
    heap_seg_max: usize,
) -> Ret<Rv64Lowered> {
    lower_body_mode_arena_impl(ir, has_input, mode, arena_size, heap_seg_max, false, true)
        .map(|(lo, _)| lo)
}

pub(crate) fn lower_body_mode_arena_impl(
    ir: &IRNodeArray,
    has_input: bool,
    mode: Rv64Mode,
    arena_size: u64,
    heap_seg_max: usize,
    want_dbg: bool,
    containers: bool,
) -> Ret<(Rv64Lowered, Option<LowerDbg>)> {
    let mut fp = plan(&ir.subs, has_input, heap_seg_max, containers)?;
    // BUG-012: flow-sensitive may-Nil analysis (init flags + value tags +
    // consumer guards). Slots proven non-Nil everywhere get no machinery.
    let nilp = nil_plan(&ir.subs, &fp.slot, has_input);
    // P3-IDIOMS: byte-assembly fusion marks — computed once (needs plan's
    // slot types + nilp's may-Nil occ), deterministic, shared by the
    // probe pass and every relaxation pass.
    let fuse = fuse_scan(&ir.subs, &fp.slot, &nilp);
    // task 3: the u128 soft-divmod routine's frame save area is reserved
    // at construction — compute the demand once, before the probe pass, so
    // both passes see the identical frame layout.
    // A9 (survey arith.md): the u128 mul routine's site count — the gate
    // is construction-time (the D2/D3 rule) and, when the routine
    // activates, it shares the divmod frame workspace (s+0..s+128), so
    // the reservation extends with the same count.
    let mul128_sites = body_u128_mul_site_count(&ir.subs, &fp.slot, &fp.slot_w, &fp.list.ew);
    let needs_divmod = body_needs_divmod(&ir.subs, &fp.slot, &fp.slot_w, &fp.list.ew)
        || mul128_sites >= MUL128_ROUTINE_MIN_SITES;
    // P1 u256 library: same construction-time reservation rule (the probe
    // pass and the real lowering must see the identical frame layout).
    let needs_u256 = body_needs_u256(&ir.subs, &fp.slot, &fp.slot_w, &fp.list.ew);
    // T26: the FIN operand-park reservation is sized by the body's static
    // FIN nesting depth (also construction-time, for the same reason).
    let fin_levels = fin_nesting_depth(&ir.subs);
    // D2/D3: the shared mul_wide / HalfUp / HalfEven routine gates — the
    // body's static per-primitive site counts (pure IR functions, same
    // construction-time rule, so the probe pass and every relaxation pass
    // lower identically; the frame layout never depends on them).
    let (mw_sites, hu_sites, he_sites) =
        body_routine_site_counts(&ir.subs, &fp.slot, &fp.slot_w, &fp.list.ew);
    // A8 (survey arith.md): the shared sqrt routines' per-variant gates —
    // the same construction-time rule (the probe pass and every relaxation
    // pass lower identically; the 64-byte frame workspace below is
    // reserved iff any variant gates on).
    let sqrt_sites = body_sqrt_site_counts(&ir.subs, &fp.slot, &fp.slot_w, &fp.list.ew);
    let routine_gates = (
        mw_sites >= FIN_ROUTINE_MIN_SITES,
        hu_sites >= FIN_ROUTINE_MIN_SITES,
        he_sites >= FIN_ROUTINE_MIN_SITES,
        mul128_sites >= MUL128_ROUTINE_MIN_SITES,
        sqrt_sites.0 >= SQRT_ROUTINE_MIN_SITES,
        sqrt_sites.1 >= SQRT_ROUTINE_MIN_SITES,
        sqrt_sites.2 >= SQRT_ROUTINE_MIN_SITES,
        sqrt_sites.3 >= SQRT_ROUTINE_MIN_SITES,
    );
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
    // T15: a body that reassigns `data` (or buf_sets into it) models slot 0
    // as a bytes local initialized at entry from a copy of the input
    let input_local = has_input && matches!(fp.slot.get(0), Some(SlotTy::Bytes));
    // dbg mirrors of the promotion plan (pure-addition bookkeeping)
    let mut lit_sites = 0usize;
    let mut bank_regs: Vec<u8> = Vec::new();
    if mode == Rv64Mode::ImcRemap {
        let mut probe = Lower::new_mode_arena_nil(&fp.slot, &fp.needs_zero, mode, arena_size, &nilp, &fp.list, &fp.slot_w, &fp.dyn_w, needs_divmod, needs_u256, fin_levels, fp.heap, input_local, routine_gates);
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
        // E4: the literal-pool base decision is independent of the bank
        // gate — x2 sits outside VSTACK_ORDER_RVC, so it is free at every
        // pool size. Deep bodies (D=7/8: no bank either) previously had
        // neither: every literal address paid the 8-byte auipc+addi pair
        // and every big constant a 6-8-byte li. With the base they pay a
        // 4-byte addi per address site and a 2-4-byte load per pooled
        // constant (E1's c.ldsp window). The prologue pcrel that
        // materializes the base runs after S0 is computed from SP
        // (emit_prologue), so the frame base is unaffected. The ±2KiB
        // pool-distance hard error (asm.rs PoolOff/PoolLd) is unchanged —
        // an oversized pool still refuses to compile.
        //
        // E5: the site count covers li_k hits (int literals > 0xFFFF, CU*
        // folds included — web.rs count_literal_sites) and the threshold is
        // 2: the base costs an 8-byte prologue pcrel and pays 4B/site, so
        // break-even sits at two sites; x2 was the bank candidate list's
        // LAST entry, so requisitioning it costs the bank nothing.
        lit_sites = count_literal_sites(&ir.subs);
        if lit_sites >= 2 {
            pool_base_reg = Some(2);
        }
        // E10: the bank construction is no longer gated on `d <= 6`. D=7/8
        // bodies keep the pool in positions 0..d of VSTACK_ORDER_RVC, but
        // the non-window group (x3,x4,x7,x16,x22..x27) is still fully free
        // there (every routine that writes one of them saves it — the
        // audit recorded at the candidate list below), so a partial bank
        // pays off: hot slots get zero-cost Ref reads instead of c.ld/sd
        // traffic. Only x14/x15 lose their candidate entries — they sit at
        // window positions 6/7 and are pool-occupied from D=7 / D=8 on.
        // P2-A27 bank expansion. Candidates are (register, busy) pairs:
        // busy = closed statement intervals during which the expression
        // machinery may write the register. Base list (all fully free):
        // the compressed window pair + gp/tp/t2/a6/s6..s11 + t1/x6
        // (S0: scratch a1 moved to SC2C) + sp/x2 (dead — no ABI stack).
        // a7/x17 is EXCLUDED (audit 3103669, §F3+§F4; defect shipped in
        // 25806ba): this list used to claim a7's only write is the
        // terminal `li a7,0; ecall`, but the shared routines use x17 as
        // an UNSAVED working scratch — emit_divmod_routine (lib_u128.rs)
        // and emit_mulmod_routine (emit/arith.rs) write it on every call
        // (`or TMP, VLO, VHI` V==0 check, TMP == 17), and
        // emit_u256_divmod_u128_routine / emit_u256_divmod_u129_routine
        // (lib_u256.rs) do the same — so an ImcRemap bank slot homed in
        // x17 and live across a `jal a0` call site was silently
        // corrupted. Do NOT re-add 17 here unless every routine's
        // save/restore set covers it too. Then
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
            (3, Vec::new()),
            (4, Vec::new()),
            (7, Vec::new()),
            (16, Vec::new()),
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
        // The compressed-window pair still leads the candidate order
        // (prefer window regs for the bank — free c.ld/sd access) but only
        // while the pool does not occupy it: x14 is window position 6, x15
        // position 7, so their entries are dropped from D=7 / D=8 on (E10).
        if d <= 6 {
            regs.insert(0, (15, Vec::new()));
            regs.insert(0, (14, Vec::new()));
        } else if d == 7 {
            regs.insert(0, (15, Vec::new()));
        }
        // E4: x2 leaves the bank candidate list when the pool base
        // claimed it above (it was the LAST entry, so the bank loses
        // its least-preferred register — near-zero cost)
        if pool_base_reg.is_some() {
            let base_pos = regs.iter().position(|&(r, _)| r == 2).expect("x2 base candidate");
            regs.remove(base_pos);
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
            &fp.slot_w,
            &fp.dyn_w,
            needs_divmod,
            needs_u256,
            fin_levels,
            fp.heap,
            input_local,
            routine_gates,
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
        // The body ends here; the shared routines and the fault thunks are
        // appended next, and the guard-copy pass below must see their final
        // positions to bake a correct c.j displacement (a c.j anchored on
        // the raw body end would jump into a shared routine whenever one is
        // emitted in between — T26 FIN bodies hit exactly that).
        let body_end = lw.asm.here();
        // task 3: the shared u128 soft-divmod routine (emitted only when a
        // division site called it; still inside the executable window)
        if let Some(dl) = lw.divmod_lbl {
            lw.asm.place(dl);
            lw.emit_divmod_routine();
        }
        // A9: the shared u128 multiply routine (checked mul + the POW
        // loop's mul_keep sites; same lazy-emission rule as the divmod
        // routine above — emitted only when a gated site staged through it)
        if let Some(l) = lw.mul128_lbl {
            lw.asm.place(l);
            lw.emit_mul128_routine();
        }
        // A8 (survey arith.md): the shared sqrt routines, one per
        // (domain, op) variant that a gated site staged through — same
        // lazy-emission rule, still inside the executable window.
        if let Some(l) = lw.sqrt64_lbl {
            lw.asm.place(l);
            lw.emit_sqrt64_routine(Bytecode::SQRT);
        }
        if let Some(l) = lw.sqrt64up_lbl {
            lw.asm.place(l);
            lw.emit_sqrt64_routine(Bytecode::SQRTUP);
        }
        if let Some(l) = lw.sqrt128_lbl {
            lw.asm.place(l);
            lw.emit_sqrt128_routine(Bytecode::SQRT);
        }
        if let Some(l) = lw.sqrt128up_lbl {
            lw.asm.place(l);
            lw.emit_sqrt128_routine(Bytecode::SQRTUP);
        }
        // T2: the shared u128 mul_mod routine (double-and-add), same lazy
        // emission as the divmod routine above.
        if let Some(ml) = lw.mulmod_lbl {
            lw.asm.place(ml);
            lw.emit_mulmod_routine();
        }
        // P1 u256 library: the shared 256-bit soft-division routines (same
        // lazy-emission rule as the u128 divmod routine — emitted only when
        // a u256 division site called them, still inside the executable
        // window).
        if let Some(l) = lw.u256div_lbl {
            lw.asm.place(l);
            lw.emit_u256_divmod_u128_routine();
        }
        if let Some(l) = lw.u256div129_lbl {
            lw.asm.place(l);
            lw.emit_u256_divmod_u129_routine();
        }
        // D3 (fin-control survey): the shared 128×128→256 wide-multiply
        // routine (same lazy-emission rule; never faults).
        if let Some(l) = lw.u256mul_lbl {
            lw.asm.place(l);
            lw.emit_u256_mul_wide_routine();
        }
        // D2 (fin-control survey): the shared HalfUp / HalfEven rounding
        // routines (same lazy-emission rule; the checked quo+1 carry →
        // fault_arith 90 branches from inside, the divmod V==0 shape).
        if let Some(l) = lw.u256round_hu_lbl {
            lw.asm.place(l);
            lw.emit_u256_round_halfup_routine();
        }
        if let Some(l) = lw.u256round_he_lbl {
            lw.asm.place(l);
            lw.emit_u256_round_halfeven_routine();
        }
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
        let thunk_arith_pos =
            lw.asm.labels[lw.fault_arith].expect("fault_arith placed above");
        let thunk_cast_pos = lw.asm.labels[lw.fault_cast].expect("fault_cast placed above");
        // P3-GUARDTAIL GT-a: insert copy thunks so clustered zero-form guard
        // branches reach a 2-byte c.bz. Re-derived identically on every
        // relaxation pass (deterministic function of the emitted stream);
        // only the BODY is scanned/rewritten (the already-emitted routines
        // keep their own direct branches).
        guard_copies_dbg = insert_guard_thunk_copies(
            &mut lw.asm,
            profile,
            lw.fault_arith,
            lw.fault_cast,
            lw.trap_label,
            body_end,
            thunk_arith_pos,
            thunk_cast_pos,
        )?;
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
        // E1: entries carry their creation-time pool offsets (8-byte value
        // entries are zero-padded to alignment by `lit_label`); the padding
        // rides in front of each label so the label positions equal the
        // recorded offsets exactly — the same decision the per-site loads
        // were emitted against.
        let mut pool_at = 0usize;
        for ent in &entries {
            assert!(
                ent.off >= pool_at,
                "rv64 lower: pool entry offset {} overlaps the previous entry ({pool_at})",
                ent.off
            );
            lw.asm.buf.extend(std::iter::repeat(0u8).take(ent.off - pool_at));
            lw.asm.place(ent.lbl);
            lw.asm.buf.extend_from_slice(&ent.bytes);
            pool_at = ent.off + ent.bytes.len();
            // E6: the interior labels of suffix-sharing sites point at
            // their delta inside the just-emitted entry — the shared read
            // sees exactly the requested bytes, and every PoolOff /
            // PcRelHiLo fixup on the interior label resolves to the true
            // address with the ±2KiB check (E12) operating on the real
            // offset.
            for (ilbl, delta) in &ent.interior {
                lw.asm.place_at(*ilbl, lw.asm.labels[ent.lbl].expect("entry label placed") + delta);
            }
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
                        SlotTy::Address => SLOT_TY_ADDRESS,
                        SlotTy::Input => SLOT_TY_INPUT,
                        SlotTy::List | SlotTy::ListBytes => SLOT_TY_LIST,
                        SlotTy::Map(_) => SLOT_TY_MAP,
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
