use super::*;


/// P3-V2IMPL: the list side of a `FramePlan` — `kind` is index-aligned with
/// slots; `lits` carries the compile-time element values of `Rodata`
/// tables (empty for the other kinds; T19: one full u128 per element, the
/// low `stride` bytes are materialized little-endian at emit time); `ew`
/// is each slot's fixed ELEMENT WIDTH in bits (8/16/32/64/128; 0 = unset —
/// B2 task 4: a list stores one element type, fixed by its creation
/// literals or its first appended / item-set value, and every later store
/// must match).
#[derive(Debug, Clone, Default)]
pub(crate) struct ListPlanData {
    pub(crate) kind: Vec<ListKind>,
    pub(crate) lits: Vec<Vec<u128>>,
    pub(crate) ew: Vec<u32>,
    /// T16: slot is a map-produced Bytes-element list (KEYS / a Bytes-
    /// valued VALUES). Index-aligned with slots; false elsewhere.
    pub(crate) bytes_elem: Vec<bool>,
}

/// Every child of `node` in evaluation order.
pub(crate) fn children(node: &dyn IRNode) -> Vec<&dyn IRNode> {
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
    } else if let Some(n) = any.downcast_ref::<IRNodeParam1Double>() {
        // T26 FIN2: the fin call's argv subtrees are plain expressions —
        // every walk (slot typing, Nil tracking, width joins, the u256
        // scan) must see them like any other op's operands.
        push_box(&mut out, &n.subx);
        push_box(&mut out, &n.suby);
    } else if let Some(n) = any.downcast_ref::<IRNodeParam1Triple>() {
        // T26 FIN3/FINP3/FINPOW3 (same shape family)
        push_box(&mut out, &n.subx);
        push_box(&mut out, &n.suby);
        push_box(&mut out, &n.subz);
    } else if let Some(n) = any.downcast_ref::<IRNodeParam1Quad>() {
        // T26 FIN4/FINP4 (same shape family)
        push_box(&mut out, &n.subx);
        push_box(&mut out, &n.suby);
        push_box(&mut out, &n.subz);
        push_box(&mut out, &n.subw);
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
pub(crate) struct FramePlan {
    pub(crate) slot: Vec<SlotTy>,
    pub(crate) needs_zero: Vec<bool>,
    pub(crate) list: ListPlanData,
    /// P3-PAIRPKG: remove/insert pair marks (Root on the REMOVE statement,
    /// Skip on the INSERT sibling) — produced by the same plan() sequence
    /// walk, consumed by the lowering's statement dispatch.
    pub(crate) pair: PairPlan,
    /// Task-1 width model: per-slot joined uint widths (None = a bool-ish
    /// value can flow in; consumers keep the legacy uniform-u64 behavior).
    pub(crate) slot_w: Vec<Option<u32>>,
    /// narrow-PUT face: slots whose `as bytes` view must follow the LAST
    /// assignment's width (the write sites carry mixed static widths) — the
    /// lowering keeps a 1-byte runtime width shadow in the frame for each
    /// of them (semantics ground truth in the DynW doc on
    /// [`join_slot_widths`]).
    pub(crate) dyn_w: Vec<bool>,
    /// Task 7: the admitted single constant HGROW's segment count (the
    /// frame reserves and zero-fills `seg × Heap::SEGLEN` bytes for the
    /// heap block, plus the grown-flag word). None = no heap_grow in the
    /// body (heap reads/writes then face a length-0 heap: everything but
    /// the empty read traps HeapError(44) at runtime, exactly like the
    /// interpreter before its first grow).
    pub(crate) heap: Option<u8>,
}

/* ------------------------------------------------------------------ */
/* P3-V2IMPL: compo-list v2 shape analysis (plan() = single source of  */
/* truth for admit/reject/lower-data; doc 74 E3/E6, doc 73 E7/E8).     */
/* ------------------------------------------------------------------ */

/// The source shape of a list-creating `PUT`: `[]`/`new_list()` (a bare
/// NEWLIST leaf) or `[e0..eN-1]` (the `[elements…, count, PACKLIST]` IRLIST
/// wrapper built by `context.rs build_list_node`). Returns the element
/// array + count for the PACKLIST form.
pub(crate) enum ListCreate<'a> {
    NewList,
    PackList { arr: &'a IRNodeArray, n: u64 },
}

pub(crate) fn list_create_src(node: &dyn IRNode) -> Option<ListCreate<'_>> {
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

/// T16: the source shape of a map-creating `PUT`: `new_map()` (a bare
/// NEWMAP leaf) or `map{k0: v0, ..}` (the `[k0,v0,.., count, PACKMAP]`
/// IRLIST wrapper built by `expr.rs parse_map_literal`; `count` is the
/// TOTAL item count = 2 × pairs). Returns the item array + count.
pub(crate) enum MapCreate<'a> {
    NewMap,
    PackMap { arr: &'a IRNodeArray, n: u64 },
}

pub(crate) fn map_create_src(node: &dyn IRNode) -> Option<MapCreate<'_>> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return map_create_src(&*w.node);
    }
    let op = Bytecode::try_from_u8(node.bytecode()).ok()?;
    match op {
        Bytecode::NEWMAP => {
            node.as_any().downcast_ref::<IRNodeLeaf>()?;
            Some(MapCreate::NewMap)
        }
        Bytecode::IRLIST => {
            let arr = node.as_any().downcast_ref::<IRNodeArray>()?;
            if arr.subs.len() < 2 {
                return None;
            }
            let last = &*arr.subs[arr.subs.len() - 1];
            let pack = last.as_any().downcast_ref::<IRNodeLeaf>()?;
            if pack.inst != Bytecode::PACKMAP {
                return None;
            }
            let cnt = &*arr.subs[arr.subs.len() - 2];
            let v = crate::lang::ir_literal_value(cnt).ok().flatten()?;
            let n = v.extract_u128().ok()?;
            if n != (arr.subs.len() - 2) as u128 {
                return None;
            }
            if n % 2 != 0 || n > u64::MAX as u128 {
                return None;
            }
            Some(MapCreate::PackMap { arr, n: n as u64 })
        }
        _ => None,
    }
}

/// T16: is `t` a list storage class (uint-element or Bytes-element)?
pub(crate) fn is_list_slot(t: SlotTy) -> bool {
    matches!(t, SlotTy::List | SlotTy::ListBytes)
}

/// T16: is `t` a map descriptor slot?
pub(crate) fn is_map_slot(t: SlotTy) -> bool {
    matches!(t, SlotTy::Map(_))
}

/// T16: the map value species a slot currently carries (Unset = no write
/// reached it yet).
pub(crate) fn map_slot_kind(slot: &[SlotTy], idx: usize) -> MapValKind {
    match slot.get(idx) {
        Some(SlotTy::Map(k)) => *k,
        _ => MapValKind::Unset,
    }
}

/// B2 task 5: the compound item assignment `l[i] += v` — the parser lowers
/// it to `IRLIST[value, receiver, index, IRNodeBytecodes(DUPN 2; ITEMGET;
/// ROLL 3; op; ITEMSET), POP]` (lang::compound, RHS-first). The trailing
/// `POP` discards the container the write leaves, so the form is a
/// statement with no return value (like the local `XOP` path). The rv64
/// backend does NOT splice the raw fragment: it emits the equivalent
/// native sequence against the IR children (one evaluation of each
/// subtree, identical fault faces and order). Returns the value/receiver/
/// index subtrees and the arithmetic opcode when `node` is exactly that
/// shape (any other IRBYTECODE payload — i.e. a fragment rv64 cannot name
/// — stays on the generic path and rejects there).
pub(crate) fn compound_assign_parts(
    node: &dyn IRNode,
) -> Option<(
    &dyn IRNode,
    &dyn IRNode,
    &dyn IRNode,
    Bytecode,
)> {
    let arr = node.as_any().downcast_ref::<IRNodeArray>()?;
    if arr.inst != Bytecode::IRLIST || arr.subs.len() != 5 {
        return None;
    }
    // subs[3] must be the exact compiler-generated item fragment
    let raw = arr.subs[3].as_any().downcast_ref::<IRNodeBytecodes>()?;
    let op = crate::lang::compound::item_compound_op(&raw.codes)?;
    // subs[4] must be the no-retval POP terminator discarding the container
    let tail = arr.subs[4].as_any().downcast_ref::<IRNodeLeaf>()?;
    if tail.inst != Bytecode::POP || tail.hrtv {
        return None;
    }
    Some((&*arr.subs[0], &*arr.subs[1], &*arr.subs[2], op))
}

/// T8: the high-slot local compound assignment `x op= v` (slot ≥ 64, no
/// `XOP` encoding) — the parser lowers it to `IRLIST[value,
/// IRNodeBytecodes(GET idx; SWAP; op; PUT idx)]` (lang::compound,
/// RHS-first). Like the item fragment above, the rv64 backend does NOT
/// splice the raw codes: the statement is consumed natively with exactly
/// the `XOP` semantics (value evaluates first, then the old slot value is
/// read, the arithmetic applied and the result written back — the
/// interpreter's fragment execution `GET; SWAP; op; PUT` is the same
/// read/modify/write). Returns the value subtree, the slot index and the
/// arithmetic opcode when `node` is exactly that shape (any other
/// IRBYTECODE payload stays on the generic path and rejects there).
pub(crate) fn local_compound_assign_parts(node: &dyn IRNode) -> Option<(&dyn IRNode, u8, Bytecode)> {
    let arr = node.as_any().downcast_ref::<IRNodeArray>()?;
    if arr.inst != Bytecode::IRLIST || arr.subs.len() != 2 {
        return None;
    }
    let raw = arr.subs[1].as_any().downcast_ref::<IRNodeBytecodes>()?;
    let (idx, op) = crate::lang::compound::local_compound_parts(&raw.codes)?;
    Some((&*arr.subs[0], idx, op))
}

/// Does this expression's VALUE classify as a list under the shape set?
/// Used by plan()'s escape check (a list value flowing into any consumer
/// outside the five admitted ops is a reject naming that consumer's
/// opcode) and by the PUT arm's alias detection.
pub(crate) fn expr_is_list_valued(node: &dyn IRNode, slot: &[SlotTy]) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return expr_is_list_valued(&*w.node, slot);
    }
    let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return false };
    use Bytecode::*;
    match op {
        NEWLIST | PACKLIST | APPEND | CLEAR => true,
        // B2 task 5: ITEMSET yields the receiver itself (bytecode metadata
        // hasretval=1) — at a block drop position it is the admitted
        // mutation statement form; in a value position the escape check
        // rejects the consumer, exactly like APPEND.
        ITEMSET => true,
        // P3-PAIRPKG: remove/insert yield the receiver (bytecode metadata
        // hasretval=1) — a pair-marked node at a block drop position is
        // the admitted mutation form (the escape-check gate consults the
        // pair marks); unmarked nodes were already plan-rejected.
        REMOVE | INSERT => true,
        // T17: MERGE yields the receiver itself (the source's elements are
        // appended into it); KEYS/VALUES yield a fresh list (and fault 83
        // on a list receiver). All three are list-valued in a value
        // position.
        MERGE | KEYS | VALUES => true,
        // P2: CLONE yields a fresh container of the receiver's species —
        // list-valued on every admitted (list) receiver. The opt-in PUT
        // binding is its only admitted consumer; everything else keeps
        // the escape reject.
        CLONE => true,
        // B2 task 5: the compound item assignment's IRList shape is a
        // statement (its POP tail discards the container) — at a block drop
        // position it is the admitted mutation form, like APPEND/ITEMSET.
        IRLIST | IRBLOCK | IRBLOCKR if compound_assign_parts(node).is_some() => true,
        // T16: KEYS/VALUES yield a fresh compo list (the map snapshot).
        KEYS | VALUES => true,
        GET | GET0 | GET1 | GET2 | GET3 => slot_get_index(node)
            .map(|i| matches!(slot.get(i as usize), Some(SlotTy::List) | Some(SlotTy::ListBytes)))
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

/// T16: does this expression's VALUE classify as a MAP under the shape
/// set? The map analogue of `expr_is_list_valued` — a map value flowing
/// into any consumer outside the admitted map ops is a reject naming that
/// consumer's opcode (PUT binding of a plain alias/create is intercepted
/// by the PUT arm before this check).
pub(crate) fn expr_is_map_valued(node: &dyn IRNode, slot: &[SlotTy]) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return expr_is_map_valued(&*w.node, slot);
    }
    let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else { return false };
    use Bytecode::*;
    let receiver_is_map = |n: &dyn IRNode| {
        slot_get_index(n)
            .map(|i| is_map_slot(slot.get(i as usize).copied().unwrap_or(SlotTy::None)))
            .unwrap_or(false)
    };
    match op {
        NEWMAP | PACKMAP => true,
        // mutation ops yield the receiver map (hasretval=1); the admitted
        // consumer is the statement drop position (plan intercepts the arm)
        INSERT | REMOVE | ITEMSET => node
            .as_any()
            .downcast_ref::<IRNodeTriple>()
            .map(|t| receiver_is_map(&*t.subx))
            .or_else(|| {
                node.as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .map(|d| receiver_is_map(&*d.subx))
            })
            .unwrap_or(false),
        MERGE => node
            .as_any()
            .downcast_ref::<IRNodeDouble>()
            .map(|d| receiver_is_map(&*d.subx))
            .unwrap_or(false),
        GET | GET0 | GET1 | GET2 | GET3 => slot_get_index(node)
            .map(|i| is_map_slot(slot.get(i as usize).copied().unwrap_or(SlotTy::None)))
            .unwrap_or(false),
        CHOOSE | IRIFR => node
            .as_any()
            .downcast_ref::<IRNodeTriple>()
            .map(|t| expr_is_map_valued(&*t.suby, slot) || expr_is_map_valued(&*t.subz, slot))
            .unwrap_or(false),
        IRLIST | IRBLOCK | IRBLOCKR => node
            .as_any()
            .downcast_ref::<IRNodeArray>()
            .filter(|_| node.hasretval())
            .and_then(|a| a.subs.last())
            .map(|s| expr_is_map_valued(&**s, slot))
            .unwrap_or(false),
        _ => false,
    }
}

/// T17: the plain `GET <list slot>` form — the container-value expression
/// every generic list consumer (TAKEFIRST/TAKELAST, MERGE, INSERT, REMOVE,
/// KEYS/VALUES, the list EQ/ordered faces, list casts, XOP old values and
/// the CHOOSE arms) accepts as an operand. Every other list-valued
/// expression form keeps the opcode-named compile reject (B-family: the
/// interpreter accepts more shapes than the kernel lowers).
pub(crate) fn list_slot_get(node: &dyn IRNode, slot: &[SlotTy]) -> Option<usize> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return list_slot_get(&*w.node, slot);
    }
    if !matches!(
        Bytecode::try_from_u8(node.bytecode()).ok(),
        Some(Bytecode::GET)
            | Some(Bytecode::GET0)
            | Some(Bytecode::GET1)
            | Some(Bytecode::GET2)
            | Some(Bytecode::GET3)
    ) {
        return None;
    }
    slot_get_index(node)
        .map(|i| i as usize)
        .filter(|i| matches!(slot.get(*i), Some(SlotTy::List)))
}

/// T17: the container-VALUE source forms an admitted PUT may bind. Every
/// one of them names a slot whose arena object is the value the
/// interpreter's PUT stores (the `Rc` clone):
///
///   * `var m = l`                GET of a list slot        -> Alias
///   * `var m = clone(l)`         CLONE of a list slot      -> Clone
///   * `var m = append(l, v)`     receiver-yielding mutation
///     (APPEND / CLEAR / ITEMSET / INSERT / REMOVE / MERGE)  -> Mutation
///   * `var m = choose(c, a, b)`  both arms list GETs       -> Choose
///   * `var m = keys(l)` / values(l) — always faults 83     -> Trap83
///
/// Anything else that is list-valued (`[l]`, a cast of a list, an ordered
/// compare) is not a storable container value here and keeps the
/// historical reject.
#[derive(Clone, Copy)]
pub(crate) enum ListBindSrc {
    Alias(usize),
    Clone(usize),
    /// The receiver slot of a receiver-yielding mutation (the value is the
    /// receiver's shared object, after the mutation ran).
    Mutation(usize),
    /// Two arm slots; the descriptor is selected at run time.
    Choose(usize, usize),
    /// KEYS/VALUES on a list: the interpreter faults CompoOpNotMatch(83)
    /// before producing the fresh list, so the binding never stores.
    Trap83,
}

pub(crate) fn list_bind_src(node: &dyn IRNode, slot: &[SlotTy]) -> Option<ListBindSrc> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return list_bind_src(&*w.node, slot);
    }
    let op = Bytecode::try_from_u8(node.bytecode()).ok()?;
    use Bytecode::*;
    match op {
        CLONE => {
            let n = node.as_any().downcast_ref::<IRNodeSingle>()?;
            list_slot_get(&*n.subx, slot).map(ListBindSrc::Clone)
        }
        GET | GET0 | GET1 | GET2 | GET3 => list_slot_get(node, slot).map(ListBindSrc::Alias),
        APPEND | REMOVE | MERGE => {
            let n = node.as_any().downcast_ref::<IRNodeDouble>()?;
            list_slot_get(&*n.subx, slot).map(ListBindSrc::Mutation)
        }
        CLEAR => {
            let n = node.as_any().downcast_ref::<IRNodeSingle>()?;
            list_slot_get(&*n.subx, slot).map(ListBindSrc::Mutation)
        }
        INSERT | ITEMSET => {
            let n = node.as_any().downcast_ref::<IRNodeTriple>()?;
            list_slot_get(&*n.subx, slot).map(ListBindSrc::Mutation)
        }
        KEYS | VALUES => {
            let n = node.as_any().downcast_ref::<IRNodeSingle>()?;
            list_slot_get(&*n.subx, slot).map(|_| ListBindSrc::Trap83)
        }
        CHOOSE => {
            let n = node.as_any().downcast_ref::<IRNodeTriple>()?;
            let a = list_slot_get(&*n.suby, slot)?;
            let b = list_slot_get(&*n.subz, slot)?;
            Some(ListBindSrc::Choose(a, b))
        }
        _ => None,
    }
}

/// P3-PAIRPKG: does this expression subtree reference the given slot at
/// all (any GET, directly or as a list-op receiver)? Every reference to a
/// list flows through a GET node, so a plain GET-index descent is the
/// complete predicate. The pair recognizer refuses receiver references in
/// the index and value expressions: the interpreter evaluates them AFTER
/// the remove executed (shifted view — doc 75 E6 k6 probe; `length(l)`
/// mid-pair differs, `l[..]` reads the shifted element).
pub(crate) fn expr_touches_slot(node: &dyn IRNode, s: usize) -> bool {
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
pub(crate) fn expr_has_stmt_form(node: &dyn IRNode) -> bool {
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
pub(crate) fn list_receiver_slot(node: &dyn IRNode, slot: &[SlotTy], op: Bytecode) -> Ret<usize> {
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

/// T16: the receiver of a READ-ONLY list consumer (ITEMGET/LENGTH/HASKEY)
/// — admits both the uint-element class and the map-produced Bytes class.
pub(crate) fn list_any_receiver_slot(node: &dyn IRNode, slot: &[SlotTy], op: Bytecode) -> Ret<usize> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return list_any_receiver_slot(&*w.node, slot, op);
    }
    match slot_get_index(node).filter(|i| {
        slot.get(*i as usize)
            .map(|t| is_list_slot(*t))
            .unwrap_or(false)
    }) {
        Some(i) => Ok(i as usize),
        None => errf!("rv64 lower: opcode {op:?} is outside the translatable kernel subset"),
    }
}

/// T16: the receiver of a MAP consumer — must be a plain GET of a map slot.
pub(crate) fn map_receiver_slot(node: &dyn IRNode, slot: &[SlotTy], op: Bytecode) -> Ret<usize> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return map_receiver_slot(&*w.node, slot, op);
    }
    match slot_get_index(node).filter(|i| {
        slot.get(*i as usize).map(|t| is_map_slot(*t)).unwrap_or(false)
    }) {
        Some(i) => Ok(i as usize),
        None => errf!(
            "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
             (the receiver must be a map variable)"
        ),
    }
}

/// B2 task 5: the receiver slot of a BUFSET — a local BYTES slot. An Input
/// receiver (`data`) is rejected by name: the interpreter's bufset mutates
/// a fresh extract copy, which has no observable effect and no rv64 need.
pub(crate) fn bytes_receiver_slot(node: &dyn IRNode, slot: &[SlotTy], op: Bytecode) -> Ret<usize> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return bytes_receiver_slot(&*w.node, slot, op);
    }
    match slot_get_index(node) {
        Some(i) if matches!(slot.get(i as usize), Some(SlotTy::Bytes)) => Ok(i as usize),
        Some(i) if matches!(slot.get(i as usize), Some(SlotTy::Input)) => errf!(
            "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
             (the receiver is the read-only input parameter — copy it into a local \
             first, e.g. `var t = data`, then {op:?} into that)"
        ),
        _ => errf!(
            "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
             (the receiver must be a bytes variable)"
        ),
    }
}
