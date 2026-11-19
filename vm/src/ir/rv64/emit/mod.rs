use super::*;

/// E6: one read-only literal-pool entry.
#[derive(Clone, Debug)]
pub(crate) struct PoolEnt {
    /// Label at the entry's content start (`off` inside the pool).
    pub(crate) lbl: usize,
    pub(crate) bytes: Vec<u8>,
    /// Pool-relative content offset, fixed at creation time (E1: 8-byte
    /// value entries may carry alignment padding in front of the label).
    pub(crate) off: usize,
    /// E6: 8-byte VALUE entries (the `li_k` c.ldsp/PoolLd loads) never
    /// take part in suffix sharing — neither as sharer (they need their
    /// own 8-aligned start for the c.ldsp window decision) nor as base
    /// (a shared tail would couple the address family's offsets to the
    /// value entry's alignment padding). Bytes-class address references
    /// share freely: reads go through `lbu` walks at arbitrary offsets.
    pub(crate) is_value: bool,
    /// E6 suffix sharing: (label, delta) of sites whose bytes are exactly
    /// this entry's suffix (delta = interior offset of the shared tail).
    /// The pool emission places each label at the entry's content start +
    /// delta, so every PoolOff/PcRelHiLo fixup resolves to the true
    /// address with no fixup-side bias arithmetic and the ±2KiB pool
    /// distance check (E12) keeps operating on the real offset.
    pub(crate) interior: Vec<(usize, usize)>,
}

pub(crate) struct Lower<'a> {
    pub(crate) asm: Asm,
    pub(crate) slot: &'a [SlotTy],
    pub(crate) needs_zero: &'a [bool],
    /// Value pool for this mode (prefix of `VSTACK_ORDER_RVC` in
    /// `ImcRemap`, sized to the body's peak expression depth; the classic
    /// fixed 8-reg pools otherwise).
    pub(crate) vregs: Vec<u8>,
    /// Promoted-local bank: per-slot kind + per-segment register homes.
    pub(crate) plan: WebPlan,
    /// Statement-index cursor mirroring the web walk (drives `home_at`).
    pub(crate) web_idx: usize,
    /// Loop/address scratch registers. In `ImcRemap` these move off the ABI
    /// a0/a1 (x10/x11) — they become pool slots — onto t0/x5 and SC2C/x31
    /// (P2-A27 S0: a1 left t1/x6, which joins the bank; x5 must stay —
    /// `emit_cbytes`/`emit_return` need three concurrent scratches); ABI-
    /// boundary writes (exit code, return length) keep using the x10/x11 consts.
    pub(crate) a0: u8,
    pub(crate) a1: u8,
    /// P2-A27 S1 (probe only): record per-vstack-position liveness spans —
    /// `pos_spans[k]` = statements during which stack position `k` (hence
    /// pool register `vregs[k]`) holds a live value. The probe lowers with
    /// an empty plan, so every operand materializes as a Pool push: its
    /// spans are a sound over-approximation of the real (banked) lowering's
    /// pool-register writes, which is what the busy-candidate conflict test
    /// needs. Recorded on `push_vreg`/`push_ref`/`pop_vreg` and the two
    /// direct re-push sites (`pop_own_replace`, value-if arm unification).
    pub(crate) record_spans: bool,
    pub(crate) span_open: [Option<usize>; 8],
    pub(crate) pos_spans: Vec<Vec<(usize, usize)>>,
    /// P2-A27 S1 (INV_CHECK only): flattened bank-home liveness —
    /// (reg, seg_start, seg_end, is_init) per segment. `push_vreg` asserts
    /// the pool register it hands out is not a live bank home at the
    /// current statement (the plan-level busy test's runtime mirror).
    pub(crate) plan_reg_live: Vec<(u8, usize, usize, bool)>,
    /// Arena end address for the exhaustion check.
    pub(crate) arena_end: u64,
    pub(crate) vstack: Vec<VLoc>,
    /// Set when the body nests deeper than the register pool; the lowered
    /// output is rejected (subset limit, see `push_vreg`).
    pub(crate) depth_err: bool,
    /// Peak vstack depth observed (diagnostics; drives local-bank sizing).
    pub(crate) max_depth: usize,
    pub(crate) frame_bytes: usize,
    /// Task 7: the admitted single constant HGROW's segment count (None =
    /// no reservation; heap faces then run against a length-0 heap).
    pub(crate) heap_seg: Option<u8>,
    /// Grown-flag word (8B, S0-relative): 0 = heap empty, 1 = the grow has
    /// executed. Zeroed in the prologue with the heap data block.
    pub(crate) heap_flag_off: i32,
    /// Heap data block base (S0-relative; a valid offset even when
    /// unallocated, though never dereferenced without a reservation).
    pub(crate) heap_off: i32,
    /// Reserved heap bytes = seg × Heap::SEGLEN (0 without a grow).
    pub(crate) heap_bytes: u32,
    /// T16: S0-relative base of the 8-word map-op scratch area (-1 when the
    /// body has no map slot; never dereferenced then).
    pub(crate) map_scratch_off: i32,
    /// Staging cells inside the map scratch area (word offsets from
    /// `map_scratch_off`): key pointer/length and the pending value pair.
    pub(crate) map_kptr_cell: i32,
    pub(crate) map_klen_cell: i32,
    pub(crate) map_v0_cell: i32,
    pub(crate) map_v1_cell: i32,
    /// One read-only pool entry. Entries are created in first-use order
    /// and laid out consecutively; `off` is the entry's offset from the
    /// pool start, fixed at creation time (E1: 8-byte value entries may
    /// carry alignment padding in front), so the pool layout and the
    /// per-site loads agree by construction.
    pub(crate) pool_entries: Vec<PoolEnt>,
    /// Bytes of pool content + alignment padding accumulated so far (the
    /// offset the NEXT entry lands at).
    pub(crate) pool_bytes: usize,
    /// Exact-dedup map: bytes → (label, pool-relative content offset).
    /// E6: suffix-shared sites are registered here too, so a repeated
    /// shared literal dedups to its interior label instead of re-scanning
    /// (and a later scan cannot double-register it).
    pub(crate) pool_map: HashMap<Vec<u8>, (usize, usize)>,
    pub(crate) break_labels: Vec<usize>,
    pub(crate) continue_labels: Vec<usize>,
    pub(crate) trap_label: usize,
    pub(crate) fault_arith: usize,
    pub(crate) fault_cast: usize,
    /// Whether the `PUT 0 (ROLL0)` param binding has been consumed.
    pub(crate) param_bound: bool,
    /// T15: slot 0 is a bytes LOCAL initialized at entry from a copy of
    /// the input (a user `data = <bytes>` reassignment or a
    /// `buf_set(data, ..)` receiver transitioned it in plan()); the
    /// prelude emits the arena copy and the slot's frame cell/bank home
    /// takes the place of the ABI capture registers.
    pub(crate) input_local: bool,
    /// Function-wide literal-pool base register (prologue pcrel + per-site
    /// `addi rd, base, off` — 2-byte c.addi per literal reference instead
    /// of an 8-byte auipc+addi pair). None when too few literals pay for it.
    pub(crate) pool_base: Option<(u8, usize)>,
    /// Frame byte offset per slot: un-promoted slots laid out hottest-first
    /// so the most-accessed stay inside the c.ld/c.sd 248-byte window.
    pub(crate) frame_off: Vec<i32>,
    /// Store-to-load forwarding: the pool register that still holds the last
    /// stored value of each un-banked frame slot (killed at every join label
    /// and whenever the register is re-handed by `push_vreg`).
    pub(crate) fwd_int: Vec<Option<u8>>,
    pub(crate) fwd_bytes: Vec<Option<(u8, u8)>>,
    /* ---- C12 (impl/cse-dse-bank): the bytes-literal descriptor table ---- */
    /// Pool label → the (ptr, len) register pair a bytes-literal descriptor
    /// was last materialized into. Producers: the pool-literal eval sites
    /// (PBUF/PBUFL, folded scalar_bytes, CTO address literal, folded-slice
    /// C4 helper) and the banked literal bind's bank homes. A repeat site
    /// whose target pair matches the recorded one re-uses the registers
    /// instead of re-emitting `lit_addr` + `li`: the registers still hold
    /// the descriptor because every rewriter invalidates the entry —
    /// `kill_fwd_reg` (push_vreg re-hands, bank writes, guard borrows)
    /// drops entries naming a rewritten register and `place` annihilates
    /// the table at every join/branch target, the same discipline the
    /// fwd_bytes table runs under. Only the exact label hits (a
    /// suffix-shared interior label is a different key). Never iterated —
    /// get/insert/retain/clear only, so lowering stays deterministic.
    pub(crate) desc_fwd: HashMap<usize, (u8, u8)>,
    /* ---- B14 (impl/arena-mapflow): the len-zero map fact ---- */
    /// Per map slot: the slot cell holds a descriptor whose object was
    /// created by THIS body's `PUT s := new_map()` in the current
    /// straight-line run, so its live length is the constant 0 (the first
    /// INSERT/ITEMSET consumes the fact; killed at every join label, by
    /// every other map operation and by an alias PUT — the fwd_int/
    /// fwd_bytes kill discipline).
    pub(crate) map_zero: Vec<bool>,
    /* ---- B9 (impl/arena-mapflow): the arena-top fact ---- */
    /// The bytes descriptor (ptr, len) whose bytes end EXACTLY at the
    /// current arena top (ptr + len == S1), as register names. Established
    /// by every value that is freshly bumped onto the arena stream
    /// (cbytes / cbytes128 / cbytes_dyn / CAT (fused or not) / buf_set /
    /// itemcat splice / the input-local copy's banked result); consumed by
    /// emit_cat (and the fused chain's head leaf) to append the right
    /// operand in place instead of re-copying the receive side. Killed by
    /// every other S1 bump, at every join label (`place`), and whenever a
    /// watched register is re-handed (`kill_fwd_reg`).
    pub(crate) cat_top: Option<(u8, u8)>,
    /// Bookkeeping epoch: bumped by every `cat_top_kill`. `cat_top_gen` is
    /// the epoch at the fact's establishment. A live fact must always
    /// satisfy `cat_top_gen == s1_gen` — the RV64_INV_CHECK mirror asserts
    /// this at the consumption sites, catching any path that mutates
    /// `cat_top` without the helpers.
    pub(crate) s1_gen: u32,
    pub(crate) cat_top_gen: u32,
    /// Highest statement index at which each slot is read (from the web
    /// walk's event timeline); a store at an index ≥ this is dead.
    pub(crate) last_read: Vec<usize>,
    /// Sorted per-slot read indices + while spans (backedge liveness: a
    /// read earlier in linear order than a store, but inside a loop span
    /// that contains the store, re-executes after it).
    pub(crate) read_idx: Vec<Vec<usize>>,
    pub(crate) while_spans: Vec<(usize, usize)>,
    /// Statement index of an elidable `var x = 0` store per slot (the
    /// prologue's forced frame zero replaces it).
    pub(crate) zero_init_skip: Vec<Option<usize>>,
    /// Slots read by a while condition (loop-carried): their writes are
    /// re-read by the backedge invisibly, so dead-store elimination and
    /// forwarding-invalid timing must both stand down for them.
    pub(crate) loop_carried: Vec<bool>,
    /// Debug side-channel (P0 experiments): when `Some`, `stmt_one` /
    /// `stmt_list` record per-statement code-offset ranges into it.
    /// Write-only bookkeeping — no existing path reads it, so lowering
    /// stays byte-identical (pinned by `dbg_engine_matches_production`).
    pub(crate) stmt_trace: Option<Vec<StmtRangeDbg>>,
    /* ---- BUG-012 Nil tracking ---- */
    /// per-node may-Nil info from the pre-pass (keyed by node address)
    pub(crate) nil_occ: HashMap<usize, NilOcc>,
    /// P3-IDIOMS: byte-assembly fusion marks (Root/Skip), computed once
    /// by fuse_scan() outside the relaxation loop (deterministic).
    pub(crate) fuse: FuseMap,
    /// P3-PAIRPKG: pair marks from plan() — Root on REMOVE statement nodes
    /// (the fused single-`sd` emission site), Skip on the INSERT sibling
    /// (fully subsumed). Consulted at the two statement-sequence dispatch
    /// sites only; the statement walk never meets these opcodes unmarked.
    pub(crate) pair_root: HashMap<usize, PairRec>,
    pub(crate) pair_skip: HashSet<usize>,
    /// slot idx → carries an init flag byte
    pub(crate) tracked: Vec<bool>,
    /// per-slot init-flag frame offset (-1 = untracked)
    pub(crate) flag_off: Vec<i32>,
    /// frame offset of the 8-byte tag stack (-1 = no tracking at all)
    pub(crate) tag_base: i32,
    /// lazily created Nil-guard fault thunks: (label, exit code); bodies
    /// emitted at the end of lowering only when used
    pub(crate) nil_thunks: Vec<(usize, u64)>,
    /// frame offset of the first init-flag byte (prologue zeroing)
    pub(crate) nil_flag_start: i32,
    /* ---- task-1 width model ---- */
    /// per-slot joined uint widths (None = bool-ish can flow in; the
    /// legacy uniform-u64 behavior), from [`join_slot_widths`]. Every
    /// lowering path carries the table; a missing entry reads as None.
    pub(crate) slot_w: Vec<Option<u32>>,
    /* ---- narrow-PUT face: runtime width shadow ---- */
    /// slots whose `as bytes` view follows the LAST assignment's width
    pub(crate) dyn_w: Vec<bool>,
    /// their 1-byte shadow's S0-relative frame offset (-1 = unflagged)
    pub(crate) dyn_w_off: Vec<i32>,
    /* ---- P3-V2IMPL: compo-list v2 shapes ---- */
    /// per-slot storage classes + rodata values (from the plan() walk)
    pub(crate) lists: ListPlanData,
    /// frame byte offset of each list's element data block (-1 = none)
    pub(crate) list_data_off: Vec<i32>,
    /// pool label of each list's rodata table, registered at the (single)
    /// creation site (-1 = none yet)
    pub(crate) list_lbl: Vec<usize>,
    /* ---- task 3: u128 support ---- */
    /// label of the shared u128 soft-divmod routine, created lazily at the
    /// first division site and emitted after the body (before the fault
    /// thunks). None when the body never divides at 128 bits.
    pub(crate) divmod_lbl: Option<usize>,
    /// T2: label of the shared u128 mul_mod routine (double-and-add), same
    /// lazy creation/emission pattern as the divmod routine. None when the
    /// body never multiplies-mod at 128 bits.
    pub(crate) mulmod_lbl: Option<usize>,
    /// frame offset of the routine's register save area (-1 = no u128
    /// division anywhere in the body)
    pub(crate) divmod_save_off: i32,
    /// P1 u256 library: label of the shared 256/128 soft-division routine
    /// (`div_u256_by_u128_to_u128`'s emit twin), created lazily at the
    /// first `emit_u256_divmod_u128` / `emit_u256_divmod_u129` site.
    pub(crate) u256div_lbl: Option<usize>,
    /// P1 u256 library: label of the shared 256/129 soft-division routine
    /// (`div_u256_by_u129_to_u128`'s emit twin, divisor bit 128 set).
    pub(crate) u256div129_lbl: Option<usize>,
    /// D3 (fin-control survey): label of the shared 128×128→256 wide-
    /// multiply routine (`mul_wide_u128`'s emit twin), created lazily at
    /// the first `emit_u256_mul_wide` site. Never faults.
    pub(crate) u256mul_lbl: Option<usize>,
    /// D2 (fin-control survey): labels of the shared HalfUp / HalfEven
    /// quotient-rounding routines (`round_quot_u128_with_policy`'s emit
    /// twins), created lazily at the first `emit_u128_round_quot` site of
    /// the respective policy. Fault 90 (checked quo+1 carry) from inside.
    pub(crate) u256round_hu_lbl: Option<usize>,
    pub(crate) u256round_he_lbl: Option<usize>,
    /// D2/D3 amortization gates: construction-time decisions (pure IR
    /// functions computed beside `body_needs_u256`, see
    /// `body_routine_site_counts`) — the shared mul_wide / HalfUp /
    /// HalfEven routines activate only when the body's static site count
    /// of that primitive reaches `FIN_ROUTINE_MIN_SITES` (3); below the
    /// threshold the historical inline form is emitted byte-for-byte, no
    /// label is created and no routine body appears.
    pub(crate) mulw_routine: bool,
    pub(crate) round_hu_routine: bool,
    pub(crate) round_he_routine: bool,
    /// A9 (survey arith.md): the shared u128 multiply routine's gate (same
    /// construction-time rule; `MUL128_ROUTINE_MIN_SITES` in widths.rs) and
    /// its lazily created label — the routine serves BOTH the checked
    /// `emit_u128_mul` sites and the POW loop's `emit_u128_mul_keep` sites,
    /// and shares the divmod frame workspace (s+0..s+56 operands/results,
    /// s+64..s+96 saves; a0 is never written inside, so the D1 `jal a0`
    /// link needs no park).
    pub(crate) mul128_routine: bool,
    pub(crate) mul128_lbl: Option<usize>,
    /// A8 (survey arith.md): the shared sqrt routines' gates (same
    /// construction-time rule; `SQRT_ROUTINE_MIN_SITES` in widths.rs) and
    /// their lazily created labels — one variant per (domain, op): the
    /// u64-domain floor/ceil bodies and the u128 pair-domain floor/ceil
    /// bodies. Each gated site stages sd(operand) + `jal a0` + ld(result)
    /// through the 64-byte frame workspace `sqrt_save_off` (s+0/8 operand
    /// pair, s+16/24 result pair, s+32..s+64 the routine's saved workers);
    /// the w<64 range check and the nil-72 guard stay at the site.
    pub(crate) sqrt64_routine: bool,
    pub(crate) sqrt64up_routine: bool,
    pub(crate) sqrt128_routine: bool,
    pub(crate) sqrt128up_routine: bool,
    pub(crate) sqrt64_lbl: Option<usize>,
    pub(crate) sqrt64up_lbl: Option<usize>,
    pub(crate) sqrt128_lbl: Option<usize>,
    pub(crate) sqrt128up_lbl: Option<usize>,
    /// A8: frame offset of the sqrt routines' workspace (layout in
    /// [`Lower::new_mode_arena_bank`]; -1 = reserved for no sqrt-gated
    /// body — the emit helpers reject then).
    pub(crate) sqrt_save_off: i32,
    /// P1 u256 library: frame offset of the u256 routines' workspace
    /// (layout in [`Lower::new_mode_arena_bank`]; -1 = reserved for no
    /// u256-using site — the emit helpers reject then).
    pub(crate) u256_save_off: i32,
    /// T26: static FIN nesting depth (park-area reservation size) and the
    /// current nesting index while a FIN node stages its operands.
    pub(crate) fin_park_levels: usize,
    pub(crate) fin_depth: usize,
    /// D4: do the shared routines need to save their bank-candidate
    /// working registers (x22/x23/x24/x25/x3/x4)? True iff ImcRemap — the
    /// only mode where the local bank exists (the promotion plan is built
    /// exclusively under `mode == Rv64Mode::ImcRemap` in the lower
    /// pipeline), so those registers can carry live slot values there. In
    /// Im/Imc nothing outside the routines themselves ever writes them
    /// (pool = x5/x6/x7/x28/x29/x30/x31/x14; scratches x15/x16 or
    /// x12/x13; a0/a1 = x10/x11; fixed RA/SP/S0..S5/A7 — no overlap), so
    /// the routines drop those saves/restores (−2 instructions per
    /// register per routine). Mode is a compile-time input → deterministic.
    pub(crate) saves_bank_candidate_regs: bool,
}

pub(crate) enum SliceKind {
    Cut,
    Left,
    Right,
    LDrop,
    RDrop,
}

mod arith;
mod bytes;
mod cast;
mod control;
mod ctr;
mod eval;
mod fin;
mod list;
mod map;
mod stmt;
mod types_test;

#[allow(unused_imports)]
pub(crate) use arith::*;
#[allow(unused_imports)]
pub(crate) use bytes::*;
#[allow(unused_imports)]
pub(crate) use cast::*;
#[allow(unused_imports)]
pub(crate) use control::*;
#[allow(unused_imports)]
pub(crate) use ctr::*;
#[allow(unused_imports)]
pub(crate) use eval::*;
#[allow(unused_imports)]
pub(crate) use fin::*;
#[allow(unused_imports)]
pub(crate) use list::*;
#[allow(unused_imports)]
pub(crate) use map::*;
#[allow(unused_imports)]
pub(crate) use stmt::*;
#[allow(unused_imports)]
pub(crate) use types_test::*;

impl<'a> Lower<'a> {
    /// Carries the Nil-tracking plan (the ImcRemap probe must see the same
    /// guard/tag emission shapes as the real lowering).
    pub(crate) fn new_mode_arena_nil(
        slot: &'a [SlotTy],
        needs_zero: &'a [bool],
        mode: Rv64Mode,
        arena_size: u64,
        nilp: &NilPlan,
        lists: &ListPlanData,
        slotw: &[Option<u32>],
        dynw: &[bool],
        divmod: bool,
        u256: bool,
        fin_park_levels: usize,
        heap: Option<u8>,
        input_local: bool,
        routine_gates: (bool, bool, bool, bool, bool, bool, bool, bool),
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
            slotw,
            dynw,
            divmod,
            u256,
            fin_park_levels,
            heap,
            input_local,
            routine_gates,
        )
    }

    /// `pool_size`/`plan`: the ImcRemap promotion plan computed by
    /// `lower_body_mode_arena` (pool shrunk to the body's peak expression
    /// depth; web segments of hot locals share the freed registers).
    /// `Im`/`Imc` always pass the fixed 8-slot pool and an empty plan.
    /// `lists` is the P3-V2IMPL list storage plan (all-`NotList` for
    /// list-free bodies — the historical layout, byte for byte).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_mode_arena_bank(
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
        slotw: &[Option<u32>],
        dynw: &[bool],
        divmod: bool,
        u256: bool,
        fin_park_levels: usize,
        heap: Option<u8>,
        input_local: bool,
        routine_gates: (bool, bool, bool, bool, bool, bool, bool, bool),
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
                // T13 stain fix: an Address slot's runtime cell is the same
                // (ptr, len) pair shape as a bytes slot
                SlotTy::Bytes | SlotTy::Address => 16,
                SlotTy::Input => 0,
                // task 3: a u128 int slot is a 16-byte cell (lo | hi)
                _ => {
                    if slotw.get(i).copied().flatten() == Some(128) {
                        16
                    } else {
                        8
                    }
                }
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
                SlotTy::Bytes | SlotTy::Address => 16,
                SlotTy::Input => 0,
                _ => {
                    if slotw.get(i).copied().flatten() == Some(128) {
                        16
                    } else {
                        8
                    }
                }
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
        if tracked.iter().any(|&t| t) || nilp.tag_bytes {
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
        // iteration-order dependence). B2 task 4: static lists take
        // N × element-step (the slot's fixed element width); growable
        // lists reserve the fixed LIST_BLOCK_MAX budget (128 × 8B = 1024B
        // at the u64 width — decision D2, byte-identical for u64 slots);
        // rodata / empty lists take nothing (the slot's own 8B cell —
        // allocated in the slot loops above — is the growable length
        // shadow and dead for the static classes).
        let mut list_data_off = vec![-1i32; slot.len()];
        for (i, lk) in lists.kind.iter().enumerate() {
            if i >= slot.len() {
                break;
            }
            let bytes = match lk {
                ListKind::FrameStatic { n } => {
                    (*n as usize) * step_of(lists.ew.get(i).copied())
                }
                ListKind::Growable { .. } => LIST_BLOCK_MAX as usize,
                _ => continue,
            };
            list_data_off[i] = acc;
            acc += bytes as i32;
        }
        // task 3: the shared u128 soft-division routine's frame workspace,
        // S0-relative, written before any read inside the routine so no
        // prologue zeroing is needed. Layout (see emit_divmod_routine):
        //   s+0/s+8    D lo/hi (caller-written), s+16/s+24 V lo/hi
        //   s+32/s+40  quotient lo/hi (routine-written)
        //   s+48/s+56  remainder lo/hi (routine-written)
        //   s+64..128  saved working regs x6/x7/x14/x23/x24/x22/x28/x29
        //              (D4: Im/Imc save only the pool members
        //              x6/x7/x14/x28/x29 — x22/x23/x24 have no writers
        //              outside the routines there; ImcRemap saves all 8)
        //   s+128      the ROUTINE's own link park (D1: the a0 return
        //              address saved at entry, restored before its jr —
        //              the call sites `jal a0` with no parking)
        // T2: the shared u128 mul_mod routine reuses the same area with its
        // own operand/result map (x→s+0/8, y→s+16/24, z→s+32/40, result→
        // s+48/56 — see emit_mulmod128); the two routines are never live
        // at the same time. The area exists only in bodies that actually
        // divide or take add_mod/mul_mod at 128 bits, so u128-free frames
        // stay byte-identical.
        let divmod_save_off: i32 = if divmod {
            let off = acc;
            acc += 136;
            off
        } else {
            -1
        };
        // P1 (§3 shared prework) — the u256 wide-arithmetic library's frame
        // workspace, S0-relative, written before any read inside its
        // routines so no prologue zeroing is needed. Layout (see the
        // `emit_u256_*` callers and `emit_u256_divmod_*_routine`):
        //   s+0..s+32    dividend N limbs N0..N3 (N0 = bits 0..63)
        //   s+32..s+48   divisor V limbs V0,V1 (the u128 below bit 128)
        //   s+48         DH — divisor bit 128 (u129 path; runtime 0 or 1)
        //   s+56..s+72   quotient Q0,Q1 (routine-written)
        //   s+72..s+88   remainder R0,R1 (routine-written, bits 0..127)
        //   s+88         RH — remainder bit 128 (u129 path, routine-written)
        //   s+96..s+192  saved working regs (12 slots; D4: Im/Imc save
        //                only the 6 pool/scratch members — x22..x25/x3/x4
        //                have no writers outside the routines there;
        //                ImcRemap saves all 12. The first slots double as
        //                the T3/T4 triop staging park, dead between calls)
        //   s+192        the ROUTINE's own link park (D1: the a0 return
        //                address saved at entry, restored before its jr —
        //                the call sites `jal a0` with no parking)
        //   s+200..s+296 T26 FIN* composite parks (six u128 slots A..F —
        //                live values held across library calls; never
        //                touched by the P1 routines themselves)
        // 296 bytes, only in bodies that actually use the u256 library, so
        // every existing frame stays byte-identical (see `body_needs_u256`).
        let u256_save_off: i32 = if u256 {
            let off = acc;
            // T26: the static FIN nesting depth sizes the operand-park
            // area behind the kernel slots (same reservation rule in the
            // probe and the real lowering — the layouts must agree).
            acc += U256WS_BYTES + fin_park_levels as i32 * FIN_PARK_LEVEL_BYTES;
            off
        } else {
            -1
        };
        // A8 (survey arith.md): the sqrt routines' frame workspace,
        // S0-relative, written before any read inside the routines so no
        // prologue zeroing is needed. 64 bytes, only in a body where a
        // sqrt variant gated on, so every existing frame stays
        // byte-identical. Layout (see emit_sqrt64/emit_sqrt128 and their
        // routine forms):
        //   s+0/s+8     operand pair (sqrt64: n at s+0; sqrt128: lo/hi)
        //   s+16/s+24   result pair (routine-written; sqrt64: value only)
        //   s+32..s+64  saved working regs (sqrt64: T1/T2/T3 = x6/x7/x28;
        //               sqrt128: T1..T4 = x6/x7/x28/x29 — all pool members
        //               in Im/Imc and pool-or-bank homes in ImcRemap, the
        //               same D4 audit class as the mul128 routine's save
        //               set; sc1/sc2 stay unsaved — dead at every call
        //               site, the A9 premise)
        let sqrt_save_off: i32 = if routine_gates.4 || routine_gates.5 || routine_gates.6 || routine_gates.7 {
            let off = acc;
            acc += 64;
            off
        } else {
            -1
        };
        // task 7: the heap block — the admitted single constant HGROW's
        // `seg × Heap::SEGLEN` bytes, plus the 8-byte grown-flag word in
        // front of it (0 = the heap is empty: the interpreter's heap
        // starts at length 0 and `heap_grow` zero-fills the grown bytes,
        // so the whole region reads as zeros until the grow executes).
        // `acc` is 8-aligned here (every preceding region is a multiple of
        // 8), so the flag word and the segment block (a 256-multiple) stay
        // word-aligned for the prologue's sd zeroing. The region rides the
        // SAME 2040B S0 window budget as the list blocks and the divmod
        // workspace — a body whose reservation does not fit rejects in the
        // prologue's frame check (the heap message names the budget). No
        // heap_grow → no bytes at all (heap-free frames stay byte
        // identical; heap ops without a grow face a length-0 heap with no
        // reservation, exactly like the interpreter before its first grow).
        let heap_flag_off: i32;
        let heap_off: i32;
        let heap_bytes: u32;
        if let Some(seg) = heap {
            heap_flag_off = acc;
            acc += 8;
            heap_off = acc;
            heap_bytes = seg as u32 * crate::space::Heap::SEGLEN as u32;
            acc += heap_bytes as i32;
        } else {
            // no reservation, but keep the base a valid S0-relative offset
            // for the (always-trapping) heap faces of heap-free bodies
            heap_flag_off = acc;
            heap_off = acc;
            heap_bytes = 0;
        }
        // T16: the map-op scratch area (16 words). Body-local: only a body
        // with a map slot reserves it, so every map-free frame stays byte
        // identical. The probe pass and the real lowering both compute it
        // from the same `slot` table (the frame invariant). Cells:
        //   0 kptr, 8 klen, 16 v0, 24 v1, 32 objA, 40 objB, 48 i/obj,
        //   56 lenB, 64.. spare
        let map_scratch_off: i32 = if slot.iter().any(|t| is_map_slot(*t)) {
            let off = acc;
            acc += 128;
            off
        } else {
            -1
        };
        // narrow-PUT face: one runtime width-shadow byte per flagged slot
        // (`FramePlan::dyn_w` — an `as bytes` view of a slot whose write
        // sites carry mixed static widths). Every PUT stores its own width,
        // XOP-class writes only grow, and the slot's `as bytes` view reads
        // the byte — the interpreter's cast_bytes face, replayed. Init 8
        // (the never-assigned slot reads as the u64 zero face; a view
        // before the first PUT faults 71 on Nil on both engines anyway).
        // Flagged slots only, so every existing frame stays byte identical.
        let dyn_w: Vec<bool> = dynw.to_vec();
        let mut dyn_w_off = vec![-1i32; slot.len()];
        if dyn_w.iter().any(|&b| b) {
            for (i, &b) in dyn_w.iter().enumerate() {
                if b {
                    dyn_w_off[i] = acc;
                    acc += 1;
                }
            }
            acc = (acc + 7) & !7;
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
            heap_seg: heap,
            heap_flag_off,
            heap_off,
            heap_bytes,
            map_scratch_off,
            map_kptr_cell: 0,
            map_klen_cell: 8,
            map_v0_cell: 16,
            map_v1_cell: 24,
            dyn_w,
            dyn_w_off,
            pool_entries: Vec::new(),
            pool_bytes: 0,
            pool_map: HashMap::new(),
            break_labels: Vec::new(),
            continue_labels: Vec::new(),
            trap_label,
            fault_arith,
            fault_cast,
            param_bound: false,
            input_local,
            slot_w: slotw.to_vec(),
            fwd_int: vec![None; slot.len()],
            fwd_bytes: vec![None; slot.len()],
            desc_fwd: HashMap::new(),
            map_zero: vec![false; slot.len()],
            cat_top: None,
            s1_gen: 0,
            cat_top_gen: 0,
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
            divmod_lbl: None,
            mulmod_lbl: None,
            divmod_save_off,
            u256div_lbl: None,
            u256div129_lbl: None,
            u256mul_lbl: None,
            u256round_hu_lbl: None,
            u256round_he_lbl: None,
            mulw_routine: routine_gates.0,
            round_hu_routine: routine_gates.1,
            round_he_routine: routine_gates.2,
            mul128_routine: routine_gates.3,
            mul128_lbl: None,
            sqrt64_routine: routine_gates.4,
            sqrt64up_routine: routine_gates.5,
            sqrt128_routine: routine_gates.6,
            sqrt128up_routine: routine_gates.7,
            sqrt64_lbl: None,
            sqrt64up_lbl: None,
            sqrt128_lbl: None,
            sqrt128up_lbl: None,
            sqrt_save_off,
            u256_save_off,
            fin_park_levels,
            fin_depth: 0,
            saves_bank_candidate_regs: mode == Rv64Mode::ImcRemap,
        }
    }

    /* ---- register stack ---- */

    /// P2-A27 S1: open a liveness span for vstack position `pos` at the
    /// current statement (probe recordings only).
    pub(crate) fn span_open_at(&mut self, pos: usize) {
        if !self.record_spans || pos >= 8 {
            return;
        }
        if self.span_open[pos].is_none() {
            self.span_open[pos] = Some(self.web_idx);
        }
    }

    /// P2-A27 S1: close the spans of positions `new_len..old_len` at the
    /// current statement (the values above the shrunk stack just died).
    pub(crate) fn span_close_from(&mut self, new_len: usize, old_len: usize) {
        if !self.record_spans {
            return;
        }
        for pos in new_len..old_len.min(8) {
            if let Some(o) = self.span_open[pos].take() {
                self.pos_spans[pos].push((o, self.web_idx));
            }
        }
    }

    pub(crate) fn push_vreg(&mut self) -> u8 {
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
            // E2 mirror: a forwarded Ref names vregs[j] with j <= its own
            // stack position (fwd_reg_reissuable), so a push — which hands
            // vregs[len], strictly above every live position — can never
            // re-hand (and clobber) a register a live Ref still names. A
            // violation means some Ref was pushed naming a register above
            // its own depth: exactly the unsoundness the read-side rule
            // excludes (the mid-expression re-hand would desync the
            // forwarded value from its slot).
            for (pos, loc) in self.vstack.iter().enumerate() {
                if let VLoc::Ref(rr) = loc {
                    if *rr == r {
                        panic!(
                            "INV push re-hands vreg {r} while a live Ref at pos {pos} still names it"
                        );
                    }
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
    pub(crate) fn kill_fwd_reg(&mut self, r: u8) {
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
        // B9: a re-hand rewrites the register — a live arena-top fact
        // watching it loses its witness (same discipline as fwd_bytes).
        if let Some((p, l)) = self.cat_top {
            if p == r || l == r {
                self.cat_top_kill();
            }
        }
        // C12: a rewritten register invalidates a descriptor watching it
        // (same discipline as fwd_bytes — see `desc_fwd`).
        self.desc_fwd.retain(|_, (p, l)| *p != r && *l != r);
    }

    /* ---- B9 (impl/arena-mapflow): the arena-top fact ---- */

    /// Kill the arena-top fact and advance the bookkeeping epoch. Called at
    /// EVERY arena S1 bump that does not itself establish a fresh arena-top
    /// descriptor, at every join label (`place`), and when a watched
    /// register is re-handed (`kill_fwd_reg`). The S1-bump site census this
    /// discipline must cover (every `add(S1`/`addi(S1`/`mv(S1` write in the
    /// module): emit_memcpy / emit_memcpy_const (killed at entry),
    /// emit_cbytes / emit_cbytes_dyn / emit_cbytes128 (killed at entry,
    /// re-established by the result), emit_itemcat (same), emit_bufset (its
    /// memcpys kill; the result re-establishes), emit_input_local_copy (the
    /// banked arm re-establishes), emit_ctr_map_key_uint (killed at
    /// `emit_map_key_finish`'s entry), ctr_list_alloc / ctr_clone_list /
    /// ctr_map_alloc / emit_map_to_list (killed inside).
    pub(crate) fn cat_top_kill(&mut self) {
        self.s1_gen = self.s1_gen.wrapping_add(1);
        self.cat_top = None;
    }

    /// Establish "(p, l) names a bytes descriptor whose bytes end exactly
    /// at the current arena top: p + l == S1". Both registers hold their
    /// values from here on (a re-hand of either kills the fact via
    /// `kill_fwd_reg`), and S1 must not move without a kill.
    pub(crate) fn cat_top_establish(&mut self, p: u8, l: u8) {
        if std::env::var("RV64_INV_CHECK").is_ok() {
            if p == l {
                panic!("INV B9 arena-top fact: ptr and len registers coincide (x{p})");
            }
            for (pos, loc) in self.vstack.iter().enumerate() {
                if let VLoc::Ref(rr) = loc {
                    if *rr == p || *rr == l {
                        panic!(
                            "INV B9 arena-top fact: watched reg x{} still named by a live Ref at pos {}",
                            if *rr == p { p } else { l },
                            pos
                        );
                    }
                }
            }
        }
        self.cat_top = Some((p, l));
        self.cat_top_gen = self.s1_gen;
    }

    /// Forwarding master switch (diagnostics: `RV64_NO_FWD=1` disables the
    /// store-to-load forwarding while keeping every other path identical).
    pub(crate) fn fwd_slot_ok(&self, slot: usize) -> bool {
        let only = FWD_ONLY_SLOT.load(std::sync::atomic::Ordering::Relaxed);
        only == usize::MAX || only == slot
    }

    /// E2 (encoding-regalloc survey): may a forwarded GET re-issue pool
    /// register `r` as a Ref at the CURRENT stack depth? The slot's value
    /// still lives in `r` — the kill discipline covers every rewriter (a
    /// re-hand in `push_vreg`, bank writes via `kill_fwd_reg`, XOP stores,
    /// and `place`/join kill every entry naming it) — so the ONLY hazard
    /// is a LATER push re-handing `r` while the pushed Ref is still live.
    /// A Ref lands at position `d = vstack.len()`; while it is live the
    /// stack keeps depth ≥ d+1, so pushes hand `vregs[d+1..]` — `r` can be
    /// re-handed (and clobbered) only if it sits ABOVE the current depth.
    /// SOUND IFF `r == vregs[j]` for some `j <= vstack.len()`. The pre-E2
    /// rule (empty stack && r == vregs[0]) is exactly the j == d == 0
    /// corner; bank homes are never pool registers, so a `vregs` hit is
    /// always a forwarding-derived name. The runtime mirror of this
    /// argument is the live-Ref check in `push_vreg`'s INV_CHECK block.
    /// (bytes forwarding keeps its stricter bottom-pair rule — a
    /// generalization there must re-issue BOTH registers of the pair.)
    pub(crate) fn fwd_reg_reissuable(&self, r: u8) -> bool {
        self.vregs.iter().take(self.vstack.len() + 1).any(|&v| v == r)
    }

    pub(crate) fn fwd_enabled(&self) -> bool {
        if !FWD_ENV_READ.swap(true, std::sync::atomic::Ordering::Relaxed) {
            if diag_env_enabled("RV64_NO_FWD") {
                FWD_DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        !FWD_DISABLED.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Join point / branch target: every path merging here can carry a
    /// different register history — forwarding state dies wholesale.
    /// B9/B14: the arena-top and len-zero facts are straight-line dataflow
    /// — a merged path may carry a different S1 history / a map object
    /// mutated on another arm, so both die here with the fwd tables.
    /// C12 joins them: an entry established on ONE arm of a CHOOSE/if
    /// does not dominate the join (the other arm's path carries a
    /// different descriptor in the same registers), so the whole table
    /// dies at every label barrier. This is what keeps the hit face
    /// basic-block-local — see `desc_fwd`.
    pub(crate) fn place(&mut self, l: usize) {
        self.fwd_int.fill(None);
        self.fwd_bytes.fill(None);
        self.desc_fwd.clear();
        self.map_zero.fill(false);
        self.cat_top_kill();
        self.asm.place(l);
    }

    /// Is the store to `slot` at the current statement dead (no later read
    /// of the slot exists anywhere in the remaining timeline)? Loop-carried
    /// slots are exempt: the backedge re-reads writes the linear timeline
    /// cannot see.
    pub(crate) fn store_is_dead(&self, slot: u8) -> bool {
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
    pub(crate) fn nil_val(&self, node: &dyn IRNode) -> bool {
        self.nil_occ.get(&node_addr(node)).map(|o| o.val).unwrap_or(false)
    }

    /// Lazily create (or find) the fault thunk for a Nil-guard exit code.
    pub(crate) fn nil_thunk(&mut self, code: u64) -> usize {
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
    pub(crate) fn pair_sibling<'s>(&self, subs: &'s [Box<dyn IRNode>], i: usize) -> Option<&'s dyn IRNode> {
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
    pub(crate) fn emit_pair(&mut self, rm: &dyn IRNode, ins: &dyn IRNode) -> Ret<()> {
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
        // T17: the plan walk may have promoted this slot to the arena
        // class AFTER the pair was recognized (a later take/insert/merge
        // forces the descriptor model). The fused flat-block overwrite
        // form does not apply then — replay the two real ops (the index
        // expression is pure and receiver-free by the pair contract, so
        // its second evaluation is unobservable, exactly as on the
        // interpreter's two separate instructions).
        if matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
            self.emit_remove(s, &*rn.suby)?;
            return self.emit_insert(s, &*nn.suby, &*nn.subz);
        }
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
                // C2: the extract_u32 72 guard is dead for a width-provable
                // (≤ 32) index — see idx_below_2pow32; the 85 remove bound
                // keeps its position (before the value evaluation, r4d)
                if !self.idx_below_2pow32(&*rn.suby) {
                    let l72 = self.nil_thunk(72);
                    self.asm.li(sc1, 1u64 << 32);
                    self.asm.bgeu(kr, sc1, l72); // extract_u32: ≥ 2³² → 72
                }
                self.asm.ld(sc1, S0, cell); // len shadow
                self.asm.bgeu(kr, sc1, l85); // remove bound: idx ≥ len → 85
                // the value evaluation runs AFTER the 85 guard: the
                // interpreter's remove traps before the insert arguments
                // are evaluated (r4d — fault-injection pin, doc 98 §Method)
                self.eval_int(&*nn.subz)?;
                let vd = self.vstack.len() - 1;
                let base = self.list_data_off[s];
                debug_assert!(base >= 0, "list data block missing from the frame plan");
                // B2 task 4: stride and store width follow the slot's
                // fixed element width
                let step = self.list_step(s);
                let sh = step.trailing_zeros();
                if sh > 0 {
                    self.asm.slli_(sc2, kr, sh); // idx * step
                } else {
                    self.asm.mv(sc2, kr);
                }
                self.asm.addi(sc1, S0, base);
                self.asm.add(sc1, sc1, sc2);
                let v = loc_reg(self.vstack[vd]);
                self.list_store_inst(v, sc1, 0, step);
                self.pop_vreg(); // value
                self.pop_vreg(); // idx
            }
            Some(k) => {
                // constant index — static faces first (the interpreter
                // faults 72 BEFORE 85; BUG-028's runtime-guard lesson
                // applies to the guarded form: a Growable constant index
                // checks the live length)
                if k >= 1u64 << 32 {
                    let l72 = self.nil_thunk(72);
                    self.asm.jal_(ZERO, l72);
                    return Ok(());
                }
                // B2 task 4: the capacity is the block budget divided by
                // the element stride (128 for the u64 width — the
                // historical form, byte for byte)
                let step = self.list_step(s);
                let capacity = (LIST_BLOCK_MAX / step as u64) as i32;
                if k >= capacity as u64 {
                    // block-budget invariant (len ≤ capacity at every write
                    // point) makes any k ≥ capacity an unconditional 85 —
                    // no load, no offset arithmetic wrap face
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
                let off = base + (k as i32) * (step as i32);
                let v = loc_reg(self.vstack[vd]);
                self.list_store_inst(v, S0, off, step);
                self.pop_vreg(); // value
            }
        }
        Ok(())
    }


    /// Fault when the tag byte of the value at vstack position `depth`
    /// says Nil.
    pub(crate) fn nil_guard_tag(&mut self, depth: usize, code: u64) {
        let t = self.nil_thunk(code);
        let sc = self.guard_scratch();
        self.asm.lbu(sc, S0, self.tag_base + depth as i32);
        self.asm.beq(sc, ZERO, t);
    }

    /// Fault when slot `s`'s init flag says unassigned (XOP old value,
    /// fused comparisons reading the slot directly).
    pub(crate) fn nil_guard_flag(&mut self, s: u8, code: u64) {
        let t = self.nil_thunk(code);
        let sc = self.guard_scratch();
        self.asm.lbu(sc, S0, self.flag_off[s as usize]);
        self.asm.beq(sc, ZERO, t);
    }

    /// E9 (encoding-regalloc survey): the scratch register for a nil
    /// guard's `lbu` + zero-compare `beq`. The branch compresses to
    /// c.beqz (2B) only when rs1 is window-resident — the dedicated
    /// scratch is x30 in ImcRemap, outside the window, so every guard
    /// site paid the full 4B B-type form. At an EMPTY stack no pool
    /// register holds an operand, so the first pool register is
    /// free-but-holding: borrow it (window-resident in ImcRemap — the
    /// arith.rs XOP-frame precedent) and let the branch compress. The
    /// borrow clobbers the register with the tag byte, so forwarding
    /// entries naming it die first (`kill_fwd_reg`) — their readers
    /// re-load from the frame. With a live stack every pool register may
    /// hold an operand or a forwarded Ref: fall back to the dedicated
    /// scratch (unchanged behavior). Non-RVC modes gain nothing from a
    /// borrow (no compression at all), and Imc's vregs[0] (x5) sits
    /// outside the window — both keep the dedicated scratch.
    fn guard_scratch(&mut self) -> u8 {
        if self.vstack.is_empty() && self.asm.rvc {
            if let Some(&r) = self.vregs.first() {
                if rvc_win(r) {
                    self.kill_fwd_reg(r);
                    return r;
                }
            }
        }
        self.asm.sc1
    }

    /// EQ/NEQ semantics: fault 90 iff EXACTLY one side is Nil (two Nils
    /// compare equal on the interpreter) — the tags differ.
    pub(crate) fn nil_guard_eq_tags(&mut self, dx: usize, dy: usize) {
        let t = self.nil_thunk(90);
        let (s1, s2) = (self.asm.sc1, self.asm.sc2);
        self.asm.lbu(s1, S0, self.tag_base + dx as i32);
        self.asm.lbu(s2, S0, self.tag_base + dy as i32);
        self.asm.bne(s1, s2, t);
    }

    /// GET of a tracked may-Nil slot: publish the slot's Nil bit as the
    /// value's tag at vstack position `depth`.
    pub(crate) fn nil_wtag_from_flag(&mut self, s: u8, depth: usize) {
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.flag_off[s as usize]);
        self.asm.sb(sc, S0, self.tag_base + depth as i32);
    }

    /// Copy a tag byte between vstack positions (CHOOSE select).
    pub(crate) fn nil_copy_tag(&mut self, from: usize, to: usize) {
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.tag_base + from as i32);
        self.asm.sb(sc, S0, self.tag_base + to as i32);
    }

    /// Publish a proven non-Nil tag byte (1) at vstack position `depth`
    /// (CHOOSE select of a mechanism-less arm, BUG-019: the arm's
    /// evaluation writes no tag bytes, and the stale byte at the result
    /// position — prologue zero reads as "Nil" — must not survive into a
    /// position the consumer's guard will read).
    pub(crate) fn nil_set_tag_one(&mut self, depth: usize) {
        let sc = self.asm.sc1;
        self.asm.addi(sc, ZERO, 1);
        self.asm.sb(sc, S0, self.tag_base + depth as i32);
    }

    /// Publish the Nil tag (0) at vstack position `depth` (a `nil` literal).
    pub(crate) fn nil_set_tag_zero(&mut self, depth: usize) {
        self.asm.sb(ZERO, S0, self.tag_base + depth as i32);
    }

    /// PUT into a tracked slot from a may-Nil source: the slot's flag
    /// copies the source value's tag (Nil-ness propagates through
    /// assignment, exactly like the interpreter's slot stores).
    pub(crate) fn nil_set_flag_from_tag(&mut self, s: u8, depth: usize) {
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, self.tag_base + depth as i32);
        self.asm.sb(sc, S0, self.flag_off[s as usize]);
    }

    /// PUT into a tracked slot from a provably non-Nil source.
    pub(crate) fn nil_set_flag_one(&mut self, s: u8) {
        let sc = self.asm.sc1;
        self.asm.addi(sc, ZERO, 1);
        self.asm.sb(sc, S0, self.flag_off[s as usize]);
    }

    /// Consumer guards for the int binary arm / branch comparison arm:
    /// EQ/NEQ fault on exactly-one-Nil (90); ordered comparisons fault on
    /// any Nil (90); arithmetic faults on any Nil (71 — the interpreter's
    /// to_uint runs before the op). Called with both operands evaluated,
    /// before any pops: x sits at `len-2`, y at `len-1`.
    pub(crate) fn nil_guard_int_pair(&mut self, op: Bytecode, lx: bool, ly: bool) {
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
    pub(crate) fn push_ref(&mut self, r: u8) {
        self.max_depth = self.max_depth.max(self.vstack.len() + 1);
        self.span_open_at(self.vstack.len());
        self.vstack.push(VLoc::Ref(r));
    }

    pub(crate) fn pop_vreg(&mut self) -> VLoc {
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
    pub(crate) fn pop_reg(&mut self) -> u8 {
        loc_reg(self.pop_vreg())
    }

    /// The in-place-update pattern (NOT/casts/POW): pop the operand, return
    /// its owned pool register WITH the value already pushed back on top —
    /// the consumer overwrites that register in place.
    pub(crate) fn pop_own_replace(&mut self) -> u8 {
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

    /// Whether `op` at width `w` may take the E3 swapped-destination form.
    /// ONLY these — LE/GE/SUB/LT/GT and the div/mod family are
    /// operand-ordered (the operand order is part of the encoding —
    /// invariant 2), so they keep pop2_dest's materializing `mv` even when
    /// both operands are banked.
    ///
    /// MUL at w == 64 is excluded: the A19 mul-check merge
    /// (`mul_check_base_const`) classifies a `mulhu` site by the copy
    /// `mv d, base` feeding it, and the swapped form reads the bank home
    /// directly — every dominated check on that shape stops merging,
    /// which costs more (the whole cascaded guard pair stays emitted) than
    /// the 2B mv saves. Narrow MUL (w < 64) has no mulhu face, so the
    /// swap applies there.
    pub(crate) fn commutative_swap(op: Bytecode, w: u32) -> bool {
        use Bytecode::*;
        if op == MUL && w == 64 {
            return false;
        }
        matches!(op, ADD | MUL | BXOR | BOR | BAND | AND | OR | EQ | NEQ)
    }

    /// Pop the top two operands of a binary op `op` and return
    /// `(d, ra_read, rb_read, swapped)`. `d` is the destination pool
    /// register holding the left value: the left operand's pool register
    /// reused when it was Pool, or a fresh register loaded with `mv` when
    /// it was a bank reference — EXCEPT for a commutative `op` (E3), where
    /// the `mv` is unnecessary: the caller emits the swapped form
    /// `d = rb OP ra` (`emit_arith_commut`) with the left value still in
    /// its bank home `ra`, and `swapped` tells it so. `rb` is only ever
    /// read there, so it may itself be a bank reference (Ref+Ref shape).
    pub(crate) fn pop2_dest(&mut self, op: Bytecode, w: u32) -> (u8, u8, u8, bool) {
        let rb = self.pop_reg();
        let la = self.pop_vreg();
        let d = self.push_vreg();
        match la {
            VLoc::Pool(r) => {
                debug_assert_eq!(d, r);
                (d, r, rb, false)
            }
            VLoc::Ref(r) => {
                // d can BE r when the ref names the pool register the
                // shrunk stack just re-handed (forwarded GET): the value is
                // already in place
                if d == r {
                    (d, r, rb, false)
                } else if Self::commutative_swap(op, w) {
                    (d, r, rb, true)
                } else {
                    self.asm.mv(d, r);
                    (d, r, rb, false)
                }
            }
        }
    }

    /* ---- misc ---- */

    pub(crate) fn slot_offset(&self, idx: u8) -> i32 {
        let off = self.frame_off.get(idx as usize).copied().unwrap_or(-1);
        assert!(off >= 0, "rv64 lower: local slot {idx} not in the frame plan");
        off
    }

    /* ---- narrow-PUT face: the runtime width shadow ---- */

    /// Whether slot `idx` replays its assignment widths into the shadow.
    pub(crate) fn dyn_w_flag(&self, idx: u8) -> bool {
        self.dyn_w.get(idx as usize).copied().unwrap_or(false)
    }

    /// Shadow store for a plain assignment (PUT/PUTX/select arm): the
    /// interpreter's `locals.save` keeps the STORED value's own width tag,
    /// so the shadow becomes exactly `bits` (narrowing allowed — the one
    /// place the static join can never be replayed). `bits` is the static
    /// width of the assigned expression; non-uint widths (bool/nil flows)
    /// never flag a slot, so a None/1/128 code cannot reach a shadow.
    pub(crate) fn shadow_put(&mut self, idx: u8, bits: Option<u32>) {
        if !self.dyn_w_flag(idx) {
            return;
        }
        let nb = match bits {
            Some(8) => 1,
            Some(16) => 2,
            Some(32) => 4,
            Some(64) => 8,
            _ => return,
        };
        let off = self.dyn_w_off[idx as usize];
        self.asm.addi(self.asm.sc1, ZERO, nb);
        self.asm.sb(self.asm.sc1, S0, off);
    }

    /// Shadow grow for a compound write (XOP / max-min): the interpreter's
    /// arithmetic stores at the slot/rhs COMMON width — `max(current, rhs)`
    /// — so the shadow only ever grows, by the rhs's own static width.
    pub(crate) fn shadow_grow(&mut self, idx: u8, bits: Option<u32>) {
        if !self.dyn_w_flag(idx) {
            return;
        }
        let nb = match bits {
            Some(8) => 1,
            Some(16) => 2,
            Some(32) => 4,
            Some(64) => 8,
            _ => return,
        };
        if nb == 1 {
            // the shadow is ≥ 1 after its first store (decl PUT); a u8 rhs
            // never grows it
            return;
        }
        let off = self.dyn_w_off[idx as usize];
        // if shadow < nb { shadow = nb } — the shadow cell is never wider
        // than the 8-byte cell (a u128 slot never flags), so nb = 8 stores
        // unconditionally
        if nb == 8 {
            self.asm.addi(self.asm.sc1, ZERO, 8);
            self.asm.sb(self.asm.sc1, S0, off);
            return;
        }
        let sc = self.asm.sc1;
        self.asm.lbu(sc, S0, off);
        self.asm.addi(self.a0, ZERO, nb);
        let skip = self.asm.new_label();
        self.asm.bgeu(sc, self.a0, skip);
        self.asm.sb(self.a0, S0, off);
        self.place(skip);
    }

    /// Write-side int home (inclusive at segment boundaries).
    pub(crate) fn bank_int(&self, idx: u8) -> Option<u8> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Int) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, true)?;
        Some(h[0])
    }

    /// Read-side int home (exclusive at segment boundaries).
    pub(crate) fn bank_int_read(&self, idx: u8) -> Option<u8> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Int) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, false)?;
        Some(h[0])
    }

    /// Read-side bytes home.
    pub(crate) fn bank_bytes(&self, idx: u8) -> Option<(u8, u8)> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Bytes) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, false)?;
        Some((h[0], h[1]))
    }

    /// Write-side bytes home.
    pub(crate) fn bank_bytes_write(&self, idx: u8) -> Option<(u8, u8)> {
        if self.plan.banked.get(idx as usize).copied() != Some(BankKind::Bytes) {
            return None;
        }
        let h = self.plan.home_at(idx as usize, self.web_idx, true)?;
        Some((h[0], h[1]))
    }


    /// Exit-code-1 trap (assert/abort): inline li+jal, rare enough. Sites
    /// with a fixed 90/71 exit code jump straight to the shared thunks.
    pub(crate) fn trap(&mut self, code: u64) {
        self.asm.li(A0, code);
        self.asm.jal_(ZERO, self.trap_label);
    }

    /// Materialize the constant `v` into `rd`. Large constants (≥2048, the
    /// lui+addi pair) come from the read-only pool via the pool-base
    /// register when one is active: one 4-byte `ld` instead of a 6-byte
    /// pair, deduplicated across every re-materialization site. E1: the
    /// entry is 8-aligned in the pool, so sites whose offset fits the
    /// c.ldsp window ([0,504], base x2) load in 2 bytes.
    pub(crate) fn li_k(&mut self, rd: u8, v: u64) {
        // Only values beyond one lui+addi pair justify the pool entry (an
        // 8-byte pool literal amortized against 2 saved bytes per site is
        // a wash for ≤0xFFFF constants — HIP5's popcount masks lose money
        // pooling).
        if v > 0xFFFF {
            if let Some((b, bl)) = self.pool_base {
                // little-endian: the load reads a raw 64-bit word
                let (lbl, off) = self.lit_label_value(&v.to_le_bytes());
                self.asm.ld_label_off(rd, b, bl, lbl, off);
                return;
            }
        }
        self.asm.li(rd, v);
    }

    /// Address of a pool literal into `rd`: the shared base register when
    /// available (c.addi), else an auipc+addi pair.
    pub(crate) fn lit_addr(&mut self, rd: u8, lbl: usize) {
        if let Some((b, bl)) = self.pool_base {
            self.asm.addi_label_off(rd, b, bl, lbl);
        } else {
            self.asm.pcrel_addr(rd, lbl);
        }
    }

    /// C12 (impl/cse-dse-bank): push the (ptr, len) pool pair for a
    /// bytes-literal `data`. When the pool label was already materialized
    /// into exactly the pair this site would push — (vregs[d], vregs[d+1])
    /// at the current stack depth, i.e. the establishing site ran at the
    /// same depth and no re-hand of either register happened since — the
    /// two registers are re-issued as zero-cost Refs and `lit_addr` + `li`
    /// (2-3 instructions) disappear. Re-issue soundness is the E2 rule: a
    /// Ref at position `d` naming `vregs[d]` can only be clobbered by a
    /// push at position ≥ d, and pushes hand `vregs[d+1..]` — never the
    /// watched registers — while the Refs are live. Registers are rewritten
    /// off-stack only through `push_vreg` re-hands or bank writes, both of
    /// which kill the entry via `kill_fwd_reg`; `place` clears the table at
    /// every join. (Value-pool exhaustion keeps the fresh path: a hit needs
    /// two more pool slots.)
    ///
    /// Trigger face (pinned by cse_dse_bank_tests): the hit is
    /// basic-block-local — consecutive same-label materializations at the
    /// same stack depth with no label barrier and no intervening push
    /// between them. Two shapes cover everything that fires today: the
    /// same-slot literal rebind (`a = "X"` twice in a row, all modes) and
    /// consecutive distinct-slot binds on the frame path (`a = "X"; b =
    /// "X"` — the second site's eval runs at the same depth 0). The
    /// survey's motivating shape — the same 21-byte address constant
    /// compared at several `data == "Addr.."` sites — does NOT fire and
    /// must not: each bytes-compare materializes its descriptor pair,
    /// then POPS both operands before its internal branch targets, so
    /// every one of those joins is a `place` that drops the entry (the
    /// pair is no longer stack-resident, and keeping below-stack entries
    /// across a join is unsound — a CHOOSE arm re-materializing a
    /// different literal into the same registers would leave the entry
    /// stale on the first arm's path; the entry would need real
    /// dominance tracking, out of C12's surveyed scope: "生命周期规则照搬
    /// fwd_bytes（重发号/标签屏障即杀）").
    pub(crate) fn eval_lit_bytes_pair(&mut self, data: &[u8]) -> (u8, u8) {
        let lbl = self.lit_label(data);
        if let Some(&(p, l)) = self.desc_fwd.get(&lbl) {
            let d = self.vstack.len();
            if d + 1 < self.vregs.len() && p == self.vregs[d] && l == self.vregs[d + 1] {
                if std::env::var("RV64_INV_CHECK").is_ok() {
                    if p == l {
                        panic!("INV C12: descriptor pair registers coincide (x{p})");
                    }
                    // runtime mirror of the plan-level busy test: the reused
                    // registers must not be live bank homes at this statement
                    // (segments are born by writes, which kill the entry — a
                    // violation means a missed kill path).
                    for (reg, s0, s1, init) in &self.plan_reg_live {
                        let hit = if *init {
                            *s0 <= self.web_idx && self.web_idx <= *s1
                        } else {
                            *s0 < self.web_idx && self.web_idx <= *s1
                        };
                        if (*reg == p || *reg == l) && hit {
                            panic!(
                                "INV C12: reuse names live bank home reg={reg} at stmt {} (seg [{s0},{s1}] init={init})",
                                self.web_idx
                            );
                        }
                    }
                }
                self.push_ref(p);
                self.push_ref(l);
                return (p, l);
            }
        }
        let rp = self.push_vreg();
        let rl = self.push_vreg();
        self.lit_addr(rp, lbl);
        self.asm.li(rl, data.len() as u64);
        self.desc_fwd.insert(lbl, (rp, rl));
        (rp, rl)
    }

    pub(crate) fn lit_label(&mut self, bytes: &[u8]) -> usize {
        self.lit_label_inner(bytes, false).0
    }

    /// E1: pooled 8-byte VALUE entries (`li_k` constants) are 8-aligned
    /// inside the pool with emission-time-determined zero padding, so the
    /// pool-base load can take the 2-byte `c.ldsp` whenever the — then
    /// already fixed — offset sits in its [0,504] window. Returns (label,
    /// pool-relative content offset). The alignment decision is made HERE,
    /// a pure function of the pool contents accumulated so far, so every
    /// relaxation pass and the pool layout in `lower_body_mode_arena`
    /// agree on it by construction.
    fn lit_label_value(&mut self, bytes: &[u8]) -> (usize, usize) {
        self.lit_label_inner(bytes, true)
    }

    fn lit_label_inner(&mut self, bytes: &[u8], value_entry: bool) -> (usize, usize) {
        if let Some(&(lbl, off)) = self.pool_map.get(bytes) {
            return (lbl, off);
        }
        // E6 suffix sharing: a bytes-class address reference whose bytes
        // are exactly the suffix of an existing entry points into that
        // entry's tail instead of paying a duplicate 8B×n pool segment.
        // The scan runs in entry order (deterministic; the first hit
        // wins — all hits carry the identical suffix bytes, so the choice
        // only shifts which entry hosts the interior label). Value
        // entries are excluded on both sides of the relation (see
        // `PoolEnt::is_value`); exact matches were already handled above.
        if !value_entry {
            let hit = self.pool_entries.iter().position(|ent| {
                !ent.is_value && ent.bytes.len() > bytes.len() && ent.bytes.ends_with(bytes)
            });
            if let Some(i) = hit {
                let delta = self.pool_entries[i].bytes.len() - bytes.len();
                let off = self.pool_entries[i].off + delta;
                let lbl = self.asm.new_label();
                self.pool_entries[i].interior.push((lbl, delta));
                self.pool_map.insert(bytes.to_vec(), (lbl, off));
                return (lbl, off);
            }
        }
        let lbl = self.asm.new_label();
        // Pad only when a compressed load can actually fire: a pool base
        // must be active (Im/Imc pools stay byte-identical — no base
        // register, no c.ldsp) and the aligned offset must still fit the
        // 504-byte c.ldsp window (entries past it fall back to the
        // alignment-agnostic 4-byte `ld`, so padding them is pure pool
        // growth).
        let pad = if value_entry && self.pool_base.is_some() {
            let aligned = (self.pool_bytes + 7) & !7;
            if aligned <= 504 { aligned - self.pool_bytes } else { 0 }
        } else {
            0
        };
        let off = self.pool_bytes + pad;
        self.pool_map.insert(bytes.to_vec(), (lbl, off));
        self.pool_entries.push(PoolEnt {
            lbl,
            bytes: bytes.to_vec(),
            off,
            is_value: value_entry,
            interior: Vec::new(),
        });
        self.pool_bytes = off + bytes.len();
        (lbl, off)
    }


    /// Top of stack: bytes descriptor → copy to output, `a0 = len`, `ret`.
    /// (D1 keeps `ret` everywhere: routine calls link through the
    /// mode-scratch a0 and never touch RA, so the kernel-entry RA
    /// survives the whole body untouched.)
    pub(crate) fn emit_return(&mut self) -> Ret<()> {
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
