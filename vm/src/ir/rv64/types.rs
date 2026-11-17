
/* ------------------------------------------------------------------ */
/* Registers and memory plan                                           */
/* ------------------------------------------------------------------ */

pub(crate) const ZERO: u8 = 0;
pub(crate) const RA: u8 = 1;
pub(crate) const SP: u8 = 2;
pub(crate) const T0: u8 = 5;
pub(crate) const T1: u8 = 6;
pub(crate) const T2: u8 = 7;
pub(crate) const T3: u8 = 28;
pub(crate) const T4: u8 = 29;
pub(crate) const T5: u8 = 30;
pub(crate) const T6: u8 = 31;
pub(crate) const S0: u8 = 8; // locals frame base
pub(crate) const S1: u8 = 9; // arena bump pointer
pub(crate) const S2: u8 = 18; // output_base (from a2)
pub(crate) const S3: u8 = 19; // output_max (from a3)
pub(crate) const S4: u8 = 20; // input_base (from a0)
pub(crate) const S5: u8 = 21; // input_len (from a1)
pub(crate) const A0: u8 = 10; // scratch; exit code / return length at exit
pub(crate) const A1: u8 = 11; // scratch
pub(crate) const A7: u8 = 17; // ecall number (never a general register)

/// Virtual register stack. Kept at 8 so 15/16 stay dedicated scratch: values
/// popped-but-still-live must never be clobbered by address temporaries.
pub(crate) const VSTACK_REGS: [u8; 8] = [T0, T1, T2, T3, T4, T5, T6, 14];
/// Dedicated scratch, outside the value stack.
pub(crate) const SC1: u8 = 15;
pub(crate) const SC2: u8 = 16;

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
pub(crate) const SC1I: u8 = 12;
pub(crate) const SC2I: u8 = 13;

/// RVC-mode value pool ORDER: the compressed-window regs x14/x15 come
/// LAST so a shrinking pool (see `lower_body_mode_arena`) releases them
/// first into the local bank. With a full 8-slot pool this is the same
/// mapping as before: a0..a5 in the window, t3/t4 outside.
pub(crate) const VSTACK_ORDER_RVC: [u8; 8] = [A0, A1, 12, 13, T3, T4, 14, 15];
/// Free registers (ImcRemap) outside the value pool, never ABI-bound:
/// gp(x3)/tp(x4) — untouched by the self-contained kernel ABI — plus
/// t2(x7), a6(x16), s6..s11 (x22..x27). The bank prefers the compressed
/// window pair (x14/x15) first, then these.
pub(crate) const BANK_NONWINDOW: [u8; 10] = [3, 4, 7, 16, 22, 23, 24, 25, 26, 27];

/* ---- P1 (§3 shared prework): u256 library frame-workspace offsets ----
 * S0-relative, base `Lower::u256_save_off` (reserved iff body_needs_u256;
 * layout mirrored in the reservation comment there). */
/// dividend N limbs N0..N3 (N0 = bits 0..63); also mul_wide's transient
/// p0 park (workspace contents are dead across `emit_u256_mul_wide`).
pub(crate) const U256WS_N0: i32 = 0;
/// divisor V limbs V0,V1 (the u128 below bit 128).
pub(crate) const U256WS_V0: i32 = 32;
/// DH — divisor bit 128 (u129 path; runtime 0 or 1).
pub(crate) const U256WS_DH: i32 = 48;
/// quotient Q0,Q1 (routine-written).
pub(crate) const U256WS_Q0: i32 = 56;
/// remainder R0,R1 (routine-written, bits 0..127).
pub(crate) const U256WS_R0: i32 = 72;
/// RH — remainder bit 128 (u129 path, routine-written; 0 on the
/// u128-delegated path, written by the caller-side dispatch).
pub(crate) const U256WS_RH: i32 = 88;
/// saved working registers of the two routines (12 slots).
pub(crate) const U256WS_SAVE: i32 = 96;
/// T3/T4 triop operand staging INSIDE the routine-save area (dead between
/// routine calls — only the division routines touch it, balanced around
/// their jal): the y pair, then the z pair.
pub(crate) const U256WS_STAGE_Y: i32 = 96;
pub(crate) const U256WS_STAGE_Z: i32 = 112;
/// caller's kernel-entry RA park (around the jal).
pub(crate) const U256WS_RA: i32 = 192;
/* T26 FIN*: composite-park slots (u128 pairs, six of them) behind the RA
 * park. The FIN kernels keep live values (divisors, loop state, the isqrt
 * working set) here across mul_wide/divmod calls — those helpers clobber
 * the slots above (N0 doubles as mul_wide's p0 park, the routines write
 * V/DH/Q/R/RH/SAVE), so the FIN lowering NEVER parks anything there. */
pub(crate) const U256WS_FINA: i32 = 200;
pub(crate) const U256WS_FINB: i32 = 216;
pub(crate) const U256WS_FINC: i32 = 232;
pub(crate) const U256WS_FIND: i32 = 248;
pub(crate) const U256WS_FINE: i32 = 264;
pub(crate) const U256WS_FINF: i32 = 280;
/// total workspace size (kernel scratch slots FINA..FINF).
pub(crate) const U256WS_BYTES: i32 = 296;
/// T26 FIN*: per-nesting-level operand-park area, immediately after the
/// kernel scratch slots (base = U256WS_BYTES). A FIN node stages its
/// arguments here while later arguments evaluate, so a nested FIN call can
/// use the whole 8-entry value pool instead of overflowing it. Each level
/// owns 4 slots of 16B (the widest FIN arity is 4); the body's static FIN
/// nesting depth sizes the reservation.
pub(crate) const FIN_PARK_BASE: i32 = U256WS_BYTES;
pub(crate) const FIN_PARK_LEVEL_BYTES: i32 = 64;

/// Where a vstack value lives: an owned pool register, or a reference to a
/// bank (promoted-local) register. Refs are read-only for expression code
/// (only PUT/XOP write bank regs, after the rhs evaluation completes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VLoc {
    Pool(u8),
    Ref(u8),
}

pub(crate) fn loc_reg(l: VLoc) -> u8 {
    match l {
        VLoc::Pool(r) | VLoc::Ref(r) => r,
    }
}

/// Dedicated scratch in RVC mode (t5/t6, outside the window).
pub(crate) const SC1C: u8 = 30;
pub(crate) const SC2C: u8 = 31;

/// `wide_c_ids` namespace split: c.j relaxation ids (from `jal_seq`) are
/// tagged with this bit so they can never collide with c.bz ids (from
/// `branch_seq`). Both counters are far below 2^31 in any real kernel.
pub(crate) const JAL_C_ID_BIT: usize = 0x8000_0000;

/// Diagnostics-only env knobs (`RV64_NO_FWD` / `RV64_NO_DSE` /
/// `RV64_EV_DEBUG`): compiled out of release consensus products so no
/// environment variable can change emitted bytes (D-04 R1). Test binaries
/// and debug builds keep them; `RV64_INV_CHECK` deliberately stays
/// unconditional — it only panics on a violated invariant, it never
/// changes output.
pub(crate) fn diag_env_enabled(name: &str) -> bool {
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
pub(crate) static FWD_DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// When Some(slot), only this slot's forwarding stays enabled (per-slot
/// isolation diagnostics).
pub(crate) static FWD_ONLY_SLOT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
pub(crate) static FWD_ENV_READ: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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

pub(crate) const EXIT_ARITH: u64 = 90; // ItrErrCode::Arithmetic
pub(crate) const EXIT_CAST: u64 = 71; // ItrErrCode::CastFail


/* ------------------------------------------------------------------ */
/* Types + pre-pass                                                    */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ty {
    Int,
    Bytes,
    /// T13: an Address VALUE — its 21 content bytes as a plain bytes
    /// descriptor (ptr, len=21), plus this compile-time stain. Producers
    /// are `CTO(Address, ..)` nodes (the address literal is exactly that);
    /// storing into a local drops the stain (the slot becomes
    /// SlotTy::Bytes — EQ/NEQ compares bytes either way, the
    /// interpreter's own Address↔21B-Bytes shared value domain).
    Address,
    /// P3-V2IMPL (A-25): a compo-list VALUE under the admitted v2 shape set
    /// (doc 74 E1 / doc 73 E7). Never a vstack value — it only classifies
    /// plan()/node_res_ty expressions and slot states. Everything a list
    /// value would flow into outside the five admitted consumers is a
    /// compile-time reject (plan() is the single admit/reject source).
    List,
    /// T16: a map VALUE (the arena map descriptor). Never a vstack value —
    /// it classifies plan()/node_res_ty expressions and slot states; every
    /// consumer outside the admitted map ops is a compile-time reject.
    Map,
}

/// T16: the static VALUE SPECIES a map slot stores (the interpreter's map
/// values are scalars; the arena entry cell is a tagged scalar). The
/// species is fixed by the map's write sites and joins across aliases and
/// MERGE (the same one-type-per-slot discipline the list element width
/// uses). `Unset` = no write reached this slot yet (an always-empty map at
/// that program point — reads fault 85 / fold to an empty list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MapValKind {
    #[default]
    Unset,
    /// uint of a fixed width in bits (8/16/32/64/128)
    Uint(u32),
    /// Bytes/Address values — a (ptr, len) descriptor pair, tag 8
    Bytes,
}

impl MapValKind {
    /// Join two observed value species; `None` = incompatible (the kernel
    /// cell model fixes one species/width per map slot).
    pub(crate) fn join(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Unset, k) | (k, Self::Unset) => Some(k),
            (Self::Uint(a), Self::Uint(b)) if a == b => Some(Self::Uint(a)),
            (Self::Bytes, Self::Bytes) => Some(Self::Bytes),
            _ => None,
        }
    }
}

/// GT-c T8-style: human-readable slot type names for error messages (no
/// Debug leakage — the words match the fitsh surface: `int`, `bytes`,
/// the `data` parameter, lists).
pub(crate) fn slot_ty_name(t: SlotTy) -> &'static str {
    match t {
        SlotTy::None => "unbound",
        SlotTy::Int => "int",
        SlotTy::Bytes => "bytes",
        SlotTy::Address => "address",
        SlotTy::Input => "the input parameter (bytes)",
        SlotTy::List => "list",
        SlotTy::ListBytes => "list",
        SlotTy::Map(_) => "map",
    }
}

/// Same for the plan-side value classification ([`Ty`]).
pub(crate) fn ty_name(t: Ty) -> &'static str {
    match t {
        Ty::Int => "int",
        Ty::Bytes => "bytes",
        Ty::Address => "address",
        Ty::List => "list",
        Ty::Map => "map",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotTy {
    None,
    Int,
    Bytes,
    /// T13 (three-way stain fix): a local slot whose every value source is
    /// an Address value. The runtime representation is IDENTICAL to
    /// `Bytes` (the 21 content bytes behind a (ptr, len) pair in a 16-byte
    /// cell) — the variant exists only so type tests (`is address` /
    /// `is bytes` / `type_id`) answer the interpreter's runtime Address
    /// stain (9) instead of the Bytes face (8). A slot that mixes Address
    /// and Bytes sources stays a compile reject: the interpreter's stain
    /// is last-write-wins (flow-sensitive) and no static face can answer.
    Address,
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
    /// T16: a map-produced LIST whose elements are Bytes values (the KEYS
    /// result, or a VALUES result over a Bytes-valued map). The descriptor
    /// lives in the slot cell exactly like `List`; the element cells carry
    /// (tag=8, ptr, len) and item_get yields a bytes pair. Mutation/append
    /// paths reject it (plan()).
    ListBytes,
    /// T16: a map slot — the cell holds the TAGGED arena map descriptor
    /// (object pointer | CTR_MAP_DESC_TAG). The payload is the static value
    /// species every write site agrees on (`MapValKind`).
    Map(MapValKind),
}

/// P3-V2IMPL: final storage decision for one list slot, decided by the
/// `plan()` walk (creation shape + APPEND/CLEAR reachability refinement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListKind {
    NotList,
    /// `[]` / `new_list()` creation, never appended/cleared: len ≡ 0, no
    /// storage at all (any item_get faults 85 / folds false).
    Empty,
    /// All-literal `[lit0..litN-1]`, never appended/cleared: a read-only
    /// table of N×(ew/8)B little-endian entries (ew = the slot's fixed
    /// element width) in the literal pool / R segment. The creation site
    /// emits nothing.
    Rodata { n: u64 },
    /// Computed-element `[e0..eN-1]`, never appended/cleared: frame block
    /// of N×(ew/8)B filled by per-element stores at creation; static
    /// length N.
    FrameStatic { n: u64 },
    /// APPEND/CLEAR reachable, or `[]`/`new_list()` with appends: frame
    /// block of LIST_BLOCK_MAX bytes (decision D2 — the interpreter cap is
    /// 128 u64 elements = 1024B and every append guards against the block
    /// end with the 40 thunk; the element count budget is LIST_BLOCK_MAX /
    /// element-step), plus the length shadow in the slot's own frame cell.
    /// `init_n` = creation length (0 for NEWLIST, N for a PACKLIST that
    /// later mutates).
    Growable { init_n: u64 },
    /// P2 (§3 shared prework, arena container descriptor model): the slot's
    /// 8-byte frame cell holds a DESCRIPTOR — the raw pointer of an arena
    /// container object (see the `P2 arena container object model` section
    /// below). Selected when the list's value escapes into aliasing (a
    /// `var m = l` PUT, a re-bind `l = <list>`, or a `clone(l)` consumer) —
    /// every such slot shares one heap object per alias group, reproducing
    /// the interpreter's `Rc<RefCell<Compo>>` shared-mutable semantics:
    /// alias PUT / DUP copies the descriptor word (all aliases observe
    /// in-place mutation), CLONE is the only deep copy (a fresh object).
    /// `init_n` = creation fill length (the alias/clone-bound target keeps
    /// 0 — its binding statement writes the descriptor, not elements).
    /// T17: this class is reachable on EVERY lowering entry (the container
    /// model is the production channel now); the op set that forces it is
    /// alias/re-bind/CLONE binds, TAKEFIRST/TAKELAST, MERGE, the true
    /// INSERT/REMOVE, and the EQ/CHOOSE descriptor reads.
    Arena { init_n: u64 },
}

/// B2 task 4: the per-list memory budget — one growable block reserves
/// LIST_BLOCK_MAX bytes on the frame (the 2040B frame window is the kernel
/// memory budget the list blocks live in; 1024B is calibrated so one
/// growable block plus the local slots fit it). The element-count capacity
/// of a list is LIST_BLOCK_MAX / element-step: 128 for u64 (the historical
/// 128×8B D2 form, byte for byte), 256 for u32, 512 for u16, 1024 for u8,
/// and 64 for u128 (T19: the 16B-stride pair block).
pub(crate) const LIST_BLOCK_MAX: u64 = 1024;

/// Byte stride of a list slot with element width `ew` (unset/unknown -> the
/// u64 form; free function so the frame planner can use it without a
/// `Lower` receiver). T19: a u128 slot strides 16 (the (lo, hi) pair).
pub(crate) fn step_of(ew: Option<u32>) -> usize {
    match ew {
        Some(w) if w >= 8 => (w / 8) as usize,
        _ => 8,
    }
}
