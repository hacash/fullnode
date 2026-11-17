use super::*;


/// Extract a bytes literal from a node (PBUF/PBUFL payload, PNBUF → empty).
pub(crate) fn bytes_literal(node: &dyn IRNode) -> Option<Vec<u8>> {
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

pub(crate) fn lxop_inst(op: LxOp) -> Bytecode {
    match op {
        LxOp::Add => Bytecode::ADD,
        LxOp::Sub => Bytecode::SUB,
        LxOp::Mul => Bytecode::MUL,
        LxOp::Div => Bytecode::DIV,
    }
}

/* ------------------------------------------------------------------ */
/* Task-1 width model: the interpreter's dynamic uint widths, mirrored */
/* statically. `arithmetic_args2` (value/cast.rs) widens both sides of */
/* an integer binop to the wider one before computing; literals/casts  */
/* carry intrinsic widths; `byte()` is U8, `size()` U16, list length   */
/* U32, list elements U64.                                            */
/* ------------------------------------------------------------------ */

/// Max-join of two optional widths. `Some(1)` models a PURE-bool subtree
/// (comparisons, `!`/`&&`/`||`, `has_key`, bool literals — the interpreter
/// renders them as exactly one canonical 0/1 byte through `as bytes`);
/// mixing `Some(1)` with a real width, or anything with `None`, yields
/// `None` — a dynamically-widthed value the static model cannot pin.
pub(crate) fn widen2(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(1), Some(1)) => Some(1),
        (Some(x), Some(y)) if x > 1 && y > 1 => Some(x.max(y)),
        _ => None,
    }
}

/// Static uint width (bits) of an int-valued node. `slotw` carries the
/// per-slot joined widths from [`join_slot_widths`]; a missing entry reads
/// as None (no static width — the `as bytes` consumer rejects, arithmetic
/// keeps the legacy uniform-u64 behavior via the None/1 → 64 fallback).
/// Nil-guarded slots fault 71 before any width question matters.
pub(crate) fn node_int_width(node: &dyn IRNode, slot: &[SlotTy], slotw: &[Option<u32>], ew: &[u32]) -> Option<u32> {
    node_int_width_with(node, slot, &slot_width_lookup(slot, slotw, ew))
}

/// B2 task 4: the width lookup behind the oracle's closures — an Int slot
/// reads the joined slot width; a List slot reads its fixed ELEMENT width
/// (an unset entry can only be a never-populated Empty list, whose reads
/// fault 85 before any width face — the conservative u64 default is safe).
pub(crate) fn slot_width_lookup<'a>(
    slot: &'a [SlotTy],
    slotw: &'a [Option<u32>],
    ew: &'a [u32],
) -> impl Fn(usize) -> Option<u32> + 'a {
    move |i| match slot.get(i) {
        Some(SlotTy::List) => match ew.get(i).copied() {
            Some(w) if w > 0 => Some(w),
            _ => Some(64),
        },
        _ => slotw.get(i).copied().flatten(),
    }
}

/// Whole-body slot-width join: every static write's expression width,
/// joined by max per slot, iterated to a fixpoint so a read that
/// textually precedes a wider write (a loop-carried rebind inside the
/// same body) settles on the final width. Result entry per slot:
/// `Some(w)` = every value the slot can hold fits `w` bits;
/// `None` = a bool-ish value can flow in (no uint width — consumers
/// keep the legacy uniform-u64 behavior there).
/// Static scan: does this body contain a u128 division or modulo (DIV/MOD
/// at width 128, or a compound `/=` on a u128 slot)? The shared soft-division
/// routine's frame scratch area is reserved at construction time (the probe
/// pass and the real lowering must see the identical frame layout), so the
/// answer must be computable before any `Lower` exists.
pub(crate) fn body_needs_divmod(
    nodes: &[Box<dyn IRNode>],
    slot: &[SlotTy],
    slotw: &[Option<u32>],
    ew: &[u32],
) -> bool {
    fn walk(n: &dyn IRNode, slot: &[SlotTy], slotw: &[Option<u32>], ew: &[u32]) -> bool {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*w.node, slot, slotw, ew);
        }
        if let Ok(op) = Bytecode::try_from_u8(n.bytecode()) {
            match op {
                Bytecode::DIV | Bytecode::MOD => {
                    if node_int_width(n, slot, slotw, ew) == Some(128) {
                        return true;
                    }
                }
                // T2: a 128-bit-wide add_mod/mul_mod needs the same frame
                // workspace — add_mod lowers two shared-divmod calls and
                // mul_mod passes operands/results through the area to its
                // own routine (the divmod routine itself is emitted only
                // when a divmod call site asks for it).
                Bytecode::ADDMOD | Bytecode::MULMOD => {
                    if node_int_width(n, slot, slotw, ew) == Some(128) {
                        return true;
                    }
                }
                // T6: DIVUP/DIVEXACT at u128 ride the same shared routine
                Bytecode::DIVUP | Bytecode::DIVEXACT => {
                    if node_int_width(n, slot, slotw, ew) == Some(128) {
                        return true;
                    }
                }
                Bytecode::XOP => {
                    if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                        let (lxop, idx) = decode_local_operand_mark(p.para);
                        if lxop == LxOp::Div {
                            if let Some(Some(128)) = slotw.get(idx as usize) {
                                if matches!(slot.get(idx as usize), Some(SlotTy::Int)) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                // T8: the high-slot local compound fragment — a `/=` on a
                // u128 slot needs the soft-divmod save area like XOP's
                Bytecode::IRLIST
                    if n
                        .as_any()
                        .downcast_ref::<IRNodeArray>()
                        .map(|a| local_compound_assign_parts(&*a).is_some())
                        .unwrap_or(false) =>
                {
                    let (_, idx, aop) = local_compound_assign_parts(n).unwrap();
                    if aop == Bytecode::DIV {
                        if let Some(Some(128)) = slotw.get(idx as usize) {
                            if matches!(slot.get(idx as usize), Some(SlotTy::Int)) {
                                return true;
                            }
                        }
                    }
                }
                // T19: the compound item assignment `l[i] /= v` on a u128
                // element slot runs the shared soft-divmod routine too —
                // the raw fragment's DIV is not a walked child (children()
                // does not descend into IRNodeBytecodes), so the DIV arm
                // above cannot see it. The receiver GET names the slot;
                // its fixed element width decides.
                Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR => {
                    if let Some((_value, recv, _index, aop)) = compound_assign_parts(n) {
                        if aop == Bytecode::DIV {
                            if let Some(i) = slot_get_index(recv) {
                                if ew.get(i as usize).copied() == Some(128) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        children(n).into_iter().any(|c| walk(c, slot, slotw, ew))
    }
    nodes.iter().any(|n| walk(&**n, slot, slotw, ew))
}

/// P1 (§3 shared prework, u256 wide-arithmetic library): does this body use
/// the u256 library? The library's frame workspace (`u256_save_off`, see
/// [`Lower::new_mode_arena_bank`]) is reserved at construction time — the
/// probe pass and the real lowering must see the identical frame layout —
/// so the answer must be computable before any `Lower` exists.
///
/// T3/T4 lowered onto the library: MULADD/MULSUB at EVERY width (the
/// interpreter's 256-bit x·y±z intermediate), and MULDIV/MULDIVUP at the
/// u64 / u128 common widths (256-bit product ÷ the shared 256/128 routine;
/// the <= 32 widths stay on the inline single-limb path). The T26 FIN* task
/// (and any other `instruction.rs` u256-intermediate lowering) extends this
/// scan the same way — the u256 emit helpers reject at emission time
/// (`u256_save_off < 0`) when the reservation is missing.
pub(crate) fn body_needs_u256(
    nodes: &[Box<dyn IRNode>],
    slot: &[SlotTy],
    slotw: &[Option<u32>],
    ew: &[u32],
) -> bool {
    fn walk(n: &dyn IRNode, slot: &[SlotTy], slotw: &[Option<u32>], ew: &[u32]) -> bool {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*w.node, slot, slotw, ew);
        }
        if let Ok(op) = Bytecode::try_from_u8(n.bytecode()) {
            match op {
                // T3 mul_add / mul_sub: the 256-bit intermediate is lowered
                // through the u256 library at every width.
                Bytecode::MULADD | Bytecode::MULSUB => return true,
                // T4 mul_div / mul_div_up at the u64 and u128 common widths
                // ride the 256/128 soft-division routine.
                Bytecode::MULDIV | Bytecode::MULDIVUP => {
                    if matches!(
                        node_int_width(n, slot, slotw, ew),
                        Some(64) | Some(128)
                    ) {
                        return true;
                    }
                }
                _ => {}
            }
            // T26: every FIN* node lowers through the u256 library (the
            // kernels stage their wide intermediates through the workspace;
            // even the pair-only kernels — quantize/sat_*/u64_mad/abs_diff_
            // lte — use the FIN park slots that live in the same
            // reservation). The scan is deliberately coarse: any FIN node
            // reserves the (frame-only) workspace, so probe and lowering
            // always agree on the frame layout.
            if matches!(
                op,
                Bytecode::FIN2
                    | Bytecode::FIN3
                    | Bytecode::FIN4
                    | Bytecode::FINP3
                    | Bytecode::FINP4
                    | Bytecode::FINPOW3
            ) {
                return true;
            }
        }
        let _ = (slot, slotw, ew); // extension point: match the lowering sites here
        children(n).into_iter().any(|c| walk(c, slot, slotw, ew))
    }
    nodes.iter().any(|n| walk(&**n, slot, slotw, ew))
}

/// T26: the body's static FIN nesting depth (0 = no FIN node). An outer
/// FIN node parks its already-evaluated operands while the remaining
/// operand expressions evaluate, including nested FIN calls, so the frame
/// reserves one 4-slot park area per level on the deepest FIN chain.
pub(crate) fn fin_nesting_depth(nodes: &[Box<dyn IRNode>]) -> usize {
    fn walk(n: &dyn IRNode) -> usize {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*w.node);
        }
        let child = children(n).into_iter().map(walk).max().unwrap_or(0);
        let is_fin = Bytecode::try_from_u8(n.bytecode())
            .map(|op| {
                matches!(
                    op,
                    Bytecode::FIN2
                        | Bytecode::FIN3
                        | Bytecode::FIN4
                        | Bytecode::FINP3
                        | Bytecode::FINP4
                        | Bytecode::FINPOW3
                )
            })
            .unwrap_or(false);
        if is_fin {
            child + 1
        } else {
            child
        }
    }
    nodes.iter().map(|n| walk(&**n)).max().unwrap_or(0)
}

pub(crate) fn join_slot_widths(
    nodes: &[Box<dyn IRNode>],
    slot: &[SlotTy],
    ew: &[u32],
) -> (Vec<Option<u32>>, Vec<bool>) {
    // internal lattice: 0 = no write seen, 1 = pure bool (canonical byte),
    // 8/16/32/64 = width, 128 = u128 (task 3), 254 = mixed u128/sub-128
    // flows (compile-rejected), 255 = mixed bool-ish (no static width)
    const BOOLISH: u8 = 255;
    const MIXED128: u8 = 254;
    const U128: u8 = 128;
    fn join(a: u8, b: u8) -> u8 {
        match (a, b) {
            (0, x) | (x, 0) => x,
            // task 3: a slot fed both u128 and ANY other width (including
            // bool-ish flows) cannot give every flow its interpreter face —
            // a u64 store into a 128-joined slot would widen its arithmetic,
            // and a bool flow into a 16-byte cell would truncate a u128
            // value's high half. Mark mixed; the plan walk rejects before
            // any lowering.
            (U128, y) | (y, U128) if y != 0 && y != U128 => MIXED128,
            (x, y) if x == BOOLISH || y == BOOLISH => BOOLISH,
            (x, y) if x == MIXED128 || y == MIXED128 => MIXED128,
            (x, y) if x == 1 || y == 1 => {
                if x == 1 && y == 1 {
                    1
                } else {
                    BOOLISH // bool mixed with a real width
                }
            }
            (x, y) => x.max(y),
        }
    }
    fn walk(node: &dyn IRNode, slot: &[SlotTy], w: &mut Vec<u8>, ew: &[u32], dw: &mut DynW) {
        if let Some(wr) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*wr.node, slot, w, ew, dw);
        }
        let grow = |w: &mut Vec<u8>, idx: usize| {
            while w.len() <= idx {
                w.push(0);
            }
        };
        if let Ok(op) = Bytecode::try_from_u8(node.bytecode()) {
            match op {
                Bytecode::PUT => {
                    if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                        let idx = p.para as usize;
                        if matches!(slot.get(idx), Some(SlotTy::Int)) {
                            let e = expr_w(&*p.subx, slot, w, ew);
                            grow(w, idx);
                            dw.record(idx, e);
                            w[idx] = join(w[idx], e);
                        }
                    }
                }
                Bytecode::XOP => {
                    if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                        let (_, idx) = decode_local_operand_mark(p.para);
                        let idx = idx as usize;
                        if matches!(slot.get(idx), Some(SlotTy::Int)) {
                            let e = expr_w(&*p.subx, slot, w, ew);
                            grow(w, idx);
                            dw.record(idx, e);
                            // T5/T7: XOP computes at the slot/rhs COMMON
                            // width and stores the WIDENED result — a
                            // narrower rhs never narrows or mixes the slot
                            // (unlike PUT, which stores the narrow value
                            // itself). Widthless/bool rhs flows keep PUT's
                            // conservative join (a bool rhs on a u128 slot
                            // still lands in the mixed-width reject).
                            if e == BOOLISH || e <= 1 || w[idx] <= 1 || w[idx] == BOOLISH {
                                w[idx] = join(w[idx], e);
                            } else if e > w[idx] {
                                w[idx] = e;
                            }
                        }
                    }
                }
                // T11: constant-index local_x_put(i, v) — the PUT width face
                Bytecode::PUTX => {
                    if let Some(p) = node.as_any().downcast_ref::<IRNodeDouble>() {
                        if let Ok(idx) = local_x_slot_index(&*p.subx, "PUTX") {
                            let idx = idx as usize;
                            if matches!(slot.get(idx), Some(SlotTy::Int)) {
                                let e = expr_w(&*p.suby, slot, w, ew);
                                grow(w, idx);
                                dw.record(idx, e);
                                w[idx] = join(w[idx], e);
                            }
                        }
                    }
                }
                // T8: the high-slot local compound fragment joins the slot
                // width exactly like XOP
                Bytecode::IRLIST
                    if node
                        .as_any()
                        .downcast_ref::<IRNodeArray>()
                        .map(|a| local_compound_assign_parts(&*a).is_some())
                        .unwrap_or(false) =>
                {
                    let (value, idx, _) = local_compound_assign_parts(node).unwrap();
                    let idx = idx as usize;
                    if matches!(slot.get(idx), Some(SlotTy::Int)) {
                        let e = expr_w(value, slot, w, ew);
                        grow(w, idx);
                        dw.record(idx, e);
                        w[idx] = join(w[idx], e);
                    }
                }
                // narrow-PUT face: an `as bytes` VIEW of a slot — the one
                // channel where the interpreter's per-assignment width is
                // directly observable (cast_bytes emits the value's own
                // width; see the DynW doc)
                Bytecode::CBYTES => {
                    if let Some(p) = node.as_any().downcast_ref::<IRNodeSingle>() {
                        let mut inner: &dyn IRNode = &*p.subx;
                        while let Some(wr) = inner.as_any().downcast_ref::<IRNodeWrapOne>() {
                            inner = &*wr.node;
                        }
                        if let Some(idx) = slot_get_index(inner) {
                            if matches!(slot.get(idx as usize), Some(SlotTy::Int)) {
                                dw.viewed(idx as usize);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for c in children(node) {
            walk(c, slot, w, ew, dw);
        }
    }
    fn expr_w(node: &dyn IRNode, slot: &[SlotTy], w: &[u8], ew: &[u32]) -> u8 {
        if let Some(wr) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return expr_w(&*wr.node, slot, w, ew);
        }
        // A `nil` literal contributes NO width to its slot: a slot whose
        // only write is `nil` stays at the "no write seen" code (which
        // resolves to the legacy u64 width), and a nil mixed with a real
        // width keeps that width. (Nil itself never reaches an arithmetic
        // face — its consumers guard 71.)
        if let Ok(Bytecode::PNIL) = Bytecode::try_from_u8(node.bytecode()) {
            return 0;
        }
        let get = |i: usize| -> Option<u32> {
            // B2 task 4: a List slot's width IS its fixed element width
            if matches!(slot.get(i), Some(SlotTy::List)) {
                return match ew.get(i).copied() {
                    Some(x) if x > 0 => Some(x),
                    _ => Some(64),
                };
            }
            match w.get(i) {
                Some(0) | None => None, // unseen this round: contributes nothing
                Some(255) => None,
                Some(x) => Some(*x as u32),
            }
        };
        // node_int_width with the internal-code lookup; the XOP/GET arms
        // call the closure, everything else is structural
        match node_int_width_with(node, slot, &get) {
            Some(v) => v as u8,
            None => BOOLISH,
        }
    }
    let mut w = vec![0u8; slot.len()];
    let mut dw = DynW::new(slot.len());
    for _ in 0..64 {
        let before = w.clone();
        let dw_before = dw.snapshot();
        for n in nodes {
            walk(&**n, slot, &mut w, ew, &mut dw);
        }
        if w == before && dw.snapshot() == dw_before {
            break;
        }
    }
    let slotw = w
        .into_iter()
        .map(|c| match c {
            BOOLISH => None,
            0 => Some(64),
            // mixed u128/sub-128 flows: the sentinel width 0 — the plan walk
            // scans for it and rejects the body before any lowering runs
            MIXED128 => Some(0),
            x => Some(x as u32),
        })
        .collect();
    (slotw, dw.finish())
}

/// Narrow-PUT dynamic-width face detection (the interpreter's per-assignment
/// width vs the kernel's one-static-width cell).
///
/// The interpreter stores an assignment's VALUE with its own width tag
/// (`locals.save` keeps the popped value untouched; `cast_bytes` emits the
/// CURRENT tag's width BE) — so a slot assigned `1001` then `0` reads back
/// through `as bytes` as ONE byte, and the same `as bytes` site can emit
/// different lengths for different inputs when the assignments sit in
/// branches. The kernel cell has one static width (the join), so its `as
/// bytes` view is fixed — the divergence the g17 no-suffix errno bodies hit
/// (interp `oe = [0]` vs kernel `oe = [0, 0]`).
///
/// The plan records, per int slot: (a) every `as bytes` VIEW of the slot,
/// and (b) the min/max static width any write site can store (plain PUT /
/// PUTX store their rhs's own width; XOP-class stores grow by the rhs). A
/// slot with both a view and NOT-all-equal write widths needs the runtime
/// width shadow (`FramePlan::dyn_w`) to reproduce the interpreter's view
/// exactly; single-width slots keep the byte-identical static lowering.
pub(crate) struct DynW {
    min_w: Vec<u8>,
    max_w: Vec<u8>,
    viewed: Vec<bool>,
}

impl DynW {
    fn new(n: usize) -> Self {
        DynW { min_w: vec![0; n], max_w: vec![0; n], viewed: vec![false; n] }
    }
    /// one write-site width contribution (lattice code; real widths only —
    /// bool/nil/u128/mixed codes carry no narrow-PUT face of their own)
    fn record(&mut self, idx: usize, e: u8) {
        if !matches!(e, 8 | 16 | 32 | 64) {
            return;
        }
        if self.min_w[idx] == 0 || e < self.min_w[idx] {
            self.min_w[idx] = e;
        }
        if e > self.max_w[idx] {
            self.max_w[idx] = e;
        }
    }
    fn viewed(&mut self, idx: usize) {
        self.viewed[idx] = true;
    }
    /// the write widths feed off the width lattice (an XOP rhs can read a
    /// slot whose width is still converging), so the flags iterate with it
    fn snapshot(&self) -> (Vec<u8>, Vec<u8>) {
        (self.min_w.clone(), self.max_w.clone())
    }
    fn finish(self) -> Vec<bool> {
        self.min_w
            .iter()
            .zip(&self.max_w)
            .zip(&self.viewed)
            .map(|((lo, hi), v)| *v && *lo != 0 && lo != hi)
            .collect()
    }
}

/// [`node_int_width`] with the slot-width lookup abstracted (the fixpoint
/// walker runs on internal lattice codes instead of `Option<u32>`).
pub(crate) fn node_int_width_with(
    node: &dyn IRNode,
    slot: &[SlotTy],
    slotw: &dyn Fn(usize) -> Option<u32>,
) -> Option<u32> {
    // The generic core mirrors node_int_width's GET/XOP lookups; rather
    // than duplicating the whole match, reuse it through a tiny adapter:
    // both entry points differ ONLY in how they read a slot's width.
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return node_int_width_with(&*w.node, slot, slotw);
    }
    let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return None };
    use Bytecode::*;
    match op {
        GET | GET0 | GET1 | GET2 | GET3 => {
            let idx = match op {
                GET => node
                    .as_any()
                    .downcast_ref::<IRNodeParam1>()?
                    .para as usize,
                GET0 => 0,
                GET1 => 1,
                GET2 => 2,
                _ => 3,
            };
            match slot.get(idx) {
                // the lookup's None covers the bool-ish join (dynamically
                // widthed — consumers treat it as the legacy u64 for
                // arithmetic and reject it for `as bytes`); every caller
                // passes the joined table from [`join_slot_widths`]
                Some(SlotTy::Int) => slotw(idx),
                // T16: a map slot's fixed value width (None for the Bytes
                // species / an always-empty Unset map)
                Some(SlotTy::Map(MapValKind::Uint(w))) => Some(*w),
                _ => None,
            }
        }
        XOP => {
            let n = node.as_any().downcast_ref::<IRNodeParam1Single>()?;
            let (_, idx) = decode_local_operand_mark(n.para);
            let old = match slot.get(idx as usize) {
                Some(SlotTy::Int) => slotw(idx as usize),
                _ => None,
            };
            widen2(old, node_int_width_with(&*n.subx, slot, slotw))
        }
        // T11: constant-index local_x(i) — the slot's width, like GET
        GETX => {
            let n = node.as_any().downcast_ref::<IRNodeSingle>()?;
            let idx = local_x_slot_index(&*n.subx, "GETX").ok()? as usize;
            match slot.get(idx) {
                Some(SlotTy::Int) => slotw(idx),
                _ => None,
            }
        }
        _ => node_int_width_generic(node, slot, slotw),
    }
}

/// The structural (non-slot) half of [`node_int_width`], shared by both
/// entry points.
pub(crate) fn node_int_width_generic(
    node: &dyn IRNode,
    slot: &[SlotTy],
    slotw: &dyn Fn(usize) -> Option<u32>,
) -> Option<u32> {
    let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return None };
    use Bytecode::*;
    match op {
        PTRUE | PFALSE => Some(1),
        EQ | NEQ | LT | GT | LE | GE | NOT | AND | OR | HASKEY => Some(1),
        // T9/T12: XLG and the type-test predicates yield Bool
        XLG | TNIL | TIS | TLIST | TMAP => Some(1),
        // T12: TID yields the type id as a U8 scalar
        TID => Some(8),
        // T1: SQRT/SQRTUP keep the operand's width (sqrt_*_checked casts
        // the result back to the input value's own type)
        SQRT | SQRTUP => {
            let s = node.as_any().downcast_ref::<IRNodeSingle>()?;
            node_int_width_with(&*s.subx, slot, slotw)
        }
        // T13: `x as bool` yields a bool-ish int, exactly the comparison
        // family's width stance; the address cast is bytes-like (None)
        CTO => {
            let p = node.as_any().downcast_ref::<IRNodeParam1Single>()?;
            match p.para {
                b if b == crate::value::ValueTy::Bool as u8 => Some(1),
                _ => None,
            }
        }
        P0 | P1 | P2 | P3 | PU8 => Some(8),
        PU16 => Some(16),
        CU8 => Some(8),
        CU16 => Some(16),
        CU32 => Some(32),
        CU64 => Some(64),
        // task 3: a u128 value is a (lo, hi) u64 pair on the vstack; the
        // width flows through the same oracle so arithmetic/slots/casts
        // dispatch on it
        CU128 => Some(128),
        BYTE => Some(8),
        SIZE => Some(16),
        LENGTH => Some(32),
        // T17: the popped element carries the receiver slot's fixed
        // element width (the arena cells keep one width per slot); the
        // slot-width lookup answers a List slot with its element width.
        TAKEFIRST | TAKELAST => {
            let s = node.as_any().downcast_ref::<IRNodeSingle>()?;
            let idx = slot_get_index(&*s.subx)? as usize;
            match slot.get(idx) {
                Some(SlotTy::List) => slotw(idx),
                _ => None,
            }
        }
        // task 7: a heap uint read's width is its marker's uint type —
        // the interpreter materializes U8..U128, so downstream `as bytes`
        // and the narrow-width overflow faces must see the same width.
        // HREADU's u8 marker reaches uty 0..3 only; HREADUL's u16 marker
        // also encodes u128 (uty 4). uty 5..7 (HREADUL only) map to no
        // uint type — the oracle returns None (the same static-unknown
        // stance as a bool-ish flow) and the lowering arm rejects.
        HREADU => {
            let p = node.as_any().downcast_ref::<IRNodeParam1>()?;
            heap_uint_len((p.para >> 6) as u64).map(|b| (b * 8) as u32)
        }
        HREADUL => {
            let p = node.as_any().downcast_ref::<IRNodeParam2>()?;
            let mark = u16::from_be_bytes(p.para) as u64;
            heap_uint_len(mark >> 13).map(|b| (b * 8) as u32)
        }
        // B2 task 4: an item_get's width is the RECEIVER SLOT's fixed
        // element width (the lookup answers List slots with `ew`; a None
        // answer falls back to the legacy uniform-u64 face exactly like a
        // bool-ish value)
        ITEMGET => {
            let d = node.as_any().downcast_ref::<IRNodeDouble>()?;
            let idx = slot_get_index(&*d.subx)? as usize;
            match slot.get(idx) {
                Some(SlotTy::List) => slotw(idx),
                // T16: a bytes-element list or a Bytes-valued map has no
                // uint width; a uint-valued map answers its fixed width
                Some(SlotTy::Map(MapValKind::Uint(w))) => Some(*w),
                Some(SlotTy::Map(_)) | Some(SlotTy::ListBytes) => None,
                _ => None,
            }
        }
        ADD | SUB | MUL | DIV | MOD | POW | MAX | MIN | ABSDIFF | DIVUP | DIVEXACT | BSHL
        | BSHR | BXOR | BOR | BAND => {
            let d = node.as_any().downcast_ref::<IRNodeDouble>()?;
            widen2(
                node_int_width_with(&*d.subx, slot, slotw),
                node_int_width_with(&*d.suby, slot, slotw),
            )
        }
        CLAMP => {
            let t = node.as_any().downcast_ref::<IRNodeTriple>()?;
            node_int_width_with(&*t.subx, slot, slotw)
        }
        // kehuobu 8 MULDIV: the interpreter widens the three operands
        // (arithmetic_args3) and casts the quotient back to that width.
        MULDIV | MULDIVUP => {
            let t = node.as_any().downcast_ref::<IRNodeTriple>()?;
            widen2(
                widen2(
                    node_int_width_with(&*t.subx, slot, slotw),
                    node_int_width_with(&*t.suby, slot, slotw),
                ),
                node_int_width_with(&*t.subz, slot, slotw),
            )
        }
        // T3 MULADD/MULSUB: same three-way widen (arithmetic_args3); the
        // u256 intermediate is internal — mul_xy_addsub_z_fit_u128 casts the
        // u128-fit result back to this common width.
        MULADD | MULSUB => {
            let t = node.as_any().downcast_ref::<IRNodeTriple>()?;
            widen2(
                widen2(
                    node_int_width_with(&*t.subx, slot, slotw),
                    node_int_width_with(&*t.suby, slot, slotw),
                ),
                node_int_width_with(&*t.subz, slot, slotw),
            )
        }
        // T2 ADDMOD/MULMOD: same three-way widen (arithmetic_args3 zeroes
        // every operand up to the max width; the modulus result is < z, so
        // the cast back to this width always fits).
        ADDMOD | MULMOD => {
            let t = node.as_any().downcast_ref::<IRNodeTriple>()?;
            widen2(
                widen2(
                    node_int_width_with(&*t.subx, slot, slotw),
                    node_int_width_with(&*t.suby, slot, slotw),
                ),
                node_int_width_with(&*t.subz, slot, slotw),
            )
        }
        // T26 FIN2: arithmetic_args2 widens both operands; the kernel result
        // casts back to that common width (cast_uint_result2).
        FIN2 => {
            let n = node.as_any().downcast_ref::<IRNodeParam1Double>()?;
            widen2(
                node_int_width_with(&*n.subx, slot, slotw),
                node_int_width_with(&*n.suby, slot, slotw),
            )
        }
        // T26 FIN3: the three-way widen — EXCEPT u64_mad (id 22): the
        // interpreter's triop_fin special case keeps the ORIGINAL base (x)
        // width as both the promotion sample and the result width
        // (instruction.rs:43-54), no common-width lift.
        FIN3 => {
            let n = node.as_any().downcast_ref::<IRNodeParam1Triple>()?;
            if n.para == 22 {
                node_int_width_with(&*n.subx, slot, slotw)
            } else {
                widen2(
                    widen2(
                        node_int_width_with(&*n.subx, slot, slotw),
                        node_int_width_with(&*n.suby, slot, slotw),
                    ),
                    node_int_width_with(&*n.subz, slot, slotw),
                )
            }
        }
        // T26 FINPOW3: the three-way widen (arithmetic_args3 → rpow's result
        // casts back to the common width).
        FINPOW3 => {
            let n = node.as_any().downcast_ref::<IRNodeParam1Triple>()?;
            widen2(
                widen2(
                    node_int_width_with(&*n.subx, slot, slotw),
                    node_int_width_with(&*n.suby, slot, slotw),
                ),
                node_int_width_with(&*n.subz, slot, slotw),
            )
        }
        // T26 FIN4: the four-way widen (arithmetic_args4).
        FIN4 => {
            let n = node.as_any().downcast_ref::<IRNodeParam1Quad>()?;
            widen2(
                widen2(
                    node_int_width_with(&*n.subx, slot, slotw),
                    node_int_width_with(&*n.suby, slot, slotw),
                ),
                widen2(
                    node_int_width_with(&*n.subz, slot, slotw),
                    node_int_width_with(&*n.subw, slot, slotw),
                ),
            )
        }
        // T26 FINP3/FINP4: the predicates return Value::bool — the same
        // canonical 0/1 byte as the comparison family.
        FINP3 | FINP4 => Some(1),
        CHOOSE | IRIFR => {
            let n = node.as_any().downcast_ref::<IRNodeTriple>()?;
            widen2(
                node_int_width_with(&*n.suby, slot, slotw),
                node_int_width_with(&*n.subz, slot, slotw),
            )
        }
        // item 10: INC/DEC keeps the operand's own width (the interpreter's
        // Value::inc/dec are checked ops on the value's own type — U8 + n
        // stays U8; no widening, probed)
        INC | DEC => {
            let n = node.as_any().downcast_ref::<IRNodeParam1Single>()?;
            node_int_width_with(&*n.subx, slot, slotw)
        }
        IRBLOCK | IRBLOCKR | IRLIST => {
            let arr = node.as_any().downcast_ref::<IRNodeArray>()?;
            match arr.subs.last() {
                Some(last) if last.hasretval() => node_int_width_with(&**last, slot, slotw),
                _ => None,
            }
        }
        _ => None,
    }
}
