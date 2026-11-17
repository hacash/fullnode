use super::super::*;

impl<'a> Lower<'a> {

    /* ================================================================ *
     * T16: arena map descriptor emission.                              *
     *                                                                  *
     * Object layout (bump-allocated, 8-byte aligned):                  *
     *   +0   len      live entry count                                 *
     *   +8   cap      entry capacity (CTR_MAP_CAP = the interpreter's  *
     *                 SpaceCap::compo_length default 128)              *
     *   +16  kused    reserved (key bytes live in the shared arena     *
     *                 bump stream, not inside the object)              *
     *   +24.. entries, CTR_MAP_ENT (40B) each, kept SORTED by the      *
     *         canonical key bytes (byte-lexicographic = BTreeMap       *
     *         order; KEYS/VALUES iterate it as-is):                    *
     *           +0  kptr — absolute arena address of the key bytes     *
     *           +8  klen                                                *
     *           +16 vtag — Value type id (ctr_uint_type_tag / 8 bytes)  *
     *           +24 va   — value lo word / bytes ptr                    *
     *           +32 vb   — value hi word / bytes len                    *
     *                                                                  *
     * The descriptor is the tagged pointer (`ptr | CTR_MAP_DESC_TAG`); *
     * every map helper masks the tag before dereferencing. Key bytes   *
     * are copied into the RW arena, so a removed key's bytes are dead  *
     * space and live keys never move.                                  *
     * ================================================================ */

    /// `dst` := the untagged map object pointer held by map slot `s`.
    pub(crate) fn ctr_map_load_obj(&mut self, s: usize, dst: u8) {
        let cell = self.slot_offset(s as u8);
        self.asm.ld(dst, S0, cell);
        self.asm.andi(dst, dst, -8);
    }

    /// Allocate a map object of `cap` entry cells: S1-bump
    /// `CTR_MAP_HDR + CTR_MAP_ENT*cap` bytes (arena exhaustion = the
    /// bytes-CAT budget face), write the header, `dst` := the object base.
    pub(crate) fn ctr_map_alloc(&mut self, dst: u8, cap: u64) {
        let total = CTR_MAP_HDR as u64 + CTR_MAP_ENT as u64 * cap;
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, total);
        self.emit_arena_check(sc1);
        self.asm.mv(dst, S1);
        self.asm.add(S1, S1, sc1);
        let a0 = self.a0;
        self.asm.sd(ZERO, dst, 0); // len = 0
        self.asm.li(a0, cap);
        self.asm.sd(a0, dst, 8); // cap
        self.asm.sd(ZERO, dst, 16); // kused (reserved)
    }

    /// Stage a normalized key (kptr, klen) into the map scratch area.
    pub(crate) fn map_stage_key(&mut self, kptr: u8, klen: u8) {
        let o = self.map_scratch_off;
        debug_assert!(o >= 0, "map scratch area missing");
        self.asm.sd(kptr, S0, o + self.map_kptr_cell);
        self.asm.sd(klen, S0, o + self.map_klen_cell);
    }

    pub(crate) fn map_scratch_cell(&self, cell: i32) -> i32 {
        debug_assert!(self.map_scratch_off >= 0, "map scratch area missing");
        self.map_scratch_off + cell
    }

    /// Evaluate a map key's expression only (no key faces yet) — the
    /// two-operand forms must evaluate BOTH operands before the first key
    /// face, exactly like the interpreter's stack build.
    pub(crate) fn emit_map_key_eval(&mut self, key: &dyn IRNode, op: Bytecode) -> Ret<(Ty, usize, usize)> {
        let ty = node_res_ty(key, self.slot)?;
        let before = self.vstack.len();
        match ty {
            Ty::Int => self.eval_int(key)?,
            Ty::Bytes | Ty::Address => self.eval_bytes(key)?,
            other => {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel \
                     subset (the map key must be a uint or bytes scalar, got {})",
                    ty_name(other)
                )
            }
        }
        let ent = self.vstack.len() - before;
        Ok((ty, before, ent))
    }

    /// Apply the key faces (Nil/Bool/empty-bytes → 73) and normalize the
    /// evaluated key into the scratch cells, then pop its value entries.
    pub(crate) fn emit_map_key_finish(
        &mut self,
        key: &dyn IRNode,
        op: Bytecode,
        ty: Ty,
        before: usize,
        ent: usize,
    ) -> Ret<()> {
        if self.nil_val(key) {
            self.nil_guard_tag(before, 73);
        }
        if ty == Ty::Int {
            match self.int_width(key) {
                Some(1) => {
                    // Bool key: extract_key_bytes rejects Bool (73)
                    for _ in 0..ent {
                        self.pop_vreg();
                    }
                    self.trap(73);
                    return Ok(());
                }
                Some(w) if w >= 8 => {
                    let wbytes = ((w / 8) as u32).min(16);
                    let lo = loc_reg(self.vstack[before]);
                    let hi = if w == 128 && ent >= 2 {
                        Some(loc_reg(self.vstack[before + 1]))
                    } else {
                        None
                    };
                    let (a0, sc1) = (self.a0, self.asm.sc1);
                    emit_ctr_map_key_uint(
                        &mut self.asm,
                        lo,
                        hi,
                        wbytes,
                        a0,
                        sc1,
                        self.fault_cast,
                        self.arena_end,
                    );
                    self.map_stage_key(a0, sc1);
                }
                _ => {
                    for _ in 0..ent {
                        self.pop_vreg();
                    }
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel \
                         subset (the map key has no static uint width)"
                    );
                }
            }
        } else {
            // Bytes/Address: (ptr, len); empty → 73, then a private copy
            let src = loc_reg(self.vstack[before]);
            let len = loc_reg(self.vstack[before + 1]);
            let l73 = self.nil_thunk(73);
            self.asm.beq(len, ZERO, l73);
            self.emit_arena_check(len);
            let a0 = self.a0;
            self.asm.mv(a0, S1);
            self.asm.sd(a0, S0, self.map_scratch_cell(self.map_kptr_cell));
            self.emit_memcpy(src, len);
            self.asm.sd(len, S0, self.map_scratch_cell(self.map_klen_cell));
        }
        for _ in 0..ent {
            self.pop_vreg();
        }
        Ok(())
    }

    /// Evaluate + normalize a single-operand map key.
    pub(crate) fn emit_map_key(&mut self, key: &dyn IRNode, op: Bytecode) -> Ret<()> {
        let (ty, before, ent) = self.emit_map_key_eval(key, op)?;
        self.emit_map_key_finish(key, op, ty, before, ent)
    }

    /// Reload the staged key into two fresh pool registers (kptr, klen).
    pub(crate) fn map_load_key_regs(&mut self) -> (u8, u8) {
        let kptr = self.push_vreg();
        let klen = self.push_vreg();
        let o = self.map_scratch_off;
        self.asm.ld(kptr, S0, o + self.map_kptr_cell);
        self.asm.ld(klen, S0, o + self.map_klen_cell);
        (kptr, klen)
    }

    /// Stage a value pair into the map scratch cells (v0, v1).
    pub(crate) fn map_stage_value(&mut self, v0: u8, v1: u8) {
        let o = self.map_scratch_off;
        self.asm.sd(v0, S0, o + self.map_v0_cell);
        self.asm.sd(v1, S0, o + self.map_v1_cell);
    }

    /// Evaluate a value expression of the map's species and stage it.
    pub(crate) fn emit_map_value_stage(&mut self, value: &dyn IRNode, kind: MapValKind, op: Bytecode) -> Ret<()> {
        // The interpreter's `check_scalar` ADMITS Nil, storing a Nil entry
        // that only faults when read back; the arena entry cell has no Nil
        // tag machinery (the list-element BUG-029 precedent). B-family
        // reject: bind the value from a definite assignment.
        if self.nil_val(value) {
            return errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset                  (the map value may be Nil: the interpreter stores a Nil entry and faults                  only when it is read back — compute the value from definite assignments)"
            );
        }
        match kind {
            MapValKind::Uint(128) => {
                self.eval_int(value)?;
                let d = self.vstack.len();
                let hi = loc_reg(self.vstack[d - 1]);
                let lo = loc_reg(self.vstack[d - 2]);
                self.map_stage_value(lo, hi);
                self.pop_vreg();
                self.pop_vreg();
            }
            MapValKind::Uint(_) | MapValKind::Unset => {
                self.eval_int(value)?;
                let v = loc_reg(self.vstack[self.vstack.len() - 1]);
                self.map_stage_value(v, ZERO);
                self.pop_vreg();
            }
            MapValKind::Bytes => {
                self.eval_bytes(value)?;
                let d = self.vstack.len();
                let len = loc_reg(self.vstack[d - 1]);
                let ptr = loc_reg(self.vstack[d - 2]);
                self.map_stage_value(ptr, len);
                self.pop_vreg();
                self.pop_vreg();
            }
        }
        Ok(())
    }

    /// Find the staged key in `obj`; returns nothing — `pos`/`found` are the
    /// caller's pool registers, and the staged key's registers stay on the
    /// vstack above them (kptr, klen).
    pub(crate) fn emit_map_find_staged(&mut self, obj: u8, pos: u8, found: u8) {
        let (kptr, klen) = self.map_load_key_regs();
        let (x, y, z) = (self.a0, self.asm.sc1, self.a1);
        emit_ctr_map_find(&mut self.asm, obj, kptr, klen, pos, found, x, y, z);
    }

    /// Insert one entry at sorted position `pos`, shifting later entries
    /// right; then len++. `tag_reg` holds the value's type id.
    pub(crate) fn emit_map_insert_at(
        &mut self,
        obj: u8,
        pos: u8,
        kptr: u8,
        klen: u8,
        tag_reg: u8,
        va: u8,
        vb: u8,
    ) {
        let (a0, a1, sc1) = (self.a0, self.a1, self.asm.sc1);
        let word = self.push_vreg();
        // wpos = pos*5 (a0), wlen = len*5 (sc1)
        self.asm.slli_(a0, pos, 2);
        self.asm.add(a0, a0, pos);
        self.asm.ld(a1, obj, 0);
        self.asm.slli_(sc1, a1, 2);
        self.asm.add(sc1, sc1, a1);
        let l_shift = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.asm.beq(sc1, a0, l_done);
        self.asm.addi(a1, sc1, -1); // w
        self.place(l_shift);
        self.asm.slli_(sc1, a1, 3);
        self.asm.addi(sc1, sc1, CTR_MAP_HDR as i32);
        self.asm.add(sc1, obj, sc1);
        self.asm.ld(word, sc1, 0);
        self.asm.sd(word, sc1, CTR_MAP_ENT as i32);
        self.asm.beq(a1, a0, l_done);
        self.asm.addi(a1, a1, -1);
        self.asm.jal_(ZERO, l_shift);
        self.place(l_done);
        // entry address = obj + 24 + pos*40
        self.asm.slli_(a0, pos, 5);
        self.asm.slli_(sc1, pos, 3);
        self.asm.add(a0, a0, sc1);
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(a0, obj, a0);
        self.asm.sd(kptr, a0, 0);
        self.asm.sd(klen, a0, 8);
        self.asm.sd(tag_reg, a0, 16);
        self.asm.sd(va, a0, 24);
        self.asm.sd(vb, a0, 32);
        // len++
        self.asm.ld(a1, obj, 0);
        self.asm.addi(a1, a1, 1);
        self.asm.sd(a1, obj, 0);
        self.pop_vreg(); // word
    }

    /// Remove entry `pos` (the caller proved it exists): shift the following
    /// entries left, then len--.
    pub(crate) fn emit_map_remove_at(&mut self, obj: u8, pos: u8) {
        let (a0, a1, sc1) = (self.a0, self.a1, self.asm.sc1);
        let word = self.push_vreg();
        let wlen = self.push_vreg();
        // wstart = pos*5+5 (a0), wlen = len*5
        self.asm.slli_(a0, pos, 2);
        self.asm.add(a0, a0, pos);
        self.asm.addi(a0, a0, 5);
        self.asm.ld(a1, obj, 0);
        self.asm.slli_(wlen, a1, 2);
        self.asm.add(wlen, wlen, a1); // wlen = len*5
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.asm.beq(a0, wlen, l_done);
        self.asm.mv(a1, a0); // w
        self.place(l_loop);
        self.asm.slli_(sc1, a1, 3);
        self.asm.addi(sc1, sc1, CTR_MAP_HDR as i32);
        self.asm.add(sc1, obj, sc1);
        self.asm.ld(word, sc1, 0);
        self.asm.sd(word, sc1, -(CTR_MAP_ENT as i32));
        self.asm.addi(a1, a1, 1);
        self.asm.bltu(a1, wlen, l_loop);
        self.place(l_done);
        self.asm.ld(a1, obj, 0);
        self.asm.addi(a1, a1, -1);
        self.asm.sd(a1, obj, 0);
        self.pop_vreg();
        self.pop_vreg();
    }

    /// Overwrite the value of existing entry `pos` (ITEMSET / INSERT of an
    /// existing key): no shift, length unchanged.
    pub(crate) fn emit_map_store_entry(
        &mut self,
        obj: u8,
        pos: u8,
        tag_reg: u8,
        va: u8,
        vb: u8,
    ) {
        let (a0, sc1) = (self.a0, self.asm.sc1);
        self.asm.slli_(a0, pos, 5);
        self.asm.slli_(sc1, pos, 3);
        self.asm.add(a0, a0, sc1);
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(a0, obj, a0);
        self.asm.sd(tag_reg, a0, 16);
        self.asm.sd(va, a0, 24);
        self.asm.sd(vb, a0, 32);
    }

    /// The value type-id constant for a map's stored species.
    pub(crate) fn map_value_tag(kind: MapValKind) -> u64 {
        match kind {
            MapValKind::Uint(w) => ctr_uint_type_tag(w),
            MapValKind::Bytes => 8, // ValueTy::Bytes
            MapValKind::Unset => ctr_uint_type_tag(64),
        }
    }

    /// The map slot's final value species (plan fixed it).
    pub(crate) fn map_kind_of_slot(&self, s: usize) -> MapValKind {
        map_slot_kind(self.slot, s)
    }

    /// `PUT <slot> := new_map()/map{..}` — allocate the fresh object, fill it
    /// pair by pair (evaluation in IR order, then key normalization, the
    /// duplicate 84 face, the 128-entry cap 40 face, and the sorted insert),
    /// then write the tagged descriptor into the slot cell.
    pub(crate) fn emit_map_create(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        let s = slot_idx as usize;
        let kind = self.map_kind_of_slot(s);
        match map_create_src(src) {
            Some(MapCreate::NewMap) => {
                let obj = self.push_vreg();
                self.ctr_map_alloc(obj, CTR_MAP_CAP);
                let a0 = self.a0;
                self.asm.addi(a0, obj, CTR_MAP_DESC_TAG as i32);
                let cell = self.slot_offset(slot_idx);
                self.asm.sd(a0, S0, cell);
                self.pop_vreg();
                Ok(())
            }
            Some(MapCreate::PackMap { arr, n }) => {
                // the interpreter's take_items_from_ops faces: n == 0 → 84,
                // n > 2*compo_length → 40
                if n == 0 {
                    self.trap(84);
                    return Ok(());
                }
                if n > 2 * CTR_MAP_CAP {
                    self.trap(40);
                    return Ok(());
                }
                let obj = self.push_vreg();
                self.ctr_map_alloc(obj, CTR_MAP_CAP);
                // the object base survives the element evaluations in the
                // scratch area (element expressions may use the whole pool)
                self.asm.sd(obj, S0, self.map_scratch_cell(32));
                self.pop_vreg();
                let pairs = n / 2;
                for j in 0..pairs as usize {
                    let k = &arr.subs[j * 2];
                    let v = &arr.subs[j * 2 + 1];
                    let (kty, kbefore, kent) = self.emit_map_key_eval(&**k, Bytecode::PACKMAP)?;
                    self.emit_map_value_stage(&**v, kind, Bytecode::PACKMAP)?;
                    self.emit_map_key_finish(&**k, Bytecode::PACKMAP, kty, kbefore, kent)?;
                    let objr = self.push_vreg();
                    self.asm.ld(objr, S0, self.map_scratch_cell(32)); // obj
                    let pos = self.push_vreg();
                    let found = self.push_vreg();
                    self.emit_map_find_staged(objr, pos, found);
                    // duplicate key → 84 (before any insert)
                    let l84 = self.nil_thunk(84);
                    self.asm.bne(found, ZERO, l84);
                    self.pop_vreg(); // found
                    self.pop_vreg(); // staged klen
                    self.pop_vreg(); // staged kptr
                    // cap face: a NEW key at len >= 128 → 40
                    let l40 = self.nil_thunk(40);
                    let sc1 = self.asm.sc1;
                    self.asm.ld(sc1, objr, 0);
                    self.asm.li(self.a1, CTR_MAP_CAP);
                    self.asm.bgeu(sc1, self.a1, l40);
                    // value from the scratch cells
                    let vb = self.push_vreg();
                    let va = self.push_vreg();
                    let o2 = self.map_scratch_off;
                    self.asm.ld(va, S0, o2 + self.map_v0_cell);
                    self.asm.ld(vb, S0, o2 + self.map_v1_cell);
                    let tag = self.push_vreg();
                    self.asm.li(tag, Self::map_value_tag(kind));
                    let (kptr, klen) = self.map_load_key_regs();
                    self.emit_map_insert_at(objr, pos, kptr, klen, tag, va, vb);
                    for _ in 0..7 {
                        self.pop_vreg(); // klen,kptr,tag,va,vb,objr,pos
                    }
                }
                // descriptor into the slot cell
                let a0 = self.a0;
                self.asm.ld(a0, S0, self.map_scratch_cell(32));
                self.asm.addi(a0, a0, CTR_MAP_DESC_TAG as i32);
                let cell = self.slot_offset(slot_idx);
                self.asm.sd(a0, S0, cell);
                Ok(())
            }
            None => errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (internal: map creation shape lost)"
            ),
        }
    }

    /// `PUT <list slot> := keys(m)/values(m)` — allocate a fresh arena list
    /// with the map's CURRENT length and copy every entry (sorted order) as
    /// a tagged cell: KEYS → (tag=8, kptr, klen); VALUES → the entry's own
    /// (vtag, va, vb). The result is a snapshot: later map mutations are
    /// not visible (the interpreter builds a new VecDeque).
    pub(crate) fn emit_map_to_list(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        let sn = src
            .as_any()
            .downcast_ref::<IRNodeSingle>()
            .ok_or("rv64 lower: map list node shape unexpected")?;
        let op = Bytecode::try_from_u8(src.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        let s = map_receiver_slot(&*sn.subx, self.slot, op)?;
        let obj = self.push_vreg();
        self.ctr_map_load_obj(s, obj);
        let len = self.push_vreg();
        self.asm.ld(len, obj, 0);
        // allocate a list object (16 + 24*128) with the map's length
        let total = CTR_LIST_HDR + CTR_ELEM_BYTES * CTR_LIST_CAP;
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, total);
        self.emit_arena_check(sc1);
        let lst = self.push_vreg();
        self.asm.mv(lst, S1);
        self.asm.add(S1, S1, sc1);
        let a0 = self.a0;
        self.asm.li(a0, CTR_LIST_CAP);
        self.asm.sd(a0, lst, 8);
        self.asm.sd(len, lst, 0);
        // walk both tables
        let eb = self.push_vreg();
        let cb = self.push_vreg();
        let cnt = self.push_vreg();
        self.asm.addi(eb, obj, CTR_MAP_HDR as i32);
        self.asm.addi(cb, lst, CTR_LIST_HDR as i32);
        self.asm.mv(cnt, len);
        let t = self.push_vreg();
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(cnt, ZERO, l_done);
        if op == Bytecode::KEYS {
            self.asm.li(t, 8);
            self.asm.sd(t, cb, 0);
            self.asm.ld(t, eb, 0);
            self.asm.sd(t, cb, 8);
            self.asm.ld(t, eb, 8);
            self.asm.sd(t, cb, 16);
        } else {
            self.asm.ld(t, eb, 16);
            self.asm.sd(t, cb, 0);
            self.asm.ld(t, eb, 24);
            self.asm.sd(t, cb, 8);
            self.asm.ld(t, eb, 32);
            self.asm.sd(t, cb, 16);
        }
        self.asm.addi(eb, eb, CTR_MAP_ENT as i32);
        self.asm.addi(cb, cb, CTR_ELEM_BYTES as i32);
        self.asm.addi(cnt, cnt, -1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // bind the list descriptor
        let cell = self.slot_offset(slot_idx);
        self.asm.sd(lst, S0, cell);
        for _ in 0..7 {
            self.pop_vreg();
        }
        Ok(())
    }

    /// The map INSERT statement: evaluate key then value (all operands
    /// first), then the key 73 faces, the sorted lookup, the existing-key
    /// overwrite / new-key 40 cap face, and the sorted insert.
    pub(crate) fn emit_map_insert_stmt(&mut self, n: &IRNodeTriple) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::INSERT)?;
        let kind = self.map_kind_of_slot(ms);
        let (kty, kbefore, kent) = self.emit_map_key_eval(&*n.suby, Bytecode::INSERT)?;
        self.emit_map_value_stage(&*n.subz, kind, Bytecode::INSERT)?;
        self.emit_map_key_finish(&*n.suby, Bytecode::INSERT, kty, kbefore, kent)?;
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        {
            self.emit_map_find_staged(obj, pos, found);
            // found → overwrite the value in place. The two runtime paths
            // have different temporary footprints; save the compile-time
            // vstack (both paths converge on obj,pos) so each arm is
            // compiled against its own runtime state.
            let l_over = self.asm.new_label();
            let l_done = self.asm.new_label();
            let arm_v = self.vstack.clone();
            let arm_sp = self.span_open;
            self.asm.bne(found, ZERO, l_over);
            // new key: cap face (len >= 128 → 40)
            let l40 = self.nil_thunk(40);
            let sc1 = self.asm.sc1;
            self.asm.ld(sc1, obj, 0);
            self.asm.li(self.a1, CTR_MAP_CAP);
            self.asm.bgeu(sc1, self.a1, l40);
            // drop the staged key regs and `found` (obj,pos remain)
            self.pop_vreg();
            self.pop_vreg();
            self.pop_vreg();
            let vb = self.push_vreg();
            let va = self.push_vreg();
            let o2 = self.map_scratch_off;
            self.asm.ld(va, S0, o2 + self.map_v0_cell);
            self.asm.ld(vb, S0, o2 + self.map_v1_cell);
            let tag = self.push_vreg();
            self.asm.li(tag, Self::map_value_tag(kind));
            let (kptr, klen) = self.map_load_key_regs();
            self.emit_map_insert_at(obj, pos, kptr, klen, tag, va, vb);
            for _ in 0..5 {
                self.pop_vreg(); // klen,kptr,tag,va,vb
            }
            self.asm.jal_(ZERO, l_done);
            self.vstack = arm_v.clone();
            self.span_open = arm_sp;
            self.place(l_over);
            // overwrite: drop the staged key regs and `found`
            self.pop_vreg();
            self.pop_vreg();
            self.pop_vreg();
            let vb = self.push_vreg();
            let va = self.push_vreg();
            let o2 = self.map_scratch_off;
            self.asm.ld(va, S0, o2 + self.map_v0_cell);
            self.asm.ld(vb, S0, o2 + self.map_v1_cell);
            let tag = self.push_vreg();
            self.asm.li(tag, Self::map_value_tag(kind));
            self.emit_map_store_entry(obj, pos, tag, va, vb);
            for _ in 0..3 {
                self.pop_vreg(); // tag,va,vb
            }
            self.place(l_done);
            self.pop_vreg(); // pos
            self.pop_vreg(); // obj
        }
        Ok(())
    }

    /// The map ITEMSET statement: overwrite an EXISTING key only (missing →
    /// 85, never an insert). Interpreter face order: value check_scalar (76
    /// — structurally absent for the admitted scalar species), then the key
    /// 73 face, then the missing-key 85.
    pub(crate) fn emit_map_itemset_stmt(&mut self, n: &IRNodeTriple) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::ITEMSET)?;
        let kind = self.map_kind_of_slot(ms);
        let (kty, kbefore, kent) = self.emit_map_key_eval(&*n.suby, Bytecode::ITEMSET)?;
        self.emit_map_value_stage(&*n.subz, kind, Bytecode::ITEMSET)?;
        self.emit_map_key_finish(&*n.suby, Bytecode::ITEMSET, kty, kbefore, kent)?;
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        self.emit_map_find_staged(obj, pos, found);
        let l85 = self.nil_thunk(85);
        self.asm.beq(found, ZERO, l85);
        self.pop_vreg(); // staged klen
        self.pop_vreg(); // staged kptr
        self.pop_vreg(); // found
        let vb = self.push_vreg();
        let va = self.push_vreg();
        let o2 = self.map_scratch_off;
        self.asm.ld(va, S0, o2 + self.map_v0_cell);
        self.asm.ld(vb, S0, o2 + self.map_v1_cell);
        let tag = self.push_vreg();
        self.asm.li(tag, Self::map_value_tag(kind));
        self.emit_map_store_entry(obj, pos, tag, va, vb);
        for _ in 0..5 {
            self.pop_vreg(); // tag,va,vb,pos,obj
        }
        Ok(())
    }

    /// The map REMOVE statement: key 73, missing → 85, shift-left removal.
    pub(crate) fn emit_map_remove_stmt(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::REMOVE)?;
        self.emit_map_key(&*n.suby, Bytecode::REMOVE)?;
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        self.emit_map_find_staged(obj, pos, found);
        let l85 = self.nil_thunk(85);
        self.asm.beq(found, ZERO, l85);
        self.pop_vreg(); // staged klen
        self.pop_vreg(); // staged kptr
        self.pop_vreg(); // found
        self.emit_map_remove_at(obj, pos);
        self.pop_vreg(); // pos
        self.pop_vreg(); // obj
        Ok(())
    }

    /// HASKEY(map): key faces then a 0/1 result from the sorted lookup.
    pub(crate) fn emit_map_haskey(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::HASKEY)?;
        self.emit_map_key(&*n.suby, Bytecode::HASKEY)?;
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        self.emit_map_find_staged(obj, pos, found);
        self.pop_vreg(); // staged klen
        self.pop_vreg(); // staged kptr
        // vstack: obj, pos, found — save found's register, drop the three,
        // re-push the 0/1 result
        let found_r = loc_reg(self.vstack[self.vstack.len() - 1]);
        self.pop_vreg(); // found
        self.pop_vreg(); // pos
        self.pop_vreg(); // obj
        let out = self.push_vreg();
        self.asm.mv(out, found_r);
        Ok(())
    }

    /// ITEMGET(map): key faces, missing → 85, then the tagged value load.
    pub(crate) fn emit_map_itemget(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::ITEMGET)?;
        let kind = self.map_kind_of_slot(ms);
        self.emit_map_key(&*n.suby, Bytecode::ITEMGET)?;
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        self.emit_map_find_staged(obj, pos, found);
        let l85 = self.nil_thunk(85);
        self.asm.beq(found, ZERO, l85);
        self.pop_vreg(); // staged klen
        self.pop_vreg(); // staged kptr
        // load the value payload (fresh pool registers = the result)
        let (a0, sc1) = (self.a0, self.asm.sc1);
        self.asm.slli_(a0, pos, 5);
        self.asm.slli_(sc1, pos, 3);
        self.asm.add(a0, a0, sc1);
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(a0, obj, a0);
        match kind {
            MapValKind::Uint(128) => {
                let lo = self.push_vreg();
                let hi = self.push_vreg();
                self.asm.ld(lo, a0, 24);
                self.asm.ld(hi, a0, 32);
            }
            MapValKind::Bytes => {
                let ptr = self.push_vreg();
                let len = self.push_vreg();
                self.asm.ld(ptr, a0, 24);
                self.asm.ld(len, a0, 32);
            }
            _ => {
                let v = self.push_vreg();
                self.asm.ld(v, a0, 24);
            }
        }
        // pop found,pos,obj (the result sits above them)
        let res = self.vstack.len();
        let _ = res;
        // saved: result entries occupy the top; pop them, drop the three,
        // then re-push the result from its registers.
        // Simpler: move the result behind the bookkeeping by popping the
        // result now, then the bookkeeping, then re-pushing via refs.
        // (registers are pool homes of the just-popped entries, so a
        // re-push below them is safe only if no push happens in between —
        // pop/push restore the same registers in order.)
        let nres = match kind {
            MapValKind::Uint(128) | MapValKind::Bytes => 2,
            _ => 1,
        };
        let mut regs = [ZERO; 2];
        for i in (0..nres).rev() {
            regs[i] = loc_reg(self.vstack[self.vstack.len() - nres + i]);
        }
        for _ in 0..nres {
            self.pop_vreg();
        }
        self.pop_vreg(); // found
        self.pop_vreg(); // pos
        self.pop_vreg(); // obj
        for i in 0..nres {
            let r = self.push_vreg();
            if r != regs[i] {
                self.asm.mv(r, regs[i]);
            }
        }
        Ok(())
    }

    /// LENGTH(map): the object's live entry count (U32 on the interpreter;
    /// the kernel keeps the u32 width model for LENGTH).
    pub(crate) fn emit_map_length(&mut self, n: &IRNodeSingle) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::LENGTH)?;
        let a0 = self.a0;
        self.ctr_map_load_obj(ms, a0);
        self.asm.ld(a0, a0, 0);
        let out = self.push_vreg();
        self.asm.mv(out, a0);
        Ok(())
    }

    /// MERGE(a, b): self-merge → 80, then (in b's sorted order) each source
    /// value's check_scalar (structurally scalar), duplicate key → 84, then
    /// the combined-length cap → 40, then the entries are inserted.
    pub(crate) fn emit_map_merge(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ma = map_receiver_slot(&*n.subx, self.slot, Bytecode::MERGE)?;
        let mb = map_receiver_slot(&*n.suby, self.slot, Bytecode::MERGE)?;
        let a0 = self.a0;
        let a1 = self.a1;
        self.ctr_map_load_obj(ma, a0);
        let objb = self.push_vreg();
        self.ctr_map_load_obj(mb, objb);
        // Rc::ptr_eq(self, src) → CompoOpInvalid 80
        let l80 = self.nil_thunk(80);
        self.asm.beq(a0, objb, l80);
        // persist the two object bases and b's length in the scratch area
        self.asm.sd(a0, S0, self.map_scratch_cell(32));
        self.asm.sd(objb, S0, self.map_scratch_cell(40));
        self.asm.ld(a1, objb, 0);
        self.asm.sd(a1, S0, self.map_scratch_cell(48)); // lenB
        self.pop_vreg();
        // duplicate pass, in b's sorted order
        self.asm.sd(ZERO, S0, self.map_scratch_cell(56)); // i = 0
        let l_dupl = self.asm.new_label();
        let l_dupl_done = self.asm.new_label();
        self.place(l_dupl);
        {
            let sc1 = self.asm.sc1;
            let i = self.push_vreg();
            self.asm.ld(i, S0, self.map_scratch_cell(56));
            self.asm.ld(sc1, S0, self.map_scratch_cell(48));
            self.asm.bgeu(i, sc1, l_dupl_done);
            let objb_r = self.push_vreg();
            self.asm.ld(objb_r, S0, self.map_scratch_cell(40));
            self.stage_map_entry_key(objb_r, i)?;
            self.pop_vreg(); // objb_r
            self.pop_vreg(); // i
            let obj = self.push_vreg();
            self.asm.ld(obj, S0, self.map_scratch_cell(32));
            let pos = self.push_vreg();
            let found = self.push_vreg();
            self.emit_map_find_staged(obj, pos, found);
            let l84 = self.nil_thunk(84);
            self.asm.bne(found, ZERO, l84);
            self.pop_vreg(); // klen
            self.pop_vreg(); // kptr
            self.pop_vreg(); // found
            self.pop_vreg(); // pos
            self.pop_vreg(); // obj
            self.asm.ld(a0, S0, self.map_scratch_cell(56));
            self.asm.addi(a0, a0, 1);
            self.asm.sd(a0, S0, self.map_scratch_cell(56));
            self.asm.jal_(ZERO, l_dupl);
        }
        self.place(l_dupl_done);
        // combined length cap → 40
        let l40 = self.nil_thunk(40);
        let sc1 = self.asm.sc1;
        self.asm.ld(a0, S0, self.map_scratch_cell(32));
        self.asm.ld(a0, a0, 0); // lenA
        self.asm.ld(sc1, S0, self.map_scratch_cell(48)); // lenB
        self.asm.add(sc1, sc1, a0);
        self.asm.li(a0, CTR_MAP_CAP);
        self.asm.bgeu(sc1, a0, l40);
        // insert pass (a keeps its sorted order via the sorted insert)
        self.asm.sd(ZERO, S0, self.map_scratch_cell(56)); // i = 0
        let l_ins = self.asm.new_label();
        let l_ins_done = self.asm.new_label();
        self.place(l_ins);
        {
            let sc1 = self.asm.sc1;
            let i = self.push_vreg();
            self.asm.ld(i, S0, self.map_scratch_cell(56));
            self.asm.ld(sc1, S0, self.map_scratch_cell(48));
            self.asm.bgeu(i, sc1, l_ins_done);
            let objb_r = self.push_vreg();
            self.asm.ld(objb_r, S0, self.map_scratch_cell(40));
            self.stage_map_entry_key(objb_r, i)?;
            self.pop_vreg(); // objb_r
            self.pop_vreg(); // i
            let obj = self.push_vreg();
            self.asm.ld(obj, S0, self.map_scratch_cell(32));
            let pos = self.push_vreg();
            let found = self.push_vreg();
            self.emit_map_find_staged(obj, pos, found);
            // the duplicate pass proved found == 0; keep it defensive
            let l84 = self.nil_thunk(84);
            self.asm.bne(found, ZERO, l84);
            self.pop_vreg(); // klen
            self.pop_vreg(); // kptr
            self.pop_vreg(); // found
            // b[i]'s value words
            let vb = self.push_vreg();
            let va = self.push_vreg();
            let tag = self.push_vreg();
            let idx = self.push_vreg();
            self.asm.ld(idx, S0, self.map_scratch_cell(56));
            let objb_r = self.push_vreg();
            self.asm.ld(objb_r, S0, self.map_scratch_cell(40));
            self.load_map_entry_value(objb_r, idx, va, vb, tag)?;
            self.pop_vreg(); // objb_r
            self.pop_vreg(); // idx
            let (kptr, klen) = self.map_load_key_regs();
            self.emit_map_insert_at(obj, pos, kptr, klen, tag, va, vb);
            for _ in 0..5 {
                self.pop_vreg(); // klen,kptr,tag,va,vb
            }
            self.pop_vreg(); // pos
            self.pop_vreg(); // obj
            self.asm.ld(a0, S0, self.map_scratch_cell(56));
            self.asm.addi(a0, a0, 1);
            self.asm.sd(a0, S0, self.map_scratch_cell(56));
            self.asm.jal_(ZERO, l_ins);
        }
        self.place(l_ins_done);
        Ok(())
    }
}
