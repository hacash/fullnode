use super::super::*;

impl<'a> Lower<'a> {

    /* ---- B14 (impl/arena-mapflow): the len-zero map fact ---- */

    /// Establish "map slot `s`'s object was created by THIS body's
    /// `PUT s := new_map()` in the current straight-line run, so its live
    /// length is the compile-time constant 0". Killed at every join label
    /// (`place`), by every other map operation and by an alias PUT — the
    /// first INSERT/ITEMSET consumes it.
    pub(crate) fn map_zero_establish(&mut self, s: usize) {
        if std::env::var("RV64_INV_CHECK").is_ok() {
            if !is_map_slot(self.slot.get(s).copied().unwrap_or(SlotTy::None)) {
                panic!("INV B14 len-zero fact established on non-map slot {s}");
            }
        }
        self.map_zero[s] = true;
    }

    /// Kill every len-zero fact (conservative wholesale — the aliasing
    /// graph between map slots is not tracked, so any operation that could
    /// mutate or re-bind a map object clears the whole table).
    pub(crate) fn map_zero_kill_all(&mut self) {
        self.map_zero.fill(false);
    }

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
     * The descriptor is the BARE object pointer (B6: the historical     *
     * low-bit species tag `ptr | 1` had no runtime reader — every      *
     * species dispatch is the static `is_map_slot` walk, the alias     *
     * copy passes the word through, and the T17 element-cell sub-tag   *
     * is an independent whole word). Key bytes are copied into the     *
     * RW arena, so a removed key's bytes are dead space and live keys  *
     * never move.                                                      *
     * ================================================================ */

    /// `dst` := the map object pointer held by map slot `s` (B6: no tag
    /// mask — the slot cell holds the bare pointer).
    pub(crate) fn ctr_map_load_obj(&mut self, s: usize, dst: u8) {
        let cell = self.slot_offset(s as u8);
        self.asm.ld(dst, S0, cell);
    }

    /// Allocate a map object of `cap` entry cells: S1-bump
    /// `CTR_MAP_HDR + CTR_MAP_ENT*cap` bytes (arena exhaustion = the
    /// bytes-CAT budget face), write the header, `dst` := the object base.
    pub(crate) fn ctr_map_alloc(&mut self, dst: u8, cap: u64) {
        self.cat_top_kill(); // B9: this allocation bumps S1
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
    /// evaluated key. `direct` = the single-key operations (REMOVE /
    /// HASKEY / ITEMGET / ITEMSET — survey B4(c)): the normalized key
    /// materializes as a LIVE pool pair (kptr, klen) left on the vstack
    /// instead of a frame-cell round trip (sd×2 at the staging site +
    /// ld×2 at `map_load_key_regs`); the caller passes the pair straight
    /// to `emit_map_find_direct`. Staged mode (INSERT/PACKMAP) keeps the
    /// scratch cells: those sites re-load the key after the find for the
    /// insert.
    pub(crate) fn emit_map_key_finish(
        &mut self,
        key: &dyn IRNode,
        op: Bytecode,
        ty: Ty,
        before: usize,
        ent: usize,
        direct: bool,
    ) -> Ret<()> {
        // B9: the dynamic int key's normalization bumps S1 (emit_ctr_map_key_uint)
        // and the bytes key's private copy runs emit_memcpy — both kill the
        // arena-top fact through their own sites; this entry kill covers the
        // normalization path wholesale (a const pool-staged key kills
        // conservatively without bumping anything).
        self.cat_top_kill();
        if self.nil_val(key) {
            self.nil_guard_tag(before, 73);
        }
        if ty == Ty::Int {
            match self.int_width(key) {
                Some(1) => {
                    // Bool key: extract_key_bytes rejects Bool (73). The
                    // trap is unconditional — everything the caller emits
                    // below is dead code — but direct mode's contract is
                    // "the normalized (kptr, klen) pair sits on the
                    // vstack", so park two (never-read) entries to keep
                    // the stack shape well-formed.
                    for _ in 0..ent {
                        self.pop_vreg();
                    }
                    self.trap(73);
                    if direct {
                        let _ = self.push_vreg();
                        let _ = self.push_vreg();
                    }
                    return Ok(());
                }
                Some(w) if w >= 8 => {
                    // B1: a constant int key stages its canonical bytes
                    // straight from the read-only literal pool instead of
                    // the ~158-inst runtime normalization. The deleted
                    // faces are structurally unreachable: the literal is a
                    // uint (never Nil/Bool — `ctr_const_map_key` rejects
                    // those, so the 73 guards above keep their paths) and
                    // no arena bytes are bumped (the key-budget fault
                    // cannot fire at a site that allocates nothing). The
                    // staged (kptr, klen) is byte-identical to the runtime
                    // form (`ctr_map_key_bytes` mirrors `uint_key_bytes`),
                    // and the pool address is legal read-only storage:
                    // every downstream consumer only lbu-reads key bytes
                    // (find) or sd-moves the pointer word whole.
                    if let Some(n) = ctr_const_map_key(key) {
                        let kb = ctr_map_key_bytes(n);
                        let lbl = self.lit_label(&kb);
                        if direct {
                            // the literal's own value entries die unread
                            for _ in 0..ent {
                                self.pop_vreg();
                            }
                            let kptr = self.push_vreg();
                            let klen = self.push_vreg();
                            self.lit_addr(kptr, lbl);
                            self.asm.addi(klen, ZERO, kb.len() as i32);
                            return Ok(());
                        }
                        let (a0, sc1) = (self.a0, self.asm.sc1);
                        self.lit_addr(a0, lbl);
                        self.asm.addi(sc1, ZERO, kb.len() as i32);
                        self.map_stage_key(a0, sc1);
                    } else {
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
                        if direct {
                            for _ in 0..ent {
                                self.pop_vreg();
                            }
                            let kptr = self.push_vreg();
                            let klen = self.push_vreg();
                            if kptr != a0 {
                                self.asm.mv(kptr, a0);
                            }
                            if klen != sc1 {
                                self.asm.mv(klen, sc1);
                            }
                            return Ok(());
                        }
                        self.map_stage_key(a0, sc1);
                    }
                }
                _ => {
                    for _ in 0..ent {
                        self.pop_vreg();
                    }
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                         (the map key has no static uint width)"
                    );
                }
            }
        } else {
            // Bytes/Address: (ptr, len); empty → 73, then a private copy.
            // The copy reserves len ROUNDED UP to the 8-byte arena
            // alignment unit and the bump cursor is re-aligned after the
            // byte-wise copy (the emit_ctr_map_key_uint sibling
            // invariant): the next descriptor-object allocation takes its
            // base from this cursor and its consumers mask the low
            // tag/alignment bits off the descriptor, so an unaligned
            // cursor would misalign the next map object (audit F1/F1b).
            let src = loc_reg(self.vstack[before]);
            let len = loc_reg(self.vstack[before + 1]);
            let l73 = self.nil_thunk(73);
            self.asm.beq(len, ZERO, l73);
            let sc1 = self.asm.sc1;
            self.asm.addi(sc1, len, 7);
            self.asm.andi(sc1, sc1, -8);
            self.emit_arena_check(sc1);
            if direct {
                // the copy destination parks in a fresh pool register
                // above (src, len) — emit_memcpy only touches the scratch
                // pair — then the pair re-seats on the dropped entries
                let kptr = self.push_vreg();
                self.asm.mv(kptr, S1);
                self.emit_memcpy(src, len);
                // re-align the cursor (the copy advanced it by len)
                self.asm.addi(sc1, S1, 7);
                self.asm.andi(sc1, sc1, -8);
                self.asm.mv(S1, sc1);
                let klen_r = len;
                self.pop_vreg(); // kptr (bookkeeping; value stays put)
                self.pop_vreg(); // len
                self.pop_vreg(); // src
                let kptr2 = self.push_vreg();
                let klen2 = self.push_vreg();
                if kptr2 != kptr {
                    self.asm.mv(kptr2, kptr);
                }
                if klen2 != klen_r {
                    self.asm.mv(klen2, klen_r);
                }
                return Ok(());
            }
            let a0 = self.a0;
            self.asm.mv(a0, S1);
            self.asm.sd(a0, S0, self.map_scratch_cell(self.map_kptr_cell));
            self.emit_memcpy(src, len);
            self.asm.sd(len, S0, self.map_scratch_cell(self.map_klen_cell));
            // re-align the cursor (the copy advanced it by len)
            self.asm.addi(sc1, S1, 7);
            self.asm.andi(sc1, sc1, -8);
            self.asm.mv(S1, sc1);
        }
        for _ in 0..ent {
            self.pop_vreg();
        }
        Ok(())
    }

    /// Evaluate + normalize a single-operand map key (direct mode: the
    /// normalized (kptr, klen) pool pair stays on the vstack).
    pub(crate) fn emit_map_key(&mut self, key: &dyn IRNode, op: Bytecode) -> Ret<()> {
        let (ty, before, ent) = self.emit_map_key_eval(key, op)?;
        self.emit_map_key_finish(key, op, ty, before, ent, true)
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

    /// B4(a): the find routine's optional fourth scratch. ImcRemap maps a1
    /// and `asm.sc2` onto the same physical register (x31), leaving exactly
    /// three distinct scratches; Im/Imc have four (a0, a1, sc1, sc2 pairwise
    /// distinct). The mode is fixed at emission time — deterministic.
    fn map_find_ebase_scratch(&self) -> Option<u8> {
        if self.a1 != self.asm.sc2 {
            Some(self.asm.sc2)
        } else {
            None
        }
    }

    /// Find the staged key in `obj`; returns nothing — `pos`/`found` are the
    /// caller's pool registers, and the staged key's registers stay on the
    /// vstack above them (kptr, klen).
    pub(crate) fn emit_map_find_staged(&mut self, obj: u8, pos: u8, found: u8) {
        let (kptr, klen) = self.map_load_key_regs();
        let (x, y, z) = (self.a0, self.asm.sc1, self.a1);
        let ebase = self.map_find_ebase_scratch();
        emit_ctr_map_find(&mut self.asm, obj, kptr, klen, pos, found, x, y, z, ebase);
    }

    /// B4(c): the single-key operations' find — the normalized key sits in
    /// the LIVE pool pair (kptr, klen) (emit_map_key_finish in direct mode
    /// left them on the vstack below obj/pos/found), skipping the frame
    /// staging round trip entirely.
    pub(crate) fn emit_map_find_direct(&mut self, obj: u8, pos: u8, found: u8, kptr: u8, klen: u8) {
        let (x, y, z) = (self.a0, self.asm.sc1, self.a1);
        let ebase = self.map_find_ebase_scratch();
        emit_ctr_map_find(&mut self.asm, obj, kptr, klen, pos, found, x, y, z, ebase);
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
    /// B14: a `new_map()` binding re-binds the slot (the old object, and
    /// every len-zero fact with it, dies) and establishes the slot's
    /// len == 0 fact for the first INSERT/ITEMSET of the following
    /// straight-line run. A `map{..}` PACKMAP has pairs >= 1 (n == 0 traps
    /// 84 statically) — no fact.
    pub(crate) fn emit_map_create(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        let s = slot_idx as usize;
        let kind = self.map_kind_of_slot(s);
        self.map_zero_kill_all();
        match map_create_src(src) {
            Some(MapCreate::NewMap) => {
                let obj = self.push_vreg();
                self.ctr_map_alloc(obj, CTR_MAP_CAP);
                let cell = self.slot_offset(slot_idx);
                self.asm.sd(obj, S0, cell);
                self.pop_vreg();
                self.map_zero_establish(s);
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
                let pairs = (n / 2) as usize;
                // B3: when every key is a compile-time uint constant the
                // whole table is laid out statically — see
                // emit_map_pack_const. Otherwise fall back to the pairwise
                // path (each constant key still takes the B1 pool staging).
                let mut ckeys: Vec<u128> = Vec::with_capacity(pairs);
                let mut all_const = true;
                for j in 0..pairs {
                    match ctr_const_map_key(&*arr.subs[j * 2]) {
                        Some(v) => ckeys.push(v),
                        None => {
                            all_const = false;
                            break;
                        }
                    }
                }
                if all_const {
                    return self.emit_map_pack_const(slot_idx, arr, &ckeys, kind);
                }
                let obj = self.push_vreg();
                self.ctr_map_alloc(obj, CTR_MAP_CAP);
                // the object base survives the element evaluations in the
                // scratch area (element expressions may use the whole pool)
                self.asm.sd(obj, S0, self.map_scratch_cell(32));
                self.pop_vreg();
                for j in 0..pairs {
                    let k = &arr.subs[j * 2];
                    let v = &arr.subs[j * 2 + 1];
                    let (kty, kbefore, kent) = self.emit_map_key_eval(&**k, Bytecode::PACKMAP)?;
                    self.emit_map_value_stage(&**v, kind, Bytecode::PACKMAP)?;
                    self.emit_map_key_finish(&**k, Bytecode::PACKMAP, kty, kbefore, kent, false)?;
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

    /// B3: PACKMAP whose keys are ALL compile-time uint constants. The
    /// entry table is laid out at compile time: the keys sort by their
    /// canonical bytes (the BTreeMap order IS the entry-array order —
    /// survey §0), each pair's value is still evaluated in IR pair order
    /// (the interpreter's pairwise loop: pair i's value evaluates when
    /// pair i is processed), and the entry is stored DIRECTLY into its
    /// sorted slot — no find, no insert, no shift, no runtime 84.
    /// `keys[j]` is pair j's constant (already resolved by the caller).
    ///
    /// A compile-time duplicate key is NOT a compile error: the emission
    /// evaluates the values pair by pair up to and INCLUDING the first
    /// duplicate pair, then traps 84 at runtime; the pairs after it are
    /// never evaluated (the interpreter's loop order — duplicate keys
    /// collide on canonical bytes across literal widths, e.g.
    /// `map{1u64:.., 1u8:..}`). The over-cap face needs no per-pair
    /// check: `n > 2*CTR_MAP_CAP` was rejected statically before this
    /// runs and n is even, so pairs ≤ 128 = CTR_MAP_CAP and the live
    /// length can never cross the cap. The alloc stays first — its
    /// arena-budget face precedes every value evaluation exactly like
    /// the pairwise path.
    fn emit_map_pack_const(
        &mut self,
        slot_idx: u8,
        arr: &IRNodeArray,
        keys: &[u128],
        kind: MapValKind,
    ) -> Ret<()> {
        let pairs = keys.len();
        let kb: Vec<Vec<u8>> = keys.iter().map(|&n| ctr_map_key_bytes(n)).collect();
        // first duplicate pair: smallest i with kb[i] == kb[j], j < i
        // (the interpreter's pairwise insert loop finds it at pair i)
        let mut dup: Option<usize> = None;
        'outer: for i in 0..pairs {
            for j in 0..i {
                if kb[i] == kb[j] {
                    dup = Some(i);
                    break 'outer;
                }
            }
        }
        let obj = self.push_vreg();
        self.ctr_map_alloc(obj, CTR_MAP_CAP);
        self.asm.sd(obj, S0, self.map_scratch_cell(32));
        self.pop_vreg();
        let end = match dup {
            Some(di) => {
                // values only, pairwise IR order, through the duplicate
                // pair — then the runtime 84 (the shared CompoPackError
                // thunk), exactly where the pairwise path's found-check
                // would fire. The slot stays unbound: every execution of
                // this body exits here.
                for j in 0..=di {
                    self.emit_map_value_stage(&*arr.subs[j * 2 + 1], kind, Bytecode::PACKMAP)?;
                }
                let l84 = self.nil_thunk(84);
                self.asm.jal_(ZERO, l84);
                return Ok(());
            }
            None => pairs,
        };
        // sorted slot per pair: rank[j] = the position of kb[j] in the
        // byte-lexicographic order (keys are unique here — a pure,
        // deterministic function of the key set)
        let mut order: Vec<usize> = (0..end).collect();
        order.sort_by(|&a, &b| kb[a].cmp(&kb[b]));
        let mut rank = vec![0usize; end];
        for (pos, &i) in order.iter().enumerate() {
            rank[i] = pos;
        }
        let tag = Self::map_value_tag(kind);
        for j in 0..end {
            // pair j's value evaluates NOW (IR order), then lands in its
            // sorted slot — a0: obj→kptr, a1: entry address, sc1: words
            self.emit_map_value_stage(&*arr.subs[j * 2 + 1], kind, Bytecode::PACKMAP)?;
            let off = CTR_MAP_HDR as u64 + CTR_MAP_ENT as u64 * rank[j] as u64;
            let (a0, a1, sc1) = (self.a0, self.a1, self.asm.sc1);
            self.asm.ld(a0, S0, self.map_scratch_cell(32)); // obj
            if off <= 2047 {
                self.asm.addi(a1, a0, off as i32);
            } else {
                self.asm.li(a1, off);
                self.asm.add(a1, a0, a1);
            }
            let lbl = self.lit_label(&kb[j]);
            self.lit_addr(a0, lbl); // kptr = the pool bytes (obj consumed)
            self.asm.sd(a0, a1, 0);
            self.asm.addi(sc1, ZERO, kb[j].len() as i32);
            self.asm.sd(sc1, a1, 8);
            self.asm.addi(sc1, ZERO, tag as i32);
            self.asm.sd(sc1, a1, 16);
            let o2 = self.map_scratch_off;
            self.asm.ld(sc1, S0, o2 + self.map_v0_cell);
            self.asm.sd(sc1, a1, 24);
            self.asm.ld(sc1, S0, o2 + self.map_v1_cell);
            self.asm.sd(sc1, a1, 32);
        }
        // live length = the pair count, then the bare descriptor into
        // the slot cell (the pairwise tail, unchanged)
        let (a0, sc1) = (self.a0, self.asm.sc1);
        self.asm.ld(a0, S0, self.map_scratch_cell(32));
        self.asm.addi(sc1, ZERO, end as i32);
        self.asm.sd(sc1, a0, 0);
        let cell = self.slot_offset(slot_idx);
        self.asm.sd(a0, S0, cell);
        Ok(())
    }

    /// `PUT <list slot> := keys(m)/values(m)` — allocate a fresh arena list
    /// with the map's CURRENT length and copy every entry (sorted order) as
    /// a tagged cell: KEYS → (tag=8, kptr, klen); VALUES → the entry's own
    /// (vtag, va, vb). The result is a snapshot: later map mutations are
    /// not visible (the interpreter builds a new VecDeque).
    pub(crate) fn emit_map_to_list(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        self.map_zero_kill_all(); // B14: any other map operation ends the fact
        self.cat_top_kill(); // B9: the list allocation below bumps S1
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
    /// B14 (impl/arena-mapflow): when the receiver slot carries the
    /// len-zero fact (its object is this straight-line run's fresh
    /// `new_map()`), find on the empty table is (pos = 0, found = 0)
    /// WHATEVER the key — constant or dynamic; the scan loop runs zero
    /// iterations — so the whole lookup and BOTH insert faces collapse:
    ///   * 84 (duplicate): found == 0 always — the interpreter's
    ///     contains_key over zero entries is false;
    ///   * 40 (cap): new_len = 0 + 1 = 1 <= CTR_MAP_CAP — the
    ///     `len >= cap` test is structurally false;
    ///   * the shift loop of emit_map_insert_at: wpos == wlen == 0 —
    ///     nothing to shift;
    ///   * the len++ reload: the new length is the constant 1.
    /// The deleted faces sat strictly between the key's 73 guards (emitted
    /// above) and the entry store — no other fault face moves, and the
    /// deleted find is face-free (a pure internal scan).
    pub(crate) fn emit_map_insert_stmt(&mut self, n: &IRNodeTriple) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::INSERT)?;
        let kind = self.map_kind_of_slot(ms);
        let (kty, kbefore, kent) = self.emit_map_key_eval(&*n.suby, Bytecode::INSERT)?;
        self.emit_map_value_stage(&*n.subz, kind, Bytecode::INSERT)?;
        self.emit_map_key_finish(&*n.suby, Bytecode::INSERT, kty, kbefore, kent, false)?;
        let zero = self.map_zero.get(ms).copied().unwrap_or(false);
        self.map_zero_kill_all();
        if zero {
            let obj = self.push_vreg();
            self.ctr_map_load_obj(ms, obj);
            let (kptr, klen) = self.map_load_key_regs();
            let vb = self.push_vreg();
            let va = self.push_vreg();
            let o2 = self.map_scratch_off;
            self.asm.ld(va, S0, o2 + self.map_v0_cell);
            self.asm.ld(vb, S0, o2 + self.map_v1_cell);
            let tag = self.push_vreg();
            self.asm.li(tag, Self::map_value_tag(kind));
            // entry 0 sits at obj + HDR; len becomes the constant 1
            let (a0, sc1) = (self.a0, self.asm.sc1);
            self.asm.addi(a0, obj, CTR_MAP_HDR as i32);
            self.asm.sd(kptr, a0, 0);
            self.asm.sd(klen, a0, 8);
            self.asm.sd(tag, a0, 16);
            self.asm.sd(va, a0, 24);
            self.asm.sd(vb, a0, 32);
            self.asm.li(sc1, 1);
            self.asm.sd(sc1, obj, 0);
            for _ in 0..6 {
                self.pop_vreg(); // tag,va,vb,klen,kptr,obj
            }
            return Ok(());
        }
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
            //
            // B5 WITHDRAWN (audit F1 differential): hoisting the shared
            // value payload (ld va + ld vb + li tag) above the found
            // branch moved `found` BELOW the hoisted payload on the
            // vstack, so the new-key arm's `pop_vreg` (aimed at `found`)
            // actually freed the TAG slot — and the key reload below
            // reused that register, clobbering the value tag with the
            // staged key pointer. The inserted entry's vtag field then
            // held the kptr word, and any later read of the entry
            // (KEYS/VALUES/ITEMGET/serialize) misread its type. Keeping
            // the payload plus two fresh key slots live across the branch
            // also pushes past the 8-register pool once insert_at's own
            // word temp is counted, so the hoist is structurally out of
            // budget; each arm re-loads the payload from the scratch
            // cells (written before the find, constant across the
            // branch).
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
    /// B14 (impl/arena-mapflow): under the len-zero fact EVERY key is
    /// missing (found == 0 whatever the key — the find's scan runs zero
    /// iterations), so the 85 face is CERTAIN: the lookup and the whole
    /// store collapse to a jump to the shared 85 thunk. Face order is
    /// preserved — the value was evaluated above (its faces fired), the key
    /// 73 guards were emitted by finish, and the deleted find is face-free.
    pub(crate) fn emit_map_itemset_stmt(&mut self, n: &IRNodeTriple) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::ITEMSET)?;
        let kind = self.map_kind_of_slot(ms);
        let (kty, kbefore, kent) = self.emit_map_key_eval(&*n.suby, Bytecode::ITEMSET)?;
        self.emit_map_value_stage(&*n.subz, kind, Bytecode::ITEMSET)?;
        self.emit_map_key_finish(&*n.suby, Bytecode::ITEMSET, kty, kbefore, kent, true)?;
        let zero = self.map_zero.get(ms).copied().unwrap_or(false);
        self.map_zero_kill_all();
        if zero {
            for _ in 0..2 {
                self.pop_vreg(); // direct-mode (kptr, klen)
            }
            let l85 = self.nil_thunk(85);
            self.asm.jal_(ZERO, l85);
            return Ok(());
        }
        // vstack: kptr, klen (direct mode)
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        let klen = loc_reg(self.vstack[self.vstack.len() - 4]);
        let kptr = loc_reg(self.vstack[self.vstack.len() - 5]);
        self.emit_map_find_direct(obj, pos, found, kptr, klen);
        let l85 = self.nil_thunk(85);
        self.asm.beq(found, ZERO, l85);
        self.pop_vreg(); // found
        let vb = self.push_vreg();
        let va = self.push_vreg();
        let o2 = self.map_scratch_off;
        self.asm.ld(va, S0, o2 + self.map_v0_cell);
        self.asm.ld(vb, S0, o2 + self.map_v1_cell);
        let tag = self.push_vreg();
        self.asm.li(tag, Self::map_value_tag(kind));
        self.emit_map_store_entry(obj, pos, tag, va, vb);
        for _ in 0..7 {
            self.pop_vreg(); // tag,va,vb,pos,obj,klen,kptr
        }
        Ok(())
    }

    /// The map REMOVE statement: key 73, missing → 85, shift-left removal.
    pub(crate) fn emit_map_remove_stmt(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::REMOVE)?;
        self.map_zero_kill_all(); // B14: the mutation ends any len-zero fact
        self.emit_map_key(&*n.suby, Bytecode::REMOVE)?;
        // vstack: kptr, klen (direct mode)
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        let klen = loc_reg(self.vstack[self.vstack.len() - 4]);
        let kptr = loc_reg(self.vstack[self.vstack.len() - 5]);
        self.emit_map_find_direct(obj, pos, found, kptr, klen);
        let l85 = self.nil_thunk(85);
        self.asm.beq(found, ZERO, l85);
        self.pop_vreg(); // found
        self.emit_map_remove_at(obj, pos);
        self.pop_vreg(); // pos
        self.pop_vreg(); // obj
        self.pop_vreg(); // klen
        self.pop_vreg(); // kptr
        Ok(())
    }

    /// HASKEY(map): key faces then a 0/1 result from the sorted lookup.
    pub(crate) fn emit_map_haskey(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::HASKEY)?;
        self.map_zero_kill_all(); // B14: any other map operation ends the fact
        self.emit_map_key(&*n.suby, Bytecode::HASKEY)?;
        // vstack: kptr, klen (direct mode)
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        let klen = loc_reg(self.vstack[self.vstack.len() - 4]);
        let kptr = loc_reg(self.vstack[self.vstack.len() - 5]);
        self.emit_map_find_direct(obj, pos, found, kptr, klen);
        // save found's register, drop the five, re-push the 0/1 result
        let found_r = loc_reg(self.vstack[self.vstack.len() - 1]);
        self.pop_vreg(); // found
        self.pop_vreg(); // pos
        self.pop_vreg(); // obj
        self.pop_vreg(); // klen
        self.pop_vreg(); // kptr
        let out = self.push_vreg();
        self.asm.mv(out, found_r);
        Ok(())
    }

    /// ITEMGET(map): key faces, missing → 85, then the tagged value load.
    pub(crate) fn emit_map_itemget(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ms = map_receiver_slot(&*n.subx, self.slot, Bytecode::ITEMGET)?;
        self.map_zero_kill_all(); // B14: any other map operation ends the fact
        let kind = self.map_kind_of_slot(ms);
        self.emit_map_key(&*n.suby, Bytecode::ITEMGET)?;
        // vstack: kptr, klen (direct mode)
        let obj = self.push_vreg();
        self.ctr_map_load_obj(ms, obj);
        let pos = self.push_vreg();
        let found = self.push_vreg();
        let klen = loc_reg(self.vstack[self.vstack.len() - 4]);
        let kptr = loc_reg(self.vstack[self.vstack.len() - 5]);
        self.emit_map_find_direct(obj, pos, found, kptr, klen);
        let l85 = self.nil_thunk(85);
        self.asm.beq(found, ZERO, l85);
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
        // the result entries occupy the top; pop them, drop the five
        // bookkeeping entries (found,pos,obj,klen,kptr), then re-push the
        // result from its saved registers (a re-push hands out different
        // pool slots than the pops freed — the mv guards below cover it;
        // pop/push are pure bookkeeping, the saved registers keep their
        // values because nothing emits in between).
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
        self.pop_vreg(); // klen
        self.pop_vreg(); // kptr
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
        self.map_zero_kill_all(); // B14: any other map operation ends the fact
        let a0 = self.a0;
        self.ctr_map_load_obj(ms, a0);
        self.asm.ld(a0, a0, 0);
        let out = self.push_vreg();
        self.asm.mv(out, a0);
        Ok(())
    }

    /// B7(a): the MERGE duplicate scan as a two-pointer merge sweep. Both
    /// entry arrays are byte-lex sorted (a's by the sorted INSERT/find
    /// discipline, b's likewise — the canonical BTreeMap order), so the
    /// interpreter's per-entry `contains_key` probe (compo.rs:501-507) is
    /// replaceable by one merge sweep: advance the cursor holding the
    /// SMALLER key; an equal pair is a duplicate → 84. O(lenA·lenB) worst
    /// case → O(lenA+lenB), and the per-entry find's staging round trip
    /// disappears. Fault faces: 84-before-40 is preserved (the sweep
    /// completes before the cap check, matching the interpreter's
    /// two-phase shape); the per-entry `check_scalar` is structurally
    /// scalar in the kernel subset and cannot fault, so its interleaving
    /// is unobservable. The sweep never mutates and never faults before
    /// the 84 thunk.
    ///
    /// Registers: six pool cursors (ea/enda/eb/endb/kla/klb — the vstack
    /// is empty at this statement site) plus the mode's scratch trio
    /// (a0 = byte index m, a1/sc1 = the two compared bytes); asm.sc2 stays
    /// untouched, so the sweep is mode-uniform. `end*` are one-past-the-
    /// last-entry sentinels (obj + 24 + len*40); an empty side ends the
    /// sweep immediately (no duplicate is possible).
    fn emit_map_dup_scan(&mut self) {
        let a0 = self.a0;
        let a1 = self.a1;
        let sc1 = self.asm.sc1;
        let ea = self.push_vreg();
        let enda = self.push_vreg();
        let eb = self.push_vreg();
        let endb = self.push_vreg();
        let kla = self.push_vreg();
        let klb = self.push_vreg();
        // side a: ea = objA + HDR, endA = ea + lenA*ENT
        self.asm.ld(a1, S0, self.map_scratch_cell(32)); // objA
        self.asm.addi(ea, a1, CTR_MAP_HDR as i32);
        self.asm.ld(sc1, a1, 0); // lenA
        self.asm.slli_(a0, sc1, 5);
        self.asm.slli_(sc1, sc1, 3);
        self.asm.add(a0, a0, sc1); // lenA*40
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(enda, ea, a0);
        // side b: eb = objB + HDR, endB = eb + lenB*ENT
        self.asm.ld(a1, S0, self.map_scratch_cell(40)); // objB
        self.asm.addi(eb, a1, CTR_MAP_HDR as i32);
        self.asm.ld(sc1, S0, self.map_scratch_cell(48)); // lenB
        self.asm.slli_(a0, sc1, 5);
        self.asm.slli_(sc1, sc1, 3);
        self.asm.add(a0, a0, sc1); // lenB*40
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(endb, eb, a0);
        self.asm.ld(kla, ea, 8);
        self.asm.ld(klb, eb, 8);
        let l_scan = self.asm.new_label();
        let l_bcmp = self.asm.new_label();
        let l_bnext = self.asm.new_label();
        let l_aend = self.asm.new_label();
        let l_lt = self.asm.new_label();
        let l_gt = self.asm.new_label();
        let l_done = self.asm.new_label();
        let l84 = self.nil_thunk(84);
        self.place(l_scan);
        self.asm.bgeu(ea, enda, l_done);
        self.asm.bgeu(eb, endb, l_done);
        self.asm.addi(a0, ZERO, 0); // m
        self.place(l_bcmp);
        // m >= klen_a: side a's key ended. The klb check below only runs
        // while m < kla, so reaching here means m == kla <= klb: equal
        // lengths → equal keys → 84; a proper prefix → a < b (fall into
        // the advance-a arm). m >= klen_b fires first when klen_b is the
        // smaller: b's key is the prefix → a > b.
        self.asm.bgeu(a0, kla, l_aend);
        self.asm.bgeu(a0, klb, l_gt);
        self.asm.ld(a1, ea, 0); // kptr_a
        self.asm.add(a1, a1, a0);
        self.asm.lbu(a1, a1, 0); // a[m]
        self.asm.ld(sc1, eb, 0); // kptr_b
        self.asm.add(sc1, sc1, a0);
        self.asm.lbu(sc1, sc1, 0); // b[m]
        self.asm.beq(a1, sc1, l_bnext);
        self.asm.bltu(a1, sc1, l_lt); // a's byte smaller → a < b
        self.asm.jal_(ZERO, l_gt); // a's byte larger → a > b
        self.place(l_bnext);
        self.asm.addi(a0, a0, 1);
        self.asm.jal_(ZERO, l_bcmp);
        self.place(l_aend);
        self.asm.beq(kla, klb, l84);
        self.place(l_lt);
        self.asm.addi(ea, ea, CTR_MAP_ENT as i32);
        self.asm.ld(kla, ea, 8);
        self.asm.jal_(ZERO, l_scan);
        self.place(l_gt);
        self.asm.addi(eb, eb, CTR_MAP_ENT as i32);
        self.asm.ld(klb, eb, 8);
        self.asm.jal_(ZERO, l_scan);
        self.place(l_done);
        for _ in 0..6 {
            self.pop_vreg();
        }
    }

    /// MERGE(a, b): self-merge → 80, then the B7(a) two-pointer duplicate
    /// sweep (84), then the combined-length cap → 40, then the entries are
    /// inserted (a keeps its sorted order via the sorted insert).
    pub(crate) fn emit_map_merge(&mut self, n: &IRNodeDouble) -> Ret<()> {
        let ma = map_receiver_slot(&*n.subx, self.slot, Bytecode::MERGE)?;
        let mb = map_receiver_slot(&*n.suby, self.slot, Bytecode::MERGE)?;
        self.map_zero_kill_all(); // B14: the mutation ends any len-zero fact
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
        // B7(a): duplicate scan as a two-pointer merge over the two sorted
        // entry arrays (84 fires before the 40 cap — the scan completes
        // first, exactly like the interpreter's compo.rs:501-517 shape)
        self.emit_map_dup_scan();
        // combined length cap → 40: the interpreter faults when
        // new_len > compo_length (compo.rs:512 — 128 exactly is ADMITTED),
        // so the branch bound is CAP+1 (the list merge's list.rs:920
        // precedent); the historical `bgeu(sc1, 128)` fired at == 128 and
        // diverged on the exactly-full merge
        let l40 = self.nil_thunk(40);
        let sc1 = self.asm.sc1;
        self.asm.ld(a0, S0, self.map_scratch_cell(32));
        self.asm.ld(a0, a0, 0); // lenA
        self.asm.ld(sc1, S0, self.map_scratch_cell(48)); // lenB
        self.asm.add(sc1, sc1, a0);
        self.asm.li(a0, CTR_MAP_CAP + 1);
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
