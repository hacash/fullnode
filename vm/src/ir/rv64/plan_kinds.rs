use super::*;


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
pub(crate) struct NilOcc {
    /// this node's VALUE may be Nil when produced (GET of a may-Nil slot,
    /// CHOOSE/IRIFR with a may-Nil arm, CAT with a may-Nil empty-fold
    /// side); consumers query their operand nodes
    pub(crate) val: bool,
    /// XOP only: the OLD slot value this XOP reads may be Nil (guard 71)
    pub(crate) xop_old: bool,
}

pub(crate) fn node_addr(node: &dyn IRNode) -> usize {
    node as *const dyn IRNode as *const u8 as usize
}

pub(crate) struct NilPlan {
    /// slot idx → needs an init flag byte in the frame
    pub(crate) tracked: Vec<bool>,
    pub(crate) occ: HashMap<usize, NilOcc>,
    /// some node's VALUE may be Nil and its tag must be materialized even
    /// though no slot needs an init flag (a literal `nil` operand flows into
    /// a guard without ever touching a tracked local).
    pub(crate) tag_bytes: bool,
}

pub(crate) fn nil_plan(nodes: &[Box<dyn IRNode>], slot: &[SlotTy], has_input: bool) -> NilPlan {
    // flow state: slot idx → may currently hold Nil. Untouched slots are
    // Nil until first written; the `data` param slot never is.
    let mut st: Vec<bool> = slot.iter().map(|s| !matches!(s, SlotTy::Input)).collect();
    let n_slots = slot.len();
    let mut plan = NilPlan { tracked: vec![false; n_slots], occ: HashMap::new(), tag_bytes: false };
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
                // T8: the high-slot local compound fragment — the same
                // read/convert/overwrite flow as XOP (the occ entry rides
                // the fragment IRLIST node; the lowering looks it up there)
                IRLIST
                    if node
                        .as_any()
                        .downcast_ref::<IRNodeArray>()
                        .map(|a| local_compound_assign_parts(&*a).is_some())
                        .unwrap_or(false) =>
                {
                    let (value, idx, _) = local_compound_assign_parts(node).unwrap();
                    self.walk(value, st);
                    let idx = idx as usize;
                    if idx >= st.len() {
                        return;
                    }
                    let old_may = st[idx];
                    let e = self.plan.occ.entry(node_addr(node)).or_default();
                    e.xop_old = old_may;
                    self.plan.tracked[idx] |= old_may;
                    st[idx] = false;
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
                // T11: constant-index local_x(i) — the GET read face
                GETX => {
                    let n = node.as_any().downcast_ref::<IRNodeSingle>().unwrap();
                    self.walk(&*n.subx, st); // the index literal (no effect)
                    let Ok(idx) = local_x_slot_index(&*n.subx, "GETX") else {
                        return; // dynamic index: the lowering rejects it
                    };
                    let idx = idx as usize;
                    let may = idx < st.len()
                        && st[idx]
                        && !matches!(self.slot.get(idx), Some(SlotTy::Input));
                    self.plan.occ.entry(node_addr(node)).or_default().val = may;
                    if may {
                        self.plan.tracked[idx] = true;
                    }
                }
                // T11: constant-index local_x_put(i, v) — the PUT store face
                PUTX => {
                    let n = node.as_any().downcast_ref::<IRNodeDouble>().unwrap();
                    self.walk(&*n.subx, st); // the index literal (no effect)
                    self.walk(&*n.suby, st);
                    let Ok(idx) = local_x_slot_index(&*n.subx, "PUTX") else {
                        return; // dynamic index: the lowering rejects it
                    };
                    let idx = idx as usize;
                    if idx >= st.len() || matches!(self.slot.get(idx), Some(SlotTy::Input)) {
                        return; // input-param assignment: lowering rejects it
                    }
                    let src_may = self.expr_may_nil(&*n.suby);
                    if src_may {
                        // the flag store copies the source's tag
                        self.plan.tracked[idx] = true;
                    }
                    st[idx] = src_may;
                }
                // T9: XLG reads the old local value; the slot is unchanged
                XLG => {
                    let n = node.as_any().downcast_ref::<IRNodeParam1Single>().unwrap();
                    self.walk(&*n.subx, st); // the rhs
                    let (_, idx) = decode_local_logic_mark(n.para);
                    let idx = idx as usize;
                    if idx >= st.len() {
                        return;
                    }
                    let old_may = st[idx];
                    let e = self.plan.occ.entry(node_addr(node)).or_default();
                    e.xop_old = old_may;
                    if old_may {
                        // Eq/Ne and the ordered compares read the init flag
                        // (exactly-one-Nil → 90 / any Nil → 90)
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
                PNIL => {
                    // a nil literal: its value is Nil, so any consumer's
                    // may-Nil guard must fire (and a tag byte must exist to
                    // carry that fact to the staged/consumed position).
                    self.plan.occ.entry(node_addr(node)).or_default().val = true;
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
    plan.tag_bytes =
        plan.tracked.iter().any(|&t| t) || plan.occ.values().any(|o| o.val || o.xop_old);
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
pub(crate) enum FuseForm {
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
pub(crate) enum FuseMark {
    Root { form: FuseForm, c0: u64, n: u64 },
    Skip,
}

pub(crate) type FuseMap = HashMap<usize, FuseMark>;

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
pub(crate) struct PairRec {
    /// shared receiver slot (the paired touch forces the Growable class)
    pub(crate) slot: usize,
    /// both index exprs fold to this literal; None = the variable-index
    /// form (the expression is evaluated once at the remove root)
    pub(crate) const_idx: Option<u64>,
}

/// plan()'s pair marks: Root on the REMOVE statement (the fused emission
/// site), Skip on the INSERT statement (fully subsumed — zero code). The
/// lowering consumes them at the two statement-sequence dispatch sites
/// (the function-body loop and the block arm); a statement walk that
/// bypasses the sibling context must never see these opcodes unmarked
/// (unmarked REMOVE/INSERT are plan-rejected).
#[derive(Default, Clone)]
pub(crate) struct PairPlan {
    pub(crate) root: HashMap<usize, PairRec>,
    pub(crate) skip: HashSet<usize>,
}

pub(crate) fn fuse_unwrap(node: &dyn IRNode) -> &dyn IRNode {
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

pub(crate) fn fuse_const(node: &dyn IRNode) -> Option<u64> {
    crate::lang::ir_literal_value(node)
        .ok()
        .flatten()
        .and_then(|v| v.extract_u128().ok())
        .filter(|x| *x <= u64::MAX as u128)
        .map(|x| x as u64)
}

/// A `byte(recv, idx)` leaf: returns (receiver node, index node).
pub(crate) fn fuse_byte_leaf(node: &dyn IRNode) -> Option<(&dyn IRNode, &dyn IRNode)> {
    let b = fuse_unwrap(node);
    if !matches!(Bytecode::try_from_u8(b.bytecode()), Ok(Bytecode::BYTE)) {
        return None;
    }
    let n = b.as_any().downcast_ref::<IRNodeDouble>()?;
    Some((&*n.subx, &*n.suby))
}

/// The receiver must be a GET/GET0-3 of one Bytes|Input slot; returns
/// the slot index.
pub(crate) fn fuse_recv_slot(node: &dyn IRNode, slot: &[SlotTy]) -> Option<usize> {
    let idx = fuse_get_slot(node)?;
    match slot.get(idx)? {
        SlotTy::Input | SlotTy::Bytes => Some(idx),
        _ => None,
    }
}

/// Raw GET slot index (any slot type — used for the chain accumulator).
pub(crate) fn fuse_get_slot(node: &dyn IRNode) -> Option<usize> {
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
pub(crate) fn fuse_struct_key(node: &dyn IRNode) -> String {
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

pub(crate) fn fuse_nil_ok(node: &dyn IRNode, nilp: &NilPlan) -> bool {
    !nilp.occ.get(&node_addr(fuse_unwrap(node))).map(|o| o.val).unwrap_or(false)
}

/// Try to recognize a byte-assembly group rooted at `node` (an
/// expression-position value root). Returns the form + params.
pub(crate) fn fuse_recognize_expr(
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

/// Task-1 A3 fuse gate: a fused LE term shifts a byte leaf by `sh`; the
/// interpreter faults that shift when the amount ≥ the common width
/// max(8, w_literal) — bare PU8 amounts (8/16/…) ALWAYS fault at u8.
/// Only a width-pinned amount (`<< 8u64`) may fuse; anything else falls
/// back to the unfused path, whose narrow-width check faults identically.
pub(crate) fn fuse_shift_width_ok(lit: &dyn IRNode, sh: u64) -> bool {
    if sh == 0 {
        return true;
    }
    match node_int_width(lit, &[], &[], &[]) {
        Some(w) => (w as u64) > sh,
        None => false,
    }
}

/// LE |/^ tree flatten: (recv, idx, shift, shift-literal) per term —
/// BOR/BXOR internal nodes free, terms are bare byte leaves (shift 0) or
/// BSHL of one. Shared by recognition and fused emission (deterministic
/// on the tree).
pub(crate) fn fuse_flatten_le<'a>(
    node: &'a dyn IRNode,
    terms: &mut Vec<(&'a dyn IRNode, &'a dyn IRNode, u64, &'a dyn IRNode)>,
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
                terms.push((recv, idx, sh, &*d.suby));
            } else {
                let (recv, idx) = fuse_byte_leaf(node)?;
                terms.push((recv, idx, 0, node));
            }
            Some(())
        }
    }
}

/// BE mul-add tree flatten: (recv, idx, mult) per term — ADD internal
/// nodes free, terms are `byte·lit` (either operand order) or a bare
/// byte leaf (mult 1).
pub(crate) fn fuse_flatten_be<'a>(
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
pub(crate) fn fuse_recognize_le_expr(
    root: &dyn IRNode,
    slot: &[SlotTy],
    nilp: &NilPlan,
) -> Option<(FuseForm, usize, u64, u64)> {
    let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64, &dyn IRNode)> = Vec::new();
    fuse_flatten_le(root, &mut terms)?;
    fuse_finish_le(terms, slot, nilp, FuseForm::LeExpr)
}

pub(crate) fn fuse_finish_le(
    terms: Vec<(&dyn IRNode, &dyn IRNode, u64, &dyn IRNode)>,
    slot: &[SlotTy],
    nilp: &NilPlan,
    form: FuseForm,
) -> Option<(FuseForm, usize, u64, u64)> {
    let n = terms.len() as u64;
    if !matches!(n, 2 | 4 | 8) {
        return None;
    }
    let (recv0, idx0, _sh0, _) = terms[0];
    let rs = fuse_recv_slot(recv0, slot)?;
    if !fuse_nil_ok(recv0, nilp) {
        return None;
    }
    let c0 = fuse_const(idx0)?;
    for (k, (recv, idx, sh, lit)) in terms.iter().enumerate() {
        let k = k as u64;
        if fuse_recv_slot(*recv, slot) != Some(rs) || !fuse_nil_ok(*recv, nilp) {
            return None;
        }
        let c = fuse_const(*idx)?;
        if c != c0 + k || *sh != 8 * k {
            return None;
        }
        // task-1 A3: a bare-literal shift amount faults in the interpreter
        // (`byte << 8` is u8 checked_shl) — only width-pinned amounts fuse
        if !fuse_shift_width_ok(*lit, *sh) {
            return None;
        }
        // the byte value and its index must both be non-Nil (the leaf's
        // 72 face is absent only for a non-Nil index)
        if !fuse_nil_ok(*idx, nilp) {
            return None;
        }
    }
    Some((form, rs, c0, n))
}

/// BE mul-add tree: flatten ADD terms, require N ∈ {2,4}, per-position
/// multiplier 2^(8·(N-1-k)), consecutive offsets. Const indexes →
/// BeConst; leaf0 = BASE / leaf_k = ADD(BASE,k) → BeVar.
pub(crate) fn fuse_recognize_be(
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
pub(crate) fn fuse_try_chain(
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
        let (leaf, want_shift, shift_lit) =
            if matches!(Bytecode::try_from_u8(tu.bytecode()), Ok(Bytecode::BSHL)) {
                let Some(dd) = tu.as_any().downcast_ref::<IRNodeDouble>() else {
                    break;
                };
                (&*dd.subx, fuse_const(&*dd.suby), Some(&*dd.suby))
            } else {
                (term, None, None)
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
        // task-1 A3: bare-literal shift amounts fault in the interpreter —
        // only width-pinned chain links fuse (else the unfused narrow check
        // faults identically)
        if !fuse_shift_width_ok(shift_lit.unwrap_or(term), 8 * k) {
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
pub(crate) fn fuse_scan_nodes(
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

pub(crate) fn fuse_scan_node(
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
pub(crate) fn fuse_scan(nodes: &[Box<dyn IRNode>], slot: &[SlotTy], nilp: &NilPlan) -> FuseMap {
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
/* ------------------------------------------------------------------ */
/* Task 7: the heap face.                                              */
/*                                                                     */
/* The kernel admits exactly ONE constant `HGROW` per body (doc §backfill items */
/* 7): the frame reserves and zero-fills that constant's segment bytes, */
/* so the heap length is statically reserved and dynamically 0 until    */
/* the grow executes. The reachability rule is per-PATH: an HGROW       */
/* inside a while body can run more than once → compile reject; more    */
/* than one HGROW node anywhere → compile reject (the frame reserves    */
/* ONE constant's bytes, and the length semantics reference that single */
/* grow — two exclusive-path grows would need two reservations and two  */
/* length phases, which the spec's "exactly once" does not describe).       */
/* HGROW under an if arm stays admissible: it runs at most once per     */
/* path, and reads/writes before it trap HeapError(44) at runtime       */
/* (length 0), the interpreter's own face.                             */
/*                                                                     */
/* seg == 0 stays a RUNTIME HeapError(44): the interpreter faults when  */
/* execution reaches the grow, and a compile reject would move that    */
/* face to compile time for no subset gain — the unconditional trap at */
/* the grow's position reproduces the interpreter exactly.             */
/*                                                                     */
/* The segment count is checked against the caller-supplied per-grow   */
/* cap (`SpaceCap::heap_grow_segment` via the lowering parameter        */
/* channel — never a kernel-side literal); the FRAME budget is the      */
/* 2040B S0 window (the same budget that bounds LIST_BLOCK_MAX and the  */
/* divmod workspace — a body whose heap block does not fit rejects at   */
/* compile time in the prologue's frame check).                        */
/*                                                                     */
/* The six read/write opcodes share that one block with the            */
/* interpreter's `checked_right` faces: the current length is          */
/* `grown_flag × reserved_bytes` (0 before the grow), and              */
/* `start + len > length` traps HeapError(44) — `start + len ==` the   */
/* length is IN bounds, and `read(0, 0)` before any grow returns the   */
/* empty bytes exactly like the interpreter's length-0 heap.           */
/* ------------------------------------------------------------------ */

/// Per-byte length of a `read_u`/`read_ul` uint type marker (0..4 =
/// u8/u16/u32/u64/u128); `None` for the markers the u16 encoding admits
/// but no uint type maps to (uty 5..7 — the interpreter faults HeapError
/// "uint type not supported" at runtime; the kernel compile-rejects).
pub(crate) fn heap_uint_len(uty: u64) -> Option<usize> {
    match uty {
        0 => Some(1),
        1 => Some(2),
        2 => Some(4),
        3 => Some(8),
        4 => Some(16),
        _ => None,
    }
}

/// Task 7 reachability analysis: `Some(seg)` = the single admitted
/// constant HGROW (its u8 immediate segment count, already checked
/// against `seg_max`). A body without HGROW lowers heap ops against a
/// length-0 heap (every non-empty access traps 44 at runtime).
pub(crate) fn heap_plan(nodes: &[Box<dyn IRNode>], seg_max: usize) -> Ret<Option<u8>> {
    fn walk(
        node: &dyn IRNode,
        in_loop: bool,
        found: &mut Option<u8>,
        seg_max: usize,
    ) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*w.node, in_loop, found, seg_max);
        }
        let Ok(op) = Bytecode::try_from_u8(node.bytecode()) else {
            return Ok(());
        };
        match op {
            Bytecode::HGROW => {
                if in_loop {
                    return errf!(
                        "rv64 lower: opcode HGROW is outside the translatable kernel subset \
                         (a heap_grow inside a loop can execute more than once; the kernel \
                         admits exactly one constant heap_grow per body)"
                    );
                }
                if found.is_some() {
                    return errf!(
                        "rv64 lower: opcode HGROW is outside the translatable kernel subset \
                         (the kernel admits exactly one heap_grow per body — the frame \
                         reserves that single constant's segment bytes and the length \
                         semantics reference it; exclusive-path pairs reject)"
                    );
                }
                let seg = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1>()
                    .ok_or("rv64 lower: HGROW node shape unexpected")?
                    .para;
                if seg as usize > seg_max {
                    return errf!(
                        "rv64 lower: opcode HGROW is outside the translatable kernel subset \
                         (the segment count {seg} exceeds SpaceCap::heap_grow_segment \
                         {seg_max})"
                    );
                }
                *found = Some(seg);
                Ok(())
            }
            // the while body AND its condition re-execute per trip: an
            // HGROW in either runs more than once per path
            Bytecode::IRWHILE => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    walk(&*n.subx, true, found, seg_max)?;
                    walk(&*n.suby, true, found, seg_max)
                } else {
                    Ok(())
                }
            }
            _ => {
                for c in children(node) {
                    walk(c, in_loop, found, seg_max)?;
                }
                Ok(())
            }
        }
    }
    let mut found = None;
    for n in nodes {
        walk(&**n, false, &mut found, seg_max)?;
    }
    Ok(found)
}

pub(crate) fn node_res_ty(node: &dyn IRNode, slot: &[SlotTy]) -> Ret<Ty> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return node_res_ty(&*w.node, slot);
    }
    let op = Bytecode::try_from_u8(node.bytecode())
        .map_err(|e| format!("rv64 lower: {e}"))?;
    use Bytecode::*;
    let ty = match op {
        P0 | P1 | P2 | P3 | PU8 | PU16 | PTRUE | PFALSE | PNIL => Ty::Int,
        PBUF | PBUFL | PNBUF => Ty::Bytes,
        CU8 | CU16 | CU32 | CU64 | CU128 => Ty::Int,
        CBYTES => Ty::Bytes,
        // T13: the frontend's `as bool` / `as address` and the address
        // literal (`CTO(Address, PBUF 21B)`, ir/mod.rs push_addr). A bool
        // cast yields a bool-ish int; an address cast yields the Address
        // stain over the operand's 21 content bytes. Every other target id
        // is unreachable from the surface (uint/bytes casts lower as
        // CU*/CBYTES) — keep the reject.
        CTO => {
            let p = node
                .as_any()
                .downcast_ref::<IRNodeParam1Single>()
                .ok_or("rv64 lower: CTO node shape unexpected")?;
            match p.para {
                b if b == crate::value::ValueTy::Bool as u8 => Ty::Int,
                b if b == crate::value::ValueTy::Address as u8 => Ty::Address,
                _ => {
                    return errf!(
                        "rv64 lower: opcode CTO is outside the translatable kernel subset \
                         (target type id {})", p.para
                    )
                }
            }
        }
        ADD | SUB | MUL | DIV | MOD | POW | MAX | MIN | CLAMP | ABSDIFF | DIVUP | DIVEXACT
        | BSHL | BSHR | BXOR | BOR | BAND | EQ | NEQ | LT | GT | LE | GE | NOT | AND | OR => {
            Ty::Int
        }
        BYTE | SIZE => Ty::Int,
        MULDIV | MULDIVUP => Ty::Int,
        // T3 mul_add/mul_sub: same cast_uint_result3 face — a plain uint of
        // the operands' common (three-way max) width.
        MULADD | MULSUB => Ty::Int,
        // T2 add_mod/mul_mod: the interpreter casts the modulus result back
        // to the operands' common width (cast_uint_result3) — always a plain
        // uint scalar.
        ADDMOD | MULMOD => Ty::Int,
        // T26 FIN*: every fin kernel yields a scalar — a uint at the common
        // width for FIN2/3/4/FINPOW3 (cast_uint_result*), a Bool for the
        // FINP3/FINP4 predicates (Value::bool).
        FIN2 | FIN3 | FIN4 | FINP3 | FINP4 | FINPOW3 => Ty::Int,
        CAT | CUT | LEFT | RIGHT | LDROP | RDROP => Ty::Bytes,
        // P3-V2IMPL: the admitted v2 list shapes (plan() has already
        // validated them by the time lowering asks). LENGTH/HASKEY yield
        // scalars; ITEMGET's type follows the receiver species (T16: a
        // bytes-element list or a Bytes-valued map yields bytes).
        LENGTH | HASKEY => Ty::Int,
        ITEMGET => {
            let d = node
                .as_any()
                .downcast_ref::<IRNodeDouble>()
                .ok_or("rv64 lower: itemget node shape unexpected")?;
            match slot_get_index(&*d.subx).and_then(|i| slot.get(i as usize).copied()) {
                Some(SlotTy::ListBytes) => Ty::Bytes,
                Some(SlotTy::Map(MapValKind::Bytes)) => Ty::Bytes,
                _ => Ty::Int,
            }
        }
        // B2 task 5: ITEMSET yields the receiver itself (the container
        // stays on the stack); BUFSET yields the receiver bytes; ITEMCAT
        // yields the concatenated bytes (opcode metadata 0,3,1 / 0,3,1 /
        // 0,1,1 — all retval=1). T16: a map receiver yields the map.
        ITEMSET => {
            let t = node
                .as_any()
                .downcast_ref::<IRNodeTriple>()
                .ok_or("rv64 lower: itemset node shape unexpected")?;
            match slot_get_index(&*t.subx).and_then(|i| slot.get(i as usize).copied()) {
                Some(SlotTy::Map(_)) => Ty::Map,
                _ => Ty::List,
            }
        }
        BUFSET | ITEMCAT => Ty::Bytes,
        // task 7: heap reads — HREAD yields bytes (a descriptor over the
        // frame-reserved heap block); HREADU/HREADUL yield uints of the
        // marker's width (the width oracle answers the same mark). HGROW
        // has no return value and stays in the reject arm (a value
        // position is structurally unreachable from the parser).
        HREAD => Ty::Bytes,
        HREADU | HREADUL => Ty::Int,
        // P3-PAIRPKG: remove/insert produce the receiver itself (the
        // interpreter pops only the arguments; the compo stays on the
        // stack) — needed by the block drop accounting for a pair root.
        // T16: a map receiver yields the map.
        REMOVE | INSERT => {
            let recv = node
                .as_any()
                .downcast_ref::<IRNodeDouble>()
                .map(|d| &*d.subx)
                .or_else(|| {
                    node.as_any()
                        .downcast_ref::<IRNodeTriple>()
                        .map(|t| &*t.subx)
                });
            match recv
                .and_then(|r| slot_get_index(r))
                .and_then(|i| slot.get(i as usize).copied())
            {
                Some(SlotTy::Map(_)) => Ty::Map,
                _ => Ty::List,
            }
        }
        MERGE => {
            let d = node
                .as_any()
                .downcast_ref::<IRNodeDouble>()
                .ok_or("rv64 lower: merge node shape unexpected")?;
            match slot_get_index(&*d.subx).and_then(|i| slot.get(i as usize).copied()) {
                Some(SlotTy::Map(_)) => Ty::Map,
                _ => Ty::List,
            }
        }
        NEWLIST | PACKLIST | APPEND | CLEAR => Ty::List,
        // T16: KEYS/VALUES yield a fresh compo list (the map snapshot).
        KEYS | VALUES => Ty::List,
        // T16: map creation yields the map value.
        NEWMAP | PACKMAP => Ty::Map,
        // T17: TAKEFIRST/TAKELAST pop one element of the receiver list and
        // yield it as a scalar (the slot's fixed element width).
        TAKEFIRST | TAKELAST => Ty::Int,
        // P2: CLONE yields the copy — same species as the receiver (the
        // admitted receivers are lists; the value is a container
        // descriptor, consumed by the opt-in PUT binding).
        CLONE => Ty::List,
        // B2 task 5: the compound item assignment's IRLIST shape is a
        // statement — its POP tail discards the container, so it yields no
        // typed value; the block's drop accounting classifies it as Ty::List
        // (pops as nothing), matching the admitted mutation statement form.
        IRLIST if compound_assign_parts(node).is_some() => Ty::List,
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
                // T13 stain fix: an Address-stained slot re-reads as an
                // Address value (the interpreter's runtime stain face)
                Some(SlotTy::Address) => Ty::Address,
                Some(SlotTy::List) | Some(SlotTy::ListBytes) => Ty::List,
                Some(SlotTy::Map(_)) => Ty::Map,
                _ => Ty::Int,
            }
        }
        XOP => Ty::Int,
        // T11: constant-index local_x(i) — the slot's type, like GET
        GETX => {
            let n = node
                .as_any()
                .downcast_ref::<IRNodeSingle>()
                .ok_or("rv64 lower: GETX node shape unexpected")?;
            let idx = local_x_slot_index(&*n.subx, "GETX")?;
            match slot.get(idx as usize) {
                Some(SlotTy::Bytes) | Some(SlotTy::Input) => Ty::Bytes,
                // T13 stain fix: an Address-stained slot re-reads as an
                // Address value (the interpreter's runtime stain face)
                Some(SlotTy::Address) => Ty::Address,
                Some(SlotTy::List) | Some(SlotTy::ListBytes) => Ty::List,
                Some(SlotTy::Map(_)) => Ty::Map,
                _ => Ty::Int,
            }
        }
        // T9: XLG yields the Bool comparison result
        XLG => Ty::Int,
        // T12: the type-test family — Bool predicates and TID's U8 scalar
        TNIL | TIS | TID | TLIST | TMAP => Ty::Int,
        // T1: SQRT/SQRTUP keep the operand's uint type (the result is cast
        // back to the operand's width by instruction.rs sqrt_*_checked)
        SQRT | SQRTUP => Ty::Int,
        // item 10: `increase(n, x)` / `decrease(n, x)` — the result is the
        // incremented stack-top COPY (the interpreter never writes the
        // source slot back; probed), a plain uint scalar.
        INC | DEC => Ty::Int,
        other => {
            return errf!(
                "rv64 lower: opcode {:?} is outside the translatable kernel subset",
                other
            )
        }
    };
    Ok(ty)
}
