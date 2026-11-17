use super::*;


pub(crate) fn plan<'a>(
    nodes: &'a [Box<dyn IRNode>],
    has_input: bool,
    heap_seg_max: usize,
    containers: bool,
) -> Ret<FramePlan> {
    let mut slot: Vec<SlotTy> = Vec::new();
    let mut needs_zero: Vec<bool> = Vec::new();
    let mut seen: Vec<bool> = Vec::new();
    let mut lst = ListSt { containers, ..Default::default() };
    // P3-PAIRPKG: pair marks collected by the statement-sequence walk
    let mut pairs = PairPlan::default();
    // T11/T9: the interpreter's local frame is sized by ALLOC's immediate —
    // a GETX/PUTX/XLG index at or beyond it faults OutOfStack(42) at run
    // time (locals.load/slot_mut). The kernel grows its slot table from
    // REFERENCES instead, so the folded/XLG index is gated against the
    // declared frame (fail-fast with the same face).
    let local_frame = {
        let mut n = 0usize;
        for s in nodes {
            if let Some(a) = s.as_any().downcast_ref::<IRNodeParam1>() {
                if a.inst == Bytecode::ALLOC {
                    n = n.max(a.para as usize);
                }
            }
        }
        n
    };
    fn grow(
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt<'_>,
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
    struct ListSt<'a> {
        kind: Vec<ListKind>,
        lits: Vec<Vec<u128>>,
        appended: Vec<bool>,
        cleared: Vec<bool>,
        paired: Vec<bool>,
        // B2 task 5: ITEMSET reachability — an in-place element store makes
        // the slot's block writable (Rodata/FrameStatic upgrade to Growable
        // exactly like an append/pair touch; the length shadow is untouched).
        itemset: Vec<bool>,
        // B2 task 4: the slot's fixed ELEMENT WIDTH in bits (0 = unset —
        // fixed by the creation literals or the first element store; every
        // later store must fit or the body rejects).
        ew: Vec<u32>,
        // P2 (§3 arena container model): the opt-in flag (the default
        // subset keeps every historical escape reject; only the
        // `lower_body_mode_arena_containers` entry sets this).
        containers: bool,
        // P2: slots promoted to the arena descriptor model — their 8-byte
        // frame cell holds an arena object pointer instead of flat-block
        // state. Set by alias PUTs (`var m = l`), re-binds (`l = <list>`)
        // and CLONE consumers; refined into `ListKind::Arena` after the
        // walk.
        arena: Vec<bool>,
        // P2: (source, target) slot pairs that must share one element
        // width — an alias pair (same object) or a clone-bind pair (the
        // copy's elements). The width replay below expands every recorded
        // element store across the pair's union-find group, so a store via
        // any alias fixes/joins the group width exactly like the
        // interpreter's shared object.
        alias_pairs: Vec<(usize, usize)>,
        // T17: (receiver, source) slot pairs joined by a MERGE — the
        // appended elements keep their own widths in the interpreter, but
        // the destination slot's fixed element stride reads them back, so
        // the two slots must share one element width (checked after the
        // fixed-point replay; a mismatch is the B-family compile reject).
        merge_pairs: Vec<(usize, usize)>,
        // B2 task 4: element stores recorded in program order. The widths
        // are fixed by a DEFERRED fixpoint replay after the walk: a store
        // value's width can depend on the per-slot width join, which in
        // turn depends on the element widths the replay itself fixes (an
        // ITEMGET read feeds the join with its slot's element width).
        width_ops: Vec<WidthOp<'a>>,
        // T16: map-produced Bytes-element list slots (KEYS / a Bytes-valued
        // VALUES) — index-aligned; the ITEMGET reader yields a bytes pair.
        bytes_elem: Vec<bool>,
        // T16: map value species snapshots for VALUES-result list slots,
        // applied AFTER the element-width replay (which would otherwise
        // reset a uint list's `ew` to the default 64).
        map_list_spec: Vec<(usize, MapValKind)>,
        // T16: map alias groups (union-find). An alias PUT (`var n = m`)
        // makes both slots share ONE object, so a write through either
        // fixes/joins the group's value species exactly like the
        // interpreter's shared `Rc<RefCell<Compo>>`; MERGE joins the
        // destination group's species with the source's.
        map_parent: Vec<usize>,
        // T16: map key expressions validated after the width fixpoint
        // (bool-ish/unknown width rejects; Nil/Bool/empty runtime 73 faces
        // are emitted by the lowering).
        map_key_ops: Vec<(Bytecode, &'a dyn IRNode)>,
        // T16: map value write sites recorded for the final species check
        // (the provisional width assumption must equal the fixed width).
        map_val_writes: Vec<(usize, &'a dyn IRNode, Bytecode, MapValKind)>,
    }
    /// B2 task 4: one recorded element-store shape (see `ListSt::width_ops`).
    enum WidthOp<'a> {
        /// a single-element store: APPEND value / ITEMSET value / fused
        /// pair INSERT value — the value's width must EQUAL the slot's
        /// element width (the first store fixes it)
        Store(usize, &'a dyn IRNode),
        /// PACKLIST creation: `n` elements of one width into slot rs; the
        /// fill must fit the LIST_BLOCK_MAX budget
        Pack(usize, usize, &'a dyn IRNode),
        /// compound assignment `l[i] op= v`: the value's width must not
        /// EXCEED the slot's element width (the interpreter's result
        /// element is the widened join, which stays at the slot's width)
        Compound(usize, &'a dyn IRNode),
    }
    impl<'a> ListSt<'a> {
        fn grow_to(&mut self, idx: usize) {
            while self.kind.len() <= idx {
                self.kind.push(ListKind::NotList);
                self.lits.push(Vec::new());
                self.appended.push(false);
                self.cleared.push(false);
                self.paired.push(false);
                self.itemset.push(false);
                self.ew.push(0);
                self.arena.push(false);
                self.bytes_elem.push(false);
                self.map_parent.push(self.map_parent.len());
            }
        }
        /// T16: root of the map alias group of `s` (path compression).
        fn map_root(&mut self, s: usize) -> usize {
            while self.map_parent.len() <= s {
                self.map_parent.push(self.map_parent.len());
            }
            let mut r = s;
            while self.map_parent[r] != r {
                r = self.map_parent[r];
            }
            let mut c = s;
            while self.map_parent[c] != r {
                let n = self.map_parent[c];
                self.map_parent[c] = r;
                c = n;
            }
            r
        }
        /// T16: join `kind` into slot `s`'s alias group, setting every
        /// member slot's `SlotTy::Map` to the joined species. A conflict
        /// (mixed value widths / uint-vs-bytes) rejects — the kernel cell
        /// holds one species per map slot.
        fn map_join_kind(
            &mut self,
            slot: &mut [SlotTy],
            s: usize,
            kind: MapValKind,
            op: Bytecode,
        ) -> Ret<()> {
            let r = self.map_root(s);
            let cur = match slot.get(r) {
                Some(SlotTy::Map(k)) => *k,
                _ => MapValKind::Unset,
            };
            let Some(j) = cur.join(kind) else {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (a map slot holds mixed value species/widths; the arena entry cell \
                     fixes one scalar species per map)"
                );
            };
            for i in 0..slot.len() {
                if self.map_root(i) == r {
                    if let SlotTy::Map(_) = slot[i] {
                        slot[i] = SlotTy::Map(j);
                    }
                }
            }
            Ok(())
        }
        /// T16: union two map slots into one alias group (an alias PUT).
        fn map_union(&mut self, slot: &mut [SlotTy], a: usize, b: usize, op: Bytecode) -> Ret<()> {
            let (ra, rb) = (self.map_root(a), self.map_root(b));
            let ka = match slot.get(ra) {
                Some(SlotTy::Map(k)) => *k,
                _ => MapValKind::Unset,
            };
            let kb = match slot.get(rb) {
                Some(SlotTy::Map(k)) => *k,
                _ => MapValKind::Unset,
            };
            let Some(j) = ka.join(kb) else {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (aliasing maps with different value species/widths; the arena entry \
                     cell fixes one scalar species per map)"
                );
            };
            if ra <= rb {
                self.map_parent[rb] = ra;
            } else {
                self.map_parent[ra] = rb;
            }
            let r = self.map_root(a);
            for i in 0..slot.len() {
                if self.map_root(i) == r {
                    if let SlotTy::Map(_) = slot[i] {
                        slot[i] = SlotTy::Map(j);
                    }
                }
            }
            Ok(())
        }
        /// T16: does the alias group of `s` contain any OTHER slot?
        fn map_group_size_gt1(&mut self, s: usize) -> bool {
            let r = self.map_root(s);
            let n = self.map_parent.len().max(s + 1);
            (0..n).any(|i| i != s && self.map_root(i) == r)
        }
        // B2 task 4: the width-fix logic moved to the nested free fns
        // `fix_ew_into` / `apply_width_op` below — the plan walk only
        // RECORDS the stores (see `width_ops`), the deferred replay
        // fixes the widths and raises the rejects.
    }
    // P3-V2IMPL: PUT whose source is list-valued — creation (NEWLIST /
    // PACKLIST wrapper), alias (GET of another list), re-bind (PUT into an
    // existing list slot) or an append/clear expression result. Routed
    // BEFORE the generic child walk: the PACKLIST marker sub is not a
    // value child.
    #[allow(clippy::too_many_arguments)]
    fn walk_list_put<'b>(
        src: &'b dyn IRNode,
        idx: usize,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt<'b>,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
        local_frame: usize,
    ) -> Ret<()> {
        let _ = has_input;
        grow(slot, seen, needs_zero, lst, idx);
        // P2: under the opt-in container model a re-bind (`l = <list>`) is
        // the interpreter's plain slot store — the slot drops its old
        // descriptor for a fresh object; every OTHER alias keeps observing
        // the old one. Only the arena class can represent that, so the
        // re-bind promotes the slot (flat classes keep the historical
        // single-assignment reject).
        let rebind = matches!(slot[idx], SlotTy::List);
        if rebind && !lst.containers {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (re-binding the list in slot {idx}; lists are single-assignment — \
                 mutate with append/clear instead)"
            );
        }
        if rebind {
            lst.arena[idx] = true;
        }
        if !rebind && !matches!(slot[idx], SlotTy::None) {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (slot {idx} is already {:?}; a list value cannot be stored into it)",
                slot[idx]
            );
        }
        if !rebind && conditional {
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
                // B2 task 4: the creation fixes the element width (all
                // elements share one width — first element fixes, the rest
                // must match) and the fill must fit the block budget —
                // recorded here, fixed by the deferred fixpoint replay
                // below (an element's width can depend on element widths
                // the replay itself fixes). T17: a NESTED container
                // element is admitted as the interpreter's runtime
                // CastBeValueFail(76) face (pack_list's check_scalar
                // rejects every non-scalar element AFTER all elements
                // evaluated) — no element width is fixed by such a pack.
                let mut vals: Vec<u128> = Vec::with_capacity(n as usize);
                let mut all_lit = true;
                let mut nested = false;
                for (i, e) in arr.subs[..n as usize].iter().enumerate() {
                    walk(&**e, slot, seen, needs_zero, lst, pairs, false, has_input, local_frame)?;
                    let ty = node_res_ty(&**e, slot)?;
                    if ty == Ty::List {
                        // the element expression must be a plain list GET
                        // (its own evaluation has no side effects or
                        // faults; the pack traps 76 after the scalar
                        // elements evaluated)
                        if list_slot_get(&**e, slot).is_none() {
                            return errf!(
                                "rv64 lower: opcode PACKLIST is outside the translatable \
                                 kernel subset (element {i} is a list value with effects; \
                                 bind it to a variable and pack the variable)"
                            );
                        }
                        nested = true;
                        all_lit = false;
                        continue;
                    }
                    if ty != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode PACKLIST is outside the translatable \
                             kernel subset (element {i} is not a u64 value)"
                        );
                    }
                    // T19: record the FULL u128 literal (a u128 element's
                    // rodata table materializes all 16 LE bytes)
                    let lit = crate::lang::ir_literal_value(&**e)
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract_u128().ok());
                    match lit {
                        Some(x) => vals.push(x),
                        None => all_lit = false,
                    }
                }
                if !nested {
                    lst.width_ops.push(WidthOp::Pack(idx, n as usize, src));
                }
                lst.grow_to(idx);
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
    /// T16: provisional uint width of an int expression during the walk —
    /// literals/casts answer exactly; a GET of an Int slot uses the legacy
    /// u64 assumption (the final validation below rejects any body whose
    /// real width differs, so a wrong assumption can never lower).
    fn provisional_int_width(node: &dyn IRNode, slot: &[SlotTy]) -> Option<u32> {
        node_int_width_with(node, slot, &|i| match slot.get(i) {
            Some(SlotTy::Int) => Some(64),
            _ => None,
        })
    }

    /// T16: the value species a map write site stores (provisional pass).
    fn map_val_kind_at(node: &dyn IRNode, slot: &[SlotTy], op: Bytecode) -> Ret<MapValKind> {
        match node_res_ty(node, slot)? {
            Ty::Int => match provisional_int_width(node, slot) {
                Some(w) if w >= 8 => Ok(MapValKind::Uint(w)),
                Some(1) => errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (a bool value cannot be stored in a map — the arena entry cell \
                     carries uint/bytes scalars only)"
                ),
                _ => errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (the map value has no static uint width; pin it with a literal \
                     suffix or an explicit cast)"
                ),
            },
            Ty::Bytes => Ok(MapValKind::Bytes),
            Ty::Address => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (an address value is not yet supported as a map value)"
            ),
            other => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (the map value must be a scalar, got {})",
                ty_name(other)
            ),
        }
    }

    /// T16: final (post-width-fixpoint) value species of a write site.
    fn map_val_kind_final(
        node: &dyn IRNode,
        slot: &[SlotTy],
        slotw: &[Option<u32>],
        ew: &[u32],
        op: Bytecode,
    ) -> Ret<MapValKind> {
        match node_res_ty(node, slot)? {
            Ty::Int => match node_int_width(node, slot, slotw, ew) {
                Some(w) if w >= 8 => Ok(MapValKind::Uint(w)),
                _ => errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (the map value's uint width is not statically fixed)"
                ),
            },
            Ty::Bytes => Ok(MapValKind::Bytes),
            Ty::Address => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (an address value is not yet supported as a map value)"
            ),
            other => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (the map value must be a scalar, got {})",
                ty_name(other)
            ),
        }
    }

    /// T16: plan-time key validation (after the width fixpoint). A map key
    /// is a uint (any width) or a Bytes/Address scalar. Bool/Nil/empty
    /// bytes are RUNTIME 73 faces the lowering emits (Bool statically
    /// detected, Nil via the tag guard, empty via the length check); a
    /// container-valued key or an unresolved width rejects here.
    fn map_key_check(
        node: &dyn IRNode,
        slot: &[SlotTy],
        slotw: &[Option<u32>],
        ew: &[u32],
        op: Bytecode,
    ) -> Ret<()> {
        match node_res_ty(node, slot)? {
            Ty::Bytes | Ty::Address => Ok(()),
            Ty::Int => match node_int_width(node, slot, slotw, ew) {
                Some(_) => Ok(()),
                None => errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (the map key's static type is unknown — a bool-ish mixed flow cannot \
                     be normalized; bind it to an explicitly typed local first)"
                ),
            },
            other => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (the map key must be a uint or bytes scalar, got {})",
                ty_name(other)
            ),
        }
    }

    /// T16: `PUT <slot> := <map creation>` — `new_map()` or `map{..}`. The
    /// slot's 8-byte cell becomes the tagged arena map descriptor; a re-bind
    /// replaces it with a fresh object (the historical list single-assignment
    /// reject does not apply — maps never existed in the flat subset). A
    /// re-bind of an ALIASED map rejects: the alias group's shared species
    /// cannot be split without a per-object kind table.
    #[allow(clippy::too_many_arguments)]
    fn walk_map_put<'b>(
        src: &'b dyn IRNode,
        idx: usize,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt<'b>,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
        local_frame: usize,
    ) -> Ret<()> {
        let _ = (has_input, local_frame);
        grow(slot, seen, needs_zero, lst, idx);
        let rebind = is_map_slot(slot[idx]);
        let old_kind = map_slot_kind(slot, idx);
        if rebind && lst.map_group_size_gt1(idx) {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (re-binding map slot {idx} while an alias of it is live; the kernel \
                 models each alias group with one static value species)"
            );
        }
        if !rebind && !matches!(slot[idx], SlotTy::None) {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (slot {idx} is already {}; a map value cannot be stored into it)",
                slot_ty_name(slot[idx])
            );
        }
        if !rebind && conditional {
            return errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (map slot {idx} is declared under a conditional; declare maps \
                 unconditionally)"
            );
        }
        match map_create_src(src) {
            Some(MapCreate::NewMap) => {
                if rebind && conditional && old_kind != MapValKind::Unset {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (a conditional map re-bind changes the slot's value species on one \
                         path only; re-bind unconditionally or keep one species)"
                    );
                }
                slot[idx] = SlotTy::Map(MapValKind::Unset);
                seen[idx] = true;
                needs_zero[idx] = false;
                Ok(())
            }
            Some(MapCreate::PackMap { arr, n }) => {
                // Every element expression evaluates first, in IR order
                // (interpreter stack order). The count/evenness are checked
                // at lowering (84/40 runtime faces); the plan only walks
                // and types the elements.
                for e in arr.subs[..n as usize].iter() {
                    walk(
                        &**e, slot, seen, needs_zero, lst, pairs,
                        false, has_input, local_frame,
                    )?;
                }
                // value species: every v (odd index) joins
                let mut kind = MapValKind::Unset;
                for j in 0..(n / 2) as usize {
                    let v = &arr.subs[j * 2 + 1];
                    let k = map_val_kind_at(&**v, slot, Bytecode::PACKMAP)?;
                    lst.map_val_writes.push((idx, &**v, Bytecode::PACKMAP, k));
                    let Some(j2) = kind.join(k) else {
                        return errf!(
                            "rv64 lower: opcode PACKMAP is outside the translatable kernel \
                             subset (mixed value species/widths in one map — the arena \
                             entry cell fixes one scalar species per map)"
                        );
                    };
                    kind = j2;
                }
                if rebind && conditional && kind != old_kind {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (a conditional map re-bind changes the slot's value species on one \
                         path only; re-bind unconditionally or keep one species)"
                    );
                }
                // Re-bind: fresh object, fresh species (the group is a
                // singleton by the check above). NEWMAP/create: same.
                slot[idx] = SlotTy::Map(kind);
                seen[idx] = true;
                needs_zero[idx] = false;
                Ok(())
            }
            None => errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (a map value cannot be copied into slot {idx})"
            ),
        }
    }
    fn walk<'b>(
        node: &'b dyn IRNode,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt<'b>,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
        local_frame: usize,
    ) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk(&*w.node, slot, seen, needs_zero, lst, pairs, conditional, has_input, local_frame);
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
has_input, local_frame,
                    );
                }
                // T16: map-creating PUTs (new_map / map{..}) — same routing
                // reason: the PACKMAP marker/count subs are not values.
                if !(has_input && pn.para == 0) && map_create_src(&*pn.subx).is_some() {
                    return walk_map_put(
                        &*pn.subx,
                        pn.para as usize,
                        slot,
                        seen,
                        needs_zero,
                        lst,
                        pairs,
                        conditional,
                        has_input,
                        local_frame,
                    );
                }
            }
        }
        // B2 task 5: the compound item assignment `l[i] op= v` — intercepted
        // BEFORE the generic statement-sequence walk (the receiver GET is a
        // list-valued child and would trip the escape check). The subtrees
        // walk in evaluation order (value, index); the receiver is validated
        // against the slot table without walking its GET as a value.
        if op == Bytecode::IRLIST {
            if let Some((value, receiver, index, aop)) = compound_assign_parts(node) {
                let rs = list_receiver_slot(receiver, slot, Bytecode::ITEMSET)?;
                let kty = node_res_ty(index, slot)?;
                if kty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode ITEMSET is outside the translatable kernel \
                         subset (the list index must be a u64 value)"
                    );
                }
                let vty = node_res_ty(value, slot)?;
                if vty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode ITEMSET is outside the translatable kernel \
                         subset (the compound-assignment value must be a u64 value)"
                    );
                }
                walk(value, slot, seen, needs_zero, lst, pairs, inner_cond, has_input, local_frame)?;
                walk(index, slot, seen, needs_zero, lst, pairs, inner_cond, has_input, local_frame)?;
                // B2 task 4: the compound's value width must not exceed the
                // slot's fixed element stride (the interpreter's result
                // element is the widened join of element and value, which
                // stays AT the slot's width when the value fits) — recorded
                // here, fixed by the deferred fixpoint replay below
                lst.grow_to(rs);
                lst.width_ops.push(WidthOp::Compound(rs, value));
                lst.itemset[rs] = true;
                let _ = aop;
                return Ok(());
            }
            // T8: the high-slot local compound assignment `x op= v`
            // (slot ≥ 64) — intercepted BEFORE the generic child walk so
            // the slot's read+write is typed exactly like the XOP arm
            // below (the raw fragment's GET/PUT are opaque to it). The
            // value subtree walks in evaluation order (RHS-first).
            if let Some((value, idx, _aop)) = local_compound_assign_parts(node) {
                let idx = idx as usize;
                grow(slot, seen, needs_zero, lst, idx);
                seen[idx] = true; // reads the old value: must be zeroed
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
                walk(value, slot, seen, needs_zero, lst, pairs, inner_cond, has_input, local_frame)?;
                return Ok(());
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
has_input, local_frame,
                )?;
            } else {
                for c in children(node) {
                    walk(c, slot, seen, needs_zero, lst, pairs, inner_cond, has_input, local_frame)?;
                }
            }
        } else {
            for c in children(node) {
                walk(c, slot, seen, needs_zero, lst, pairs, inner_cond, has_input, local_frame)?;
            }
        }
        match op {
            Bytecode::PUT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: PUT node shape unexpected")?;
                if has_input && n.para == 0 && matches!(slot.get(0), Some(SlotTy::None) | None) {
                    // The prelude `PUT 0 (ROLL0)` param binding (always the
                    // body's first statement): slot 0 IS the calc input; the
                    // ROLL0 source is stack machinery.
                    grow(slot, seen, needs_zero, lst, 0);
                    slot[0] = SlotTy::Input;
                    seen[0] = true;
                    needs_zero[0] = false;
                    return Ok(());
                }
                if has_input && n.para == 0 && matches!(slot.get(0), Some(SlotTy::Input)) {
                    // T15: a user `data = <bytes>` reassignment (the first
                    // PUT 0 after the prelude). The interpreter rebinds slot
                    // 0 to the value; the kernel models slot 0 as a bytes
                    // LOCAL initialized at entry from a copy of the input
                    // (reads before this statement still see the input, and
                    // the caller's buffer is never touched — the interpreter
                    // only ever mutates freshly extracted copies). Only a
                    // bytes-typed source can rebind: an int value would make
                    // later `data` reads dynamically typed, which the static
                    // one-type-per-slot model cannot track.
                    let ty = node_res_ty(&*n.subx, slot)?;
                    if ty != Ty::Bytes {
                        return errf!(
                            "rv64 lower: the input parameter cannot be assigned a non-bytes \
                             value — the call-data argument is read-only; copy it into a \
                             local first (e.g. `var t = data`) and assign to that"
                        );
                    }
                    slot[0] = SlotTy::Bytes;
                    // needs_zero[0] stays false (the entry copy initializes
                    // the slot) and seen[0] stays true (the prelude bound it)
                    return Ok(());
                }
                let ty = node_res_ty(&*n.subx, slot)?;
                let idx = n.para as usize;
                grow(slot, seen, needs_zero, lst, idx);
                // T16: a MAP-valued source binds the descriptor word (alias:
                // both slots share one object; a create form is intercepted
                // before the walk). A map-producing expression (insert/merge
                // result) has no admitted binding — fail fast.
                if ty == Ty::Map {
                    let alias_src = slot_get_index(&*n.subx).filter(|&i| {
                        is_map_slot(slot.get(i as usize).copied().unwrap_or(SlotTy::None))
                    });
                    let Some(ss) = alias_src else {
                        return errf!(
                            "rv64 lower: opcode PUT is outside the translatable kernel subset \
                             (a map value cannot be copied into slot {idx}; bind it from a \
                             map variable or a map literal)"
                        );
                    };
                    if conditional {
                        return errf!(
                            "rv64 lower: opcode PUT is outside the translatable kernel subset \
                             (map slot {idx} is alias-bound under a conditional; the descriptor \
                             could be read before the binding runs — bind maps unconditionally)"
                        );
                    }
                    if is_map_slot(slot[idx]) && lst.map_group_size_gt1(idx) {
                        return errf!(
                            "rv64 lower: opcode PUT is outside the translatable kernel subset \
                             (re-binding map slot {idx} while an alias of it is live)"
                        );
                    }
                    if !matches!(slot[idx], SlotTy::None) && !is_map_slot(slot[idx]) {
                        return errf!(
                            "rv64 lower: opcode PUT is outside the translatable kernel subset \
                             (slot {idx} is already {}; a map value cannot be stored into it)",
                            slot_ty_name(slot[idx])
                        );
                    }
                    slot[idx] = SlotTy::Map(MapValKind::Unset);
                    lst.map_union(slot.as_mut_slice(), ss as usize, idx, Bytecode::PUT)?;
                    seen[idx] = true;
                    needs_zero[idx] = false;
                    return Ok(());
                }
                // T16: `var ks = keys(m)` / `var vs = values(m)` — the
                // result is a fresh arena list snapshot of the map. KEYS is
                // a Bytes-element list; VALUES takes the map slot's current
                // value species (Unset = the map is empty here → uint 64).
                if ty == Ty::List {
                    let src_op = Bytecode::try_from_u8(n.subx.bytecode()).ok();
                    // T16: only a MAP receiver takes the snapshot binding;
                    // KEYS/VALUES on a list receiver falls through to the
                    // T17 list_bind_src Trap83 face (the runtime 83).
                    let receiver_is_map = n
                        .subx
                        .as_any()
                        .downcast_ref::<IRNodeSingle>()
                        .and_then(|sn| slot_get_index(&*sn.subx))
                        .map(|i| {
                            is_map_slot(slot.get(i as usize).copied().unwrap_or(SlotTy::None))
                        })
                        .unwrap_or(false);
                    if receiver_is_map
                        && matches!(src_op, Some(Bytecode::KEYS) | Some(Bytecode::VALUES))
                    {
                        let sn = n
                            .subx
                            .as_any()
                            .downcast_ref::<IRNodeSingle>()
                            .ok_or("rv64 lower: map list node shape unexpected")?;
                        let ms = map_receiver_slot(&*sn.subx, slot, src_op.unwrap())?;
                        if !matches!(slot[idx], SlotTy::None) {
                            return errf!(
                                "rv64 lower: opcode PUT is outside the translatable kernel \
                                 subset (slot {idx} is already {}; a map snapshot list cannot \
                                 be re-bound)",
                                slot_ty_name(slot[idx])
                            );
                        }
                        let kind = map_slot_kind(slot, ms);
                        let bytes = src_op == Some(Bytecode::KEYS)
                            || kind == MapValKind::Bytes;
                        let ew = match kind {
                            MapValKind::Uint(w) if src_op == Some(Bytecode::VALUES) => w,
                            _ => 64,
                        };
                        lst.grow_to(idx);
                        slot[idx] = if bytes {
                            SlotTy::ListBytes
                        } else {
                            SlotTy::List
                        };
                        seen[idx] = true;
                        needs_zero[idx] = false;
                        lst.kind[idx] = ListKind::Arena { init_n: 0 };
                        lst.arena[idx] = true;
                        lst.bytes_elem[idx] = bytes;
                        lst.ew[idx] = ew;
                        lst.map_list_spec.push((idx, kind));
                        return Ok(());
                    }
                }
                // T17 (§3 arena container model): the general container-
                // value binding forms — an ALIAS (`var m = l`: the
                // descriptor word is copied, both slots share one arena
                // object), a CLONE-bind (the deep copy's fresh
                // descriptor), a receiver-yielding mutation result
                // (`var m = append(l, v)` / insert / remove / merge /
                // clear — the receiver's shared object), a CHOOSE between
                // two list slots (descriptor selected at run time) and
                // the always-faulting KEYS/VALUES(list) (83). Every bound
                // slot joins the element-width group and takes the arena
                // class.
                let ctr_src = if ty == Ty::List {
                    list_bind_src(&*n.subx, slot)
                } else {
                    None
                };
                // a container value can only be bound into a FRESH slot
                // or a slot already holding a list — an int/bytes slot
                // cannot change species (the interpreter's dynamic slot
                // values are not representable in the static slot table)
                if ctr_src.is_some() && !matches!(slot[idx], SlotTy::None | SlotTy::List) {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (aliasing a list value into slot {idx}; list values cannot be \
                         copied)"
                    );
                }
                // the container binding writes a descriptor that later
                // reads dereference — a conditional binding would leave
                // the cell unset on the untaken path (the creation gate's
                // precedent, walk_list_put)
                if ctr_src.is_some() && conditional {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (list slot {idx} is bound under a conditional; bind containers \
                         unconditionally)"
                    );
                }
                if matches!(slot[idx], SlotTy::List) && ctr_src.is_none() {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (slot {idx} holds a list; only list-producing expressions may \
                         bind it)"
                    );
                }
                // P3-V2IMPL: a list-valued source into a non-list slot is
                // the alias escape (children were walked first, so a list
                // consumed by a deeper opcode was already named there)
                if ty == Ty::List && ctr_src.is_none() {
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (aliasing a list value into slot {idx}; list values cannot be \
                         copied)"
                    );
                }
                if let Some(bind) = ctr_src {
                    // P2 admit: the slot's 8-byte cell becomes the object
                    // descriptor (the binding statement writes it before
                    // any read — the program order guarantees the source
                    // slot was created first)
                    slot[idx] = SlotTy::List;
                    if !seen[idx] {
                        seen[idx] = true;
                        if !conditional {
                            needs_zero[idx] = false;
                        }
                    }
                    // P2: every SOURCE slot belongs to the same alias group
                    // (the copy reads the shared descriptor) and must take
                    // the descriptor class too — a source left on the flat
                    // class would never write its slot cell and the alias
                    // would copy 0. T17: a Trap83 binding never stores, so
                    // the target keeps a plain Empty class (dead code).
                    let promote = |lst: &mut ListSt<'b>, ss: usize, target: usize| {
                        lst.grow_to(ss.max(target));
                        lst.arena[ss] = true;
                        lst.arena[target] = true;
                        lst.alias_pairs.push((ss, target));
                    };
                    match bind {
                        ListBindSrc::Alias(ss) | ListBindSrc::Clone(ss) => {
                            promote(lst, ss, idx);
                        }
                        ListBindSrc::Mutation(ss) => {
                            promote(lst, ss, idx);
                        }
                        ListBindSrc::Choose(a, b) => {
                            promote(lst, a, idx);
                            promote(lst, b, idx);
                        }
                        ListBindSrc::Trap83 => {
                            lst.grow_to(idx);
                            if matches!(lst.kind[idx], ListKind::NotList) {
                                lst.kind[idx] = ListKind::Empty;
                                lst.lits[idx] = Vec::new();
                            }
                        }
                    }
                    return Ok(());
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
                    // T13 stain fix: an Address-typed source keeps the slot
                    // Address-stained — the interpreter's local holds the
                    // runtime Value (Address), so `is address` / `type_id`
                    // on a re-read answer the Address face (9), not Bytes
                    // (8). The runtime representation is the same 21 content
                    // bytes a bytes slot stores.
                    (SlotTy::None, Ty::Address) => SlotTy::Address,
                    (SlotTy::Address, Ty::Address) => SlotTy::Address,
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
                // T15: after slot 0 transitioned to a bytes local, XOP 0
                // falls through to the type-conflict reject below (the
                // interpreter's arithmetic on the bytes old value faults
                // CastFail 71 — the kernel refuses up front instead)
                if has_input && idx == 0 && matches!(slot.get(0), Some(SlotTy::Input)) {
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
                if matches!(slot[idx], SlotTy::List) {
                    // T17: the old value is a Compo — the interpreter's
                    // local_binop extract faults CastFail(71) after the RHS
                    // evaluated. Only an int RHS can be lowered (its own
                    // faults fire first, exactly like the interpreter).
                    if node_res_ty(&*n.subx, slot)? != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode XOP is outside the translatable kernel \
                             subset (the compound-assignment value must be a u64 value)"
                        );
                    }
                    return Ok(());
                }
                slot[idx] = match slot[idx] {
                    SlotTy::None | SlotTy::Int => SlotTy::Int,
                    SlotTy::List => unreachable!("list handled above"),
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
                    // T15: once slot 0 transitioned to a bytes local (a user
                    // `data = <bytes>` / a BUFSET on `data`), reads must not
                    // stamp the Input marker back over it
                    Bytecode::GET0 => (0, has_input && !matches!(slot.get(0), Some(SlotTy::Bytes))),
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
            // T11: constant-index local_x(i) — the slot bookkeeping of GET
            // (dynamic indices never get here: local_x_slot_index rejects).
            // A read of a List slot is the list-value escape (reject).
            Bytecode::GETX => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: GETX node shape unexpected")?;
                let idx = local_x_slot_index(&*n.subx, "GETX")? as usize;
                if idx >= local_frame {
                    return errf!(
                        "rv64 lower: opcode GETX is outside the translatable kernel subset \
                         (local index {idx} is outside the declared local frame — the \
                         interpreter faults OutOfStack(42))"
                    );
                }
                grow(slot, seen, needs_zero, lst, idx);
                if has_input && idx == 0 {
                    slot[idx] = SlotTy::Input;
                }
                if matches!(slot[idx], SlotTy::List) {
                    return errf!(
                        "rv64 lower: opcode GETX is outside the translatable kernel subset \
                         (a list value cannot be used as a value)"
                    );
                }
                seen[idx] = true;
                Ok(())
            }
            // T11: constant-index local_x_put(i, v) — the slot bookkeeping
            // of PUT (children were walked first; the value's type decides
            // the join exactly like a plain assignment).
            Bytecode::PUTX => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: PUTX node shape unexpected")?;
                let idx = local_x_slot_index(&*n.subx, "PUTX")? as usize;
                if idx >= local_frame {
                    return errf!(
                        "rv64 lower: opcode PUTX is outside the translatable kernel subset \
                         (local index {idx} is outside the declared local frame — the \
                         interpreter faults OutOfStack(42))"
                    );
                }
                if has_input && idx == 0 {
                    // the interpreter's prelude binding is always a plain
                    // PUT; a PUTX into slot 0 is an input-parameter
                    // assignment (eval rejects it by name)
                    grow(slot, seen, needs_zero, lst, 0);
                    slot[0] = SlotTy::Input;
                    seen[0] = true;
                    needs_zero[0] = false;
                    return Ok(());
                }
                let ty = node_res_ty(&*n.suby, slot)?;
                grow(slot, seen, needs_zero, lst, idx);
                if matches!(slot[idx], SlotTy::List) {
                    return errf!(
                        "rv64 lower: opcode PUTX is outside the translatable kernel subset \
                         (slot {idx} holds a list; only list-producing expressions may \
                         bind it)"
                    );
                }
                if ty == Ty::List {
                    return errf!(
                        "rv64 lower: opcode PUTX is outside the translatable kernel subset \
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
                    // T13 stain fix: same Address-stain rule as the PUT arm
                    (SlotTy::None, Ty::Address) => SlotTy::Address,
                    (SlotTy::Address, Ty::Address) => SlotTy::Address,
                    (a, b)
                        if (a == SlotTy::Int && b == Ty::Int)
                            || (a == SlotTy::Bytes && b == Ty::Bytes) =>
                    {
                        a
                    }
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
            // T9: XLG reads local[idx] (mark's low 5 bits) — the same slot
            // bookkeeping as XOP's read side (the slot is NOT written).
            Bytecode::XLG => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: XLG node shape unexpected")?;
                let (_, idx) = decode_local_logic_mark(n.para);
                let idx = idx as usize;
                if idx >= local_frame {
                    return errf!(
                        "rv64 lower: opcode XLG is outside the translatable kernel subset \
                         (local index {idx} is outside the declared local frame — the \
                         interpreter faults OutOfStack(42))"
                    );
                }
                let rty = node_res_ty(&*n.subx, slot)?;
                if rty == Ty::List {
                    return errf!(
                        "rv64 lower: opcode XLG is outside the translatable kernel subset \
                         (a list value cannot be used by XLG)"
                    );
                }
                grow(slot, seen, needs_zero, lst, idx);
                seen[idx] = true; // XLG reads the old value: must be zeroed
                slot[idx] = match slot[idx] {
                    SlotTy::None | SlotTy::Int => SlotTy::Int,
                    other => {
                        // GT-c T8-style: no Debug leak — name the slot type
                        return errf!(
                            "rv64 lower: opcode XLG is outside the translatable kernel \
                             subset (slot {idx} is a {}; XLG needs an int local)",
                            slot_ty_name(other)
                        )
                    }
                };
                Ok(())
            }
            // ---- P3-V2IMPL: the five admitted list consumers ------------
            Bytecode::ITEMGET | Bytecode::HASKEY | Bytecode::APPEND => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: a MAP receiver — HASKEY/ITEMGET are admitted (key
                // normalization + sorted lookup); APPEND is not (maps have no
                // append: use insert).
                if let Some(ms) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    if op == Bytecode::APPEND {
                        return errf!(
                            "rv64 lower: opcode APPEND is outside the translatable kernel \
                             subset (append needs a list variable; use insert(m, k, v) on maps)"
                        );
                    }
                    let _ = ms;
                    match node_res_ty(&*n.suby, slot)? {
                        Ty::Int | Ty::Bytes | Ty::Address => {}
                        other => {
                            return errf!(
                                "rv64 lower: opcode {op:?} is outside the translatable kernel \
                                 subset (the map key must be a uint or bytes scalar, got {})",
                                ty_name(other)
                            )
                        }
                    }
                    lst.grow_to(ms);
                    lst.map_key_ops.push((op, &*n.suby));
                    return Ok(());
                }
                let rs = if op == Bytecode::APPEND {
                    list_receiver_slot(&*n.subx, slot, op)?
                } else {
                    // T16: ITEMGET/HASKEY also admit a map-produced
                    // Bytes-element snapshot list.
                    list_any_receiver_slot(&*n.subx, slot, op)?
                };
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
                    if vty == Ty::List {
                        // T17: a list value is the interpreter's
                        // check_scalar 76 face; the mutation still runs
                        // (the receiver may be flat or arena).
                        if list_slot_get(&*n.suby, slot).is_none() {
                            return errf!(
                                "rv64 lower: opcode APPEND is outside the translatable kernel \
                                 subset (the appended value is a list expression with \
                                 effects; bind it to a variable first)"
                            );
                        }
                        lst.grow_to(rs);
                        lst.appended[rs] = true;
                        return Ok(());
                    }
                    if vty != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode APPEND is outside the translatable kernel \
                             subset (the appended value must be a u64 value)"
                        );
                    }
                    // B2 task 4: the value's width fixes or must match the
                    // slot's element width — recorded here, fixed by the
                    // deferred fixpoint replay below (the value's width can
                    // depend on element widths the replay itself fixes)
                    lst.grow_to(rs);
                    lst.width_ops.push(WidthOp::Store(rs, &*n.suby));
                    lst.appended[rs] = true;
                }
                Ok(())
            }
            // B2 task 5: in-place element store (`l[i] = v` / item_set) —
            // receiver = local list slot, index u64, value u64. The
            // interpreter's value-side faces (83 non-compo / 80 map) are
            // structurally absent on a local list slot; the may-Nil value
            // face (76 check_scalar admits Nil → the element is silently
            // stored) rejects at lowering like APPEND's BUG-029.
            Bytecode::ITEMSET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: map item_set — overwrite an EXISTING key only (a
                // missing key is 85, never an insert); the value species
                // joins the map's.
                if let Some(ms) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    match node_res_ty(&*n.suby, slot)? {
                        Ty::Int | Ty::Bytes | Ty::Address => {}
                        other => {
                            return errf!(
                                "rv64 lower: opcode ITEMSET is outside the translatable kernel \
                                 subset (the map key must be a uint or bytes scalar, got {})",
                                ty_name(other)
                            )
                        }
                    }
                    let kind = map_val_kind_at(&*n.subz, slot, Bytecode::ITEMSET)?;
                    lst.grow_to(ms);
                    lst.map_join_kind(slot.as_mut_slice(), ms, kind, Bytecode::ITEMSET)?;
                    lst.map_key_ops.push((Bytecode::ITEMSET, &*n.suby));
                    lst.map_val_writes.push((ms, &*n.subz, Bytecode::ITEMSET, kind));
                    return Ok(());
                }
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                let kty = node_res_ty(&*n.suby, slot)?;
                if kty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode ITEMSET is outside the translatable kernel \
                         subset (the list index must be a u64 value)"
                    );
                }
                let vty = node_res_ty(&*n.subz, slot)?;
                if vty == Ty::List {
                    // T17: item_set's check_scalar runs FIRST — a list
                    // value is the 76 face before any index check.
                    if list_slot_get(&*n.subz, slot).is_none() {
                        return errf!(
                            "rv64 lower: opcode ITEMSET is outside the translatable kernel \
                             subset (the stored value is a list expression with effects; \
                             bind it to a variable first)"
                        );
                    }
                    lst.grow_to(rs);
                    lst.itemset[rs] = true;
                    return Ok(());
                }
                if vty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode ITEMSET is outside the translatable kernel \
                         subset (the stored value must be a u64 value; the interpreter \
                         would store it and only a later read of the element faults)"
                    );
                }
                // B2 task 4: the value's width fixes or must match the
                // slot's element width — recorded here, fixed by the
                // deferred fixpoint replay below (see the APPEND arm)
                lst.grow_to(rs);
                lst.width_ops.push(WidthOp::Store(rs, &*n.subz));
                lst.itemset[rs] = true;
                Ok(())
            }
            // B2 task 5: `buf_set(b, ost, src)` — the receiver must be a
            // local BYTES slot (rv64 bytes values are (ptr,len) descriptors:
            // a cut/alias/rodata/input reference shares its underlying
            // buffer, so the store first copies the slot's own bytes into
            // the arena — compiler.md premise 24, iron-law level). The
            // interpreter's in-place `Value::Bytes` mutation is value-copy
            // semantics; the copy-then-write reproduces it exactly. An
            // Input receiver is rejected: the interpreter mutates a fresh
            // extract copy (no observable effect), and materializing one
            // here would need a whole-input arena buffer.
            Bytecode::BUFSET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T15: `buf_set(data, ..)` — the receiver is the input
                // parameter. The interpreter patches a freshly extracted
                // stack copy (slot 0 itself is UNCHANGED); modeling slot 0
                // as an entry-copied bytes local reproduces that exactly
                // (emit_bufset copies the slot's bytes again and never
                // writes the slot back, and reads keep seeing the input).
                if has_input
                    && slot_get_index(&*n.subx) == Some(0)
                    && matches!(slot.get(0), Some(SlotTy::Input))
                {
                    grow(slot, seen, needs_zero, lst, 0);
                    slot[0] = SlotTy::Bytes;
                    // needs_zero[0]/seen[0] already set by the prelude
                }
                let rs = bytes_receiver_slot(&*n.subx, slot, op)?;
                let oty = node_res_ty(&*n.suby, slot)?;
                if oty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode BUFSET is outside the translatable kernel \
                         subset (the write start must be a u64 value)"
                    );
                }
                let sty = node_res_ty(&*n.subz, slot)?;
                if sty != Ty::Bytes && sty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode BUFSET is outside the translatable kernel \
                         subset (the written value must be bytes or a uint scalar — \
                         the interpreter extracts its canonical bytes; address values \
                         are not in the kernel subset)"
                    );
                }
                let _ = rs;
                Ok(())
            }
            // B2 task 5: `item_concat(l)` — list → bytes, every element by
            // its `extract_bytes` (same splice as `extract_call_data`'s
            // one-layer list). The element failure faces (BytesHandle 91 on
            // Nil/bytes/handle elements) are structurally absent: the slot's
            // elements are definite uints (may-Nil stores are compile-
            // rejected). The OutOfValueSize 39 face is the live length ×
            // element step against the kernel's output budget (S3).
            Bytecode::ITEMCAT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                // P2: item_concat over an arena-model (alias/clone
                // escaped) list is not lowered in the prework — the
                // element splice reads tagged cells. B-family reject; T17
                // lowers it on the descriptor.
                if lst.arena.get(rs).copied().unwrap_or(false) {
                    return errf!(
                        "rv64 lower: opcode ITEMCAT is outside the translatable kernel \
                         subset (the receiver list escapes into the arena container model; \
                         item_concat on an aliased list is not yet lowered)"
                    );
                }
                let _ = rs;
                Ok(())
            }
            Bytecode::LENGTH | Bytecode::CLEAR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: map length reads the object header; clear(map) is not
                // in the admitted set (use a fresh map).
                if let Some(ms) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    let _ = ms;
                    if op == Bytecode::CLEAR {
                        return errf!(
                            "rv64 lower: opcode CLEAR is outside the translatable kernel \
                             subset (clearing a map is not in the subset; bind a fresh map)"
                        );
                    }
                    return Ok(());
                }
                let rs = list_any_receiver_slot(&*n.subx, slot, op)?;
                if op == Bytecode::CLEAR {
                    if matches!(slot.get(rs), Some(SlotTy::ListBytes)) {
                        return errf!(
                            "rv64 lower: opcode CLEAR is outside the translatable kernel \
                             subset (a map snapshot list cannot be cleared)"
                        );
                    }
                    lst.grow_to(rs);
                    lst.cleared[rs] = true;
                }
                Ok(())
            }
            // P2 (§3 arena container model): CLONE — the interpreter's only
            // deep copy. Opt-in: admitted on a local list receiver; the
            // result must flow into a PUT (the ctr_src binding form above)
            // — every other consumer keeps the escape reject. The receiver
            // promotes to the arena class (the clone reads the shared
            // object through its descriptor).
            Bytecode::CLONE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                if !lst.containers {
                    return errf!(
                        "rv64 lower: opcode CLONE is outside the translatable kernel subset"
                    );
                }
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                lst.grow_to(rs);
                lst.arena[rs] = true;
                Ok(())
            }
            // ---- every other compo opcode: outside the subset, named -----
            // ---- T17: the general list-value consumers -----------------
            // Every one of these takes the container on the vstack top (or
            // as a plain GET of a list slot) and works on the shared arena
            // object: the receiver promotes to the descriptor class.
            Bytecode::TAKEFIRST | Bytecode::TAKELAST => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                let s = list_receiver_slot(&*n.subx, slot, op)?;
                lst.grow_to(s);
                lst.arena[s] = true;
                Ok(())
            }
            // KEYS/VALUES are map ops: a LIST receiver reaches the
            // interpreter's CompoOpNotMatch(83) (map_ref fails) before any
            // list is built, so the lowering emits that exact trap.
            Bytecode::KEYS | Bytecode::VALUES => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: a MAP receiver — the snapshot joins the map's dir.
                if let Some(ms) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    lst.grow_to(ms);
                    return Ok(());
                }
                let _ = list_receiver_slot(&*n.subx, slot, op)?;
                Ok(())
            }
            // T17: the true shifting INSERT. Interpreter face order
            // (compo.rs insert): the value's check_scalar (76) FIRST, then
            // extract_u32 (72), then i > len (81), then the 128 cap (40).
            Bytecode::INSERT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: a MAP receiver plans a map insertion instead.
                if let Some(ms) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    match node_res_ty(&*n.suby, slot)? {
                        Ty::Int | Ty::Bytes | Ty::Address => {}
                        other => {
                            return errf!(
                                "rv64 lower: opcode INSERT is outside the translatable kernel \
                                 subset (the map key must be a uint or bytes scalar, got {})",
                                ty_name(other)
                            )
                        }
                    }
                    let kind = map_val_kind_at(&*n.subz, slot, Bytecode::INSERT)?;
                    lst.grow_to(ms);
                    lst.map_join_kind(slot.as_mut_slice(), ms, kind, Bytecode::INSERT)?;
                    lst.map_key_ops.push((Bytecode::INSERT, &*n.suby));
                    lst.map_val_writes.push((ms, &*n.subz, Bytecode::INSERT, kind));
                    return Ok(());
                }
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                if node_res_ty(&*n.suby, slot)? != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode INSERT is outside the translatable kernel \
                         subset (the insert index must be a u64 value)"
                    );
                }
                let vty = node_res_ty(&*n.subz, slot)?;
                if vty == Ty::List {
                    // a list value is the interpreter's 76 face; only the
                    // plain GET form is lowered (no effects to replay)
                    if list_slot_get(&*n.subz, slot).is_none() {
                        return errf!(
                            "rv64 lower: opcode INSERT is outside the translatable kernel \
                             subset (the inserted value is a list expression with effects; \
                             bind it to a variable first)"
                        );
                    }
                } else if vty != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode INSERT is outside the translatable kernel \
                         subset (the inserted value must be a u64 value)"
                    );
                }
                lst.grow_to(rs);
                lst.arena[rs] = true;
                if vty == Ty::Int {
                    lst.width_ops.push(WidthOp::Store(rs, &*n.subz));
                }
                Ok(())
            }
            // T17: the true shifting REMOVE (extract_u32 72, then i ≥ len
            // 85, then the left shift).
            Bytecode::REMOVE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: a MAP receiver plans a map removal instead.
                if let Some(ms) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    match node_res_ty(&*n.suby, slot)? {
                        Ty::Int | Ty::Bytes | Ty::Address => {}
                        other => {
                            return errf!(
                                "rv64 lower: opcode REMOVE is outside the translatable kernel \
                                 subset (the map key must be a uint or bytes scalar, got {})",
                                ty_name(other)
                            )
                        }
                    }
                    lst.grow_to(ms);
                    lst.map_key_ops.push((Bytecode::REMOVE, &*n.suby));
                    return Ok(());
                }
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                if node_res_ty(&*n.suby, slot)? != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode REMOVE is outside the translatable kernel \
                         subset (the remove index must be a u64 value)"
                    );
                }
                lst.grow_to(rs);
                lst.arena[rs] = true;
                Ok(())
            }
            // T17: MERGE(list, list) — append the source's elements. Same
            // shared object → CompoOpInvalid(80); the 128 cap → 40; a
            // non-scalar source element → 76 (unrepresentable here: the
            // arena cells only ever hold scalars).
            Bytecode::MERGE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                // T16: a MAP receiver plans a map merge instead.
                if let Some(ma) = slot_get_index(&*n.subx)
                    .map(|i| i as usize)
                    .filter(|&i| is_map_slot(slot.get(i).copied().unwrap_or(SlotTy::None)))
                {
                    let mb = map_receiver_slot(&*n.suby, slot, Bytecode::MERGE)?;
                    let kb = map_slot_kind(slot, mb);
                    lst.grow_to(ma.max(mb));
                    lst.map_join_kind(slot.as_mut_slice(), ma, kb, Bytecode::MERGE)?;
                    return Ok(());
                }
                let rs = list_receiver_slot(&*n.subx, slot, op)?;
                let ss = list_receiver_slot(&*n.suby, slot, op)?;
                lst.grow_to(rs.max(ss));
                lst.arena[rs] = true;
                lst.arena[ss] = true;
                lst.merge_pairs.push((rs, ss));
                Ok(())
            }
            // T17: EQ/NEQ over two list values — the interpreter's
            // `value_content_eq` (ptr-equal shortcut, else element-wise
            // numeric equality). Both operands must be plain list GETs.
            Bytecode::EQ | Bytecode::NEQ => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: binary node shape unexpected")?;
                let lx = expr_is_list_valued(&*n.subx, slot);
                let ly = expr_is_list_valued(&*n.suby, slot);
                if lx || ly {
                    let (Some(sx), Some(sy)) = (
                        list_slot_get(&*n.subx, slot),
                        list_slot_get(&*n.suby, slot),
                    ) else {
                        return errf!(
                            "rv64 lower: opcode {op:?} is outside the translatable kernel \
                             subset (a list value cannot be used by {op:?})"
                        );
                    };
                    lst.grow_to(sx.max(sy));
                    lst.arena[sx] = true;
                    lst.arena[sy] = true;
                }
                // T16: a MAP value has no equality form — keep the generic
                // escape reject (this arm preempts the catch-all walk).
                if expr_is_map_valued(&*n.subx, slot) || expr_is_map_valued(&*n.suby, slot) {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel \
                         subset (a map value cannot be used by {op:?})"
                    );
                }
                Ok(())
            }
            // T17: ordered compare on a list value — lgc_ord_cmp is
            // uint-only, so the interpreter faults Arithmetic(90). Both
            // sides are plain GETs (no effects to replay), so the trap is
            // exact.
            Bytecode::LT | Bytecode::GT | Bytecode::LE | Bytecode::GE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: binary node shape unexpected")?;
                if expr_is_list_valued(&*n.subx, slot) || expr_is_list_valued(&*n.suby, slot) {
                    if list_slot_get(&*n.subx, slot).is_none()
                        || list_slot_get(&*n.suby, slot).is_none()
                    {
                        return errf!(
                            "rv64 lower: opcode {op:?} is outside the translatable kernel \
                             subset (a list value cannot be used by {op:?})"
                        );
                    }
                }
                // T16: a MAP value cannot be ordered-compared either.
                if expr_is_map_valued(&*n.subx, slot) || expr_is_map_valued(&*n.suby, slot) {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel \
                         subset (a map value cannot be used by {op:?})"
                    );
                }
                Ok(())
            }
            // T17: casts of a list value — the interpreter's CastFail(71).
            Bytecode::CBYTES
            | Bytecode::CU8
            | Bytecode::CU16
            | Bytecode::CU32
            | Bytecode::CU64
            | Bytecode::CU128 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: cast node shape unexpected")?;
                if expr_is_list_valued(&*n.subx, slot)
                    && list_slot_get(&*n.subx, slot).is_none()
                {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel \
                         subset (a list value cannot be used by {op:?})"
                    );
                }
                // T16: a MAP value has no cast form.
                if expr_is_map_valued(&*n.subx, slot) {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel \
                         subset (a map value cannot be used by {op:?})"
                    );
                }
                Ok(())
            }
            // T17: the CHOOSE value with list arms — the descriptor is
            // selected at run time (the arms are plain GETs, so the eager
            // evaluation order is unobservable).
            Bytecode::CHOOSE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                if expr_is_list_valued(&*n.suby, slot) || expr_is_list_valued(&*n.subz, slot) {
                    if list_slot_get(&*n.suby, slot).is_none()
                        || list_slot_get(&*n.subz, slot).is_none()
                    {
                        return errf!(
                            "rv64 lower: opcode CHOOSE is outside the translatable kernel \
                             subset (a list value cannot be used by CHOOSE)"
                        );
                    }
                    if node_res_ty(&*n.subx, slot)? != Ty::Int {
                        return errf!(
                            "rv64 lower: opcode CHOOSE is outside the translatable kernel \
                             subset (the condition must be a u64 value)"
                        );
                    }
                }
                // T16: a MAP arm cannot be selected by CHOOSE.
                if expr_is_map_valued(&*n.suby, slot) || expr_is_map_valued(&*n.subz, slot) {
                    return errf!(
                        "rv64 lower: opcode CHOOSE is outside the translatable kernel \
                         subset (a map value cannot be used by CHOOSE)"
                    );
                }
                Ok(())
            }
            Bytecode::NEWMAP
            | Bytecode::PACKMAP
            | Bytecode::PACKTUPLE
            | Bytecode::TUPLE2LIST
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
                // children's values — an append/clear/item-set there is the
                // admitted mutation statement form, its list result is not
                // consumed by the block. Every other list-valued child of a
                // block is a pointless drop (fail-fast: B-family, generator-
                // and target-unreachable).
                let is_block = matches!(
                    op,
                    Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR
                );
                if !is_packlist_wrapper {
                    for c in children(node) {
                        if expr_is_list_valued(c, slot) {
                            let dropped_mutation = is_block && {
                                // B2 task 5: the compound item assignment
                                // (IRLIST shape) is the admitted mutation
                                // statement form — its POP tail discards the
                                // container, so the block consumes nothing
                                let compound = compound_assign_parts(c).is_some();
                                // T17: the remaining receiver-yielding
                                // mutations (INSERT/REMOVE/MERGE) and the
                                // container-value producers whose result the
                                // statement simply DROPS (KEYS/VALUES(list)
                                // — always the 83 face — and a list-arm
                                // CHOOSE) are the same admitted drop form.
                                let cop = Bytecode::try_from_u8(c.bytecode())
                                    .map(|o| {
                                        matches!(
                                            o,
                                            Bytecode::APPEND
                                                | Bytecode::CLEAR
                                                | Bytecode::ITEMSET
                                                | Bytecode::INSERT
                                                | Bytecode::REMOVE
                                                | Bytecode::MERGE
                                                | Bytecode::KEYS
                                                | Bytecode::VALUES
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
                                                    Bytecode::APPEND
                                                        | Bytecode::CLEAR
                                                        | Bytecode::ITEMSET
                                                        | Bytecode::INSERT
                                                        | Bytecode::REMOVE
                                                        | Bytecode::MERGE
                                                        | Bytecode::KEYS
                                                        | Bytecode::VALUES
                                                )
                                            })
                                            .unwrap_or(false)
                                    })
                                    .unwrap_or(false);
                                // a list-arm CHOOSE dropped as a statement
                                // (plan's CHOOSE arm admitted the plain-GET
                                // arm form)
                                let choose_drop = Bytecode::try_from_u8(c.bytecode())
                                    .map(|o| o == Bytecode::CHOOSE)
                                    .unwrap_or(false)
                                    && c.as_any()
                                        .downcast_ref::<IRNodeTriple>()
                                        .map(|t| {
                                            (expr_is_list_valued(&*t.suby, slot)
                                                && list_slot_get(&*t.suby, slot).is_some())
                                                && (expr_is_list_valued(&*t.subz, slot)
                                                    && list_slot_get(&*t.subz, slot).is_some())
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
                                compound || cop || wrapped || marked || choose_drop
                            };
                            if dropped_mutation {
                                continue;
                            }
                            return errf!(
                                "rv64 lower: opcode {op:?} is outside the translatable \
                                 kernel subset (a list value cannot be used by {op:?})"
                            );
                        }
                        // T16: a MAP value flowing into a non-map consumer.
                        if expr_is_map_valued(c, slot) {
                            let dropped_map_mutation = is_block
                                && Bytecode::try_from_u8(fuse_unwrap(&*c).bytecode())
                                    .map(|o| {
                                        matches!(
                                            o,
                                            Bytecode::INSERT
                                                | Bytecode::REMOVE
                                                | Bytecode::MERGE
                                                | Bytecode::ITEMSET
                                        )
                                    })
                                    .unwrap_or(false);
                            if dropped_map_mutation {
                                continue;
                            }
                            return errf!(
                                "rv64 lower: opcode {op:?} is outside the translatable \
                                 kernel subset (a map value cannot be used by {op:?})"
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
    fn walk_stmt_seq<'b>(
        subs: &'b [Box<dyn IRNode>],
        keep_tail: bool,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt<'b>,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
        local_frame: usize,
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
has_input, local_frame,
            )? {
                i += consumed;
                continue;
            }
            walk(&*subs[i], slot, seen, needs_zero, lst, pairs, conditional, has_input, local_frame)?;
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
    fn walk_try_pair<'b>(
        subs: &'b [Box<dyn IRNode>],
        i: usize,
        keep_tail: bool,
        slot: &mut Vec<SlotTy>,
        seen: &mut Vec<bool>,
        needs_zero: &mut Vec<bool>,
        lst: &mut ListSt<'b>,
        pairs: &mut PairPlan,
        conditional: bool,
        has_input: bool,
        local_frame: usize,
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
        // P2: the fused pair is a flat-frame-block overwrite form — an
        // arena-model receiver (alias/clone/rebind escaped) refuses the
        // fusion and falls through to the named REMOVE reject (B-family;
        // T17 lowers the true shifting INSERT/REMOVE on the descriptor).
        if lst.arena.get(rs).copied().unwrap_or(false) {
            return Ok(None);
        }
        // index/value types (fall-through keeps the canonical reject text)
        if node_res_ty(&*rn.suby, slot)? != Ty::Int {
            return Ok(None);
        }
        if node_res_ty(&*nn.subz, slot)? != Ty::Int {
            return Ok(None);
        }
        // B2 task 4: the stored value's width fixes or must match the
        // slot's element width — recorded below with the other width ops
        // (a mismatch rejects the whole body in the deferred replay, which
        // subsumes this arm's fall-through: a body whose insert cannot fit
        // the slot's stride is rejected either way)
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
        lst.width_ops.push(WidthOp::Store(rs, &*nn.subz));
        // walk the receiver GET + the index subtree (the remove side owns
        // both — the insert's are the same shapes) and the value subtree
        walk(&*rn.subx, slot, seen, needs_zero, lst, pairs, conditional, has_input, local_frame)?;
        walk(&*rn.suby, slot, seen, needs_zero, lst, pairs, conditional, has_input, local_frame)?;
        walk(&*nn.subz, slot, seen, needs_zero, lst, pairs, conditional, has_input, local_frame)?;
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
has_input, local_frame,
    )?;
    // B2 task 4: replay the recorded element stores to a fixpoint — a
    // store value's width can depend on the per-slot width join, which in
    // turn depends on the element widths this replay fixes (an ITEMGET
    // read feeds the join with its slot's element width). Element widths
    // only ever get FIXED (never reset), so the replay converges in one
    // pass per typed slot; the final strict pass raises the rejects.
    // P2: first expand the recorded stores across the alias groups — an
    // alias pair shares ONE arena object (and a clone-bind copies its
    // elements), so a store via any member must fix/join the width of
    // every member exactly like the interpreter's shared VecDeque.
    let width_ops = {
        let nslots = slot.len();
        let mut grp = vec![usize::MAX; nslots];
        for &(a, b) in &lst.alias_pairs {
            let (mut ra, mut rb) = (a.min(nslots.saturating_sub(1)), b.min(nslots.saturating_sub(1)));
            while grp[ra] != usize::MAX && grp[ra] != ra {
                ra = grp[ra];
            }
            while grp[rb] != usize::MAX && grp[rb] != rb {
                rb = grp[rb];
            }
            let (root, child) = if ra <= rb { (ra, rb) } else { (rb, ra) };
            grp[child] = root;
        }
        let members = |x: usize| -> Vec<usize> {
            let find = |mut r: usize| -> usize {
                while grp[r] != usize::MAX && grp[r] != r {
                    r = grp[r];
                }
                r
            };
            let root_x = find(x);
            (0..nslots).filter(|&i| find(i) == root_x).collect()
        };
        let mut expanded: Vec<WidthOp> = Vec::new();
        for op in &lst.width_ops {
            let rs = match op {
                WidthOp::Store(rs, _) | WidthOp::Pack(rs, _, _) | WidthOp::Compound(rs, _) => *rs,
            };
            for m in members(rs) {
                expanded.push(match op {
                    WidthOp::Store(_, v) => WidthOp::Store(m, *v),
                    WidthOp::Pack(_, n, src) => WidthOp::Pack(m, *n, *src),
                    WidthOp::Compound(_, v) => WidthOp::Compound(m, *v),
                });
            }
        }
        expanded
    };
    let mut ew_fix: Vec<u32> = vec![0; slot.len()];
    for _ in 0..=slot.len() {
        let (slotw, _) = join_slot_widths(nodes, &slot, &ew_fix);
        let mut next = vec![0u32; slot.len()];
        for op in &width_ops {
            let _ = apply_width_op(op, &slot, &slotw, &ew_fix, &mut next, &lst.arena);
        }
        if next == ew_fix {
            break;
        }
        ew_fix = next;
    }
    {
        let (slotw, _) = join_slot_widths(nodes, &slot, &ew_fix);
        let mut next = vec![0u32; slot.len()];
        for op in &width_ops {
            apply_width_op(op, &slot, &slotw, &ew_fix, &mut next, &lst.arena)?;
        }
        lst.ew = next;
    }
    // T17: a MERGE copies the source's tagged cells into the receiver's
    // object without re-tagging, and every later read of them uses the
    // RECEIVER's fixed element width. The interpreter keeps each
    // element's own width, so two STORED element widths must agree
    // (B-family compile reject on a mismatch; the interpreter accepts it).
    // An UNTYPED receiver (0 — it never stored an element) ADOPTS the
    // source's width: after the merge its cells are the source's tagged
    // elements, and every later read uses the receiver's width. An
    // untyped SOURCE contributes no elements, so it imposes nothing
    // (both empty: the default width is fine).
    for i in 0..lst.merge_pairs.len() {
        let (a, b) = lst.merge_pairs[i];
        let wa = lst.ew.get(a).copied().unwrap_or(0);
        let wb = lst.ew.get(b).copied().unwrap_or(0);
        if wb == 0 {
            continue;
        }
        if wa == 0 {
            while lst.ew.len() <= a {
                lst.ew.push(0);
            }
            lst.ew[a] = wb;
            continue;
        }
        if wa != wb {
            return errf!(
                "rv64 lower: opcode MERGE is outside the translatable kernel subset \
                 (the receiver's element width is {wa} bits and the source's is {wb}; \
                 the arena object stores one element width per slot — merge lists of \
                 the same element width)"
            );
        }
    }
    // P3-V2IMPL: refine the list storage classes — any APPEND/CLEAR
    // reachability upgrades the slot to the growable form (frame block
    // 1024B + len shadow, decision D2). P3-PAIRPKG: a pair touch does the
    // same (the fused form stores into the element block; the length
    // shadow itself is net-zero — see the M6 lemmas). B2 task 5: an
    // in-place ITEMSET also makes the block writable (an all-literal
    // Rodata table lives in the read-only pool — the store must land in
    // the frame block).
    if !slot.is_empty() {
        lst.grow_to(slot.len() - 1);
    }
    for idx in 0..slot.len() {
        if !matches!(slot[idx], SlotTy::List) {
            continue;
        }
        let mutated =
            lst.appended[idx] || lst.cleared[idx] || lst.paired[idx] || lst.itemset[idx];
        lst.kind[idx] = match lst.kind[idx] {
            ListKind::Empty if mutated => ListKind::Growable { init_n: 0 },
            ListKind::Rodata { n } if mutated => ListKind::Growable { init_n: n },
            ListKind::FrameStatic { n } if mutated => ListKind::Growable { init_n: n },
            k => k,
        };
        // P2: an arena-promoted slot (alias/rebind/clone escape) takes the
        // descriptor class — the flat-block budget/stride machinery does
        // not apply to it (elements are tagged arena cells; the capacity
        // is the interpreter's 128-element compo_length, not the 1024B
        // frame block).
        if lst.arena.get(idx).copied().unwrap_or(false)
            && !matches!(lst.kind[idx], ListKind::Arena { .. })
        {
            let init_n = match lst.kind[idx] {
                ListKind::Rodata { n }
                | ListKind::FrameStatic { n }
                | ListKind::Growable { init_n: n } => n,
                _ => 0,
            };
            lst.kind[idx] = ListKind::Arena { init_n };
        }
        // B2 task 4: a never-typed list (an Empty class — every element
        // access faults 85 before any width face) defaults to the uniform
        // u64 width
        if lst.ew[idx] == 0 {
            lst.ew[idx] = 64;
        }
    }
    /// The width a stored element must have: the slot's fixed width, or
    /// the store's own width when the slot is not yet typed. Rejects bool
    /// (`Some(1)`) and widthless (`None`) element values — the interpreter
    /// would store them, but a fixed-step frame block cannot (may-Nil
    /// BUG-029 precedent for Nil). T19: u128 (16B stride) is admitted —
    /// the flat frame block stores the (lo, hi) pair like every other
    /// u128 value; the element-count budget shrinks to
    /// LIST_BLOCK_MAX / 16 = 64.
    fn fix_ew_into(ew: &mut Vec<u32>, idx: usize, w: Option<u32>) -> Ret<()> {
        let Some(w) = w.filter(|&w| w >= 8) else {
            return errf!(
                "rv64 lower: list element store is outside the translatable kernel \
                 subset (the element value has no static uint width — the slot's \
                 fixed element stride cannot represent a bool or bool-ish value)"
            );
        };
        match ew.get(idx).copied().unwrap_or(0) {
            0 => {
                while ew.len() <= idx {
                    ew.push(0);
                }
                ew[idx] = w;
                Ok(())
            }
            same if same == w => Ok(()),
            other => errf!(
                "rv64 lower: list element store is outside the translatable kernel \
                 subset (the slot's element width is fixed at {other} bits; a \
                 {w}-bit store would mix element widths — the interpreter keeps \
                 per-element widths, the kernel block does not)"
            ),
        }
    }

    /// B2 task 4: apply one recorded element store against the given width
    /// tables, mutating `ew_out` (the candidate fix of the replay round).
    /// Widths are looked up with `ew_in` (the previous round's fix) so the
    /// whole round reads one consistent snapshot; first-touch stores fix
    /// the slot's width, later stores must fit it.
    fn apply_width_op(
        op: &WidthOp<'_>,
        slot: &[SlotTy],
        slotw: &[Option<u32>],
        ew_in: &[u32],
        ew_out: &mut Vec<u32>,
        arena: &[bool],
    ) -> Ret<()> {
        match op {
            WidthOp::Store(rs, v) => {
                let w = node_int_width(*v, slot, slotw, ew_in);
                fix_ew_into(ew_out, *rs, w)
            }
            WidthOp::Pack(rs, n, src) => {
                let n = *n as u64;
                let Some(ListCreate::PackList { arr, n: pn }) = list_create_src(*src) else {
                    return errf!(
                        "rv64 lower: opcode PACKLIST is outside the translatable kernel \
                         subset (internal: the width replay lost the pack shape)"
                    );
                };
                debug_assert_eq!(pn, n);
                for e in arr.subs[..pn as usize].iter() {
                    let w = node_int_width(&**e, slot, slotw, ew_in);
                    fix_ew_into(ew_out, *rs, w)?;
                }
                // P2: an arena-model slot has no frame block — its capacity
                // is the interpreter's 128-element compo_length (the pack
                // fill's runtime 40 guard mirrors take_items_from_ops), so
                // the byte budget does not apply; a count over CTR_LIST_CAP
                // is a cap face left to the runtime guard.
                if arena.get(*rs).copied().unwrap_or(false) {
                    return Ok(());
                }
                // the fill must fit the fixed block budget (B2 task 4: the
                // element-count capacity is LIST_BLOCK_MAX / element-step)
                let step = match ew_out.get(*rs).copied() {
                    Some(w) if w >= 8 => (w / 8) as u64,
                    _ => 8,
                };
                if pn * step > LIST_BLOCK_MAX {
                    return errf!(
                        "rv64 lower: opcode PACKLIST is outside the translatable \
                         kernel subset ({pn} × {step}B elements exceed the \
                         {LIST_BLOCK_MAX}B list block budget)"
                    );
                }
                Ok(())
            }
            WidthOp::Compound(rs, v) => {
                let w = node_int_width(*v, slot, slotw, ew_in).filter(|&x| x >= 8);
                let Some(w) = w else {
                    return errf!(
                        "rv64 lower: list element store is outside the translatable \
                         kernel subset (the compound-assignment value has no static \
                         uint width — the slot's fixed element stride cannot \
                         represent a bool or bool-ish value)"
                    );
                };
                match ew_out.get(*rs).copied().unwrap_or(0) {
                    0 => {
                        // first touch: the slot has never stored an element,
                        // so every element read faults 85 before any width
                        // face — take the value's own width (T19: u128
                        // included, 16B stride)
                        while ew_out.len() <= *rs {
                            ew_out.push(0);
                        }
                        ew_out[*rs] = w;
                        Ok(())
                    }
                    same if w <= same => Ok(()),
                    other => errf!(
                        "rv64 lower: list element store is outside the translatable \
                         kernel subset (the compound result {} bits exceeds the \
                         slot's fixed {other}-bit element stride — the interpreter \
                         stores the widened element, the kernel block does not)",
                        w.max(other)
                    ),
                }
            }
        }
    }


    // Task-1 width model: the fixpoint slot-width join over every static
    // write (after the slot types are final). B2 task 4: List slots answer
    // with their fixed element width. The second table flags the narrow-PUT
    // dynamic-width face: an `as bytes` view of a slot whose write sites
    // carry mixed static widths — the interpreter's view follows the LAST
    // assignment's width, the one-width cell cannot, so the lowering plays
    // the width back from a runtime shadow byte (FramePlan::dyn_w).
    let (slot_w, put_view_mixed) = join_slot_widths(nodes, &slot, &lst.ew);
    // T16: the deferred map checks. (a) every key expression must be a uint
    // of statically known width or a bytes/address scalar (Bool keys are
    // admitted here — the lowering traps 73 exactly where the interpreter
    // does; Nil/empty-bytes 73 faces are runtime guards). (b) every map
    // VALUE write's final species must equal the provisional species the
    // walk used to type reads (otherwise a read could have been typed
    // wrongly — reject, B-family).
    for &(op, key) in &lst.map_key_ops {
        map_key_check(key, &slot, &slot_w, &lst.ew, op)?;
    }
    for &(ms, vnode, op, prov) in &lst.map_val_writes {
        let fin = map_val_kind_final(vnode, &slot, &slot_w, &lst.ew, op)?;
        if fin != prov {
            return errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (the map value's uint width is fixed after the read sites were typed; \
                 pin the value with an explicit width cast so every flow agrees)"
            );
        }
        let _ = ms;
    }
    // T16: apply the VALUES-result element species (the width replay above
    // resets every list slot's `ew`; a map snapshot list is not part of the
    // replay's width ops).
    for &(sidx, kind) in &lst.map_list_spec {
        if sidx >= lst.ew.len() {
            continue;
        }
        lst.ew[sidx] = if lst.bytes_elem[sidx] {
            64
        } else {
            match kind {
                MapValKind::Uint(w) => w,
                _ => 64,
            }
        };
    }
    // task 3: a slot fed both u128 and narrower static widths cannot give
    // every flow its interpreter overflow face — the narrower flows would
    // compute (and overflow-check) at 128 bits once the join widens the
    // cell. Compile-reject (the interpreter accepts these bodies; a compile
    // rejection is the iron-rule-legal stance).
    if slot.iter().enumerate().any(|(i, s)| {
        matches!(s, SlotTy::Int) && slot_w.get(i).copied().flatten() == Some(0)
    }) {
        return errf!(
            "rv64 lower: opcode PUT is outside the translatable kernel subset (a local \
             slot mixes u128 and narrower values; the kernel subset needs one static \
             width per slot — pin the narrower flows with `as u128` or keep the u128 \
             flow in its own slot)"
        );
    }
    // narrow-PUT face: flag (not reject) the slots that need the runtime
    // width shadow — an `as bytes` view of a mixed-width-write slot. The
    // interpreter's view width follows the LAST assignment; the shadow
    // replays exactly that (the u128/bool mixings above keep their own
    // rejects; a sub-128 mixed-width slot stays compilable — hacd200's
    // `if total < 1 { total = 1 }` + `total as bytes` shape must compile).
    let dyn_w: Vec<bool> = slot
        .iter()
        .enumerate()
        .map(|(i, s)| {
            matches!(s, SlotTy::Int)
                && matches!(slot_w.get(i).copied().flatten(), Some(w) if (8..128).contains(&w))
                && put_view_mixed.get(i).copied().unwrap_or(false)
        })
        .collect();
    let list = ListPlanData { kind: lst.kind, lits: lst.lits, ew: lst.ew, bytes_elem: lst.bytes_elem };
    let pair = pairs;
    // task 7: the heap face — the reachability analysis over the whole IR
    // (exactly one constant HGROW, never inside a loop; the segment count
    // is checked against the caller-supplied SpaceCap field number)
    let heap = heap_plan(nodes, heap_seg_max)?;
    Ok(FramePlan { slot, needs_zero, list, pair, slot_w, dyn_w, heap })

}
