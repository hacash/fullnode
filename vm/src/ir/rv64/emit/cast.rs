use super::super::*;

impl<'a> Lower<'a> {

    /* ---- T13: CTO relaxed casts + the address value model ---- */

    /// Whether a bytes literal decodes as an address — exactly the
    /// interpreter's `address_from_bytes`: length 21 AND a version byte
    /// the current protocol supports (`Address::decode`).
    pub(crate) fn literal_is_valid_address(b: &[u8]) -> bool {
        use field::Decode;
        b.len() == field::Address::SIZE && field::Address::decode(b).is_ok()
    }

    /// Top of stack: bytes descriptor (ptr, len) → truthiness int (0/1).
    /// The interpreter's `extract_bool` on Bytes/Address: any non-zero
    /// byte is true. Operands are read-only (bank refs pass through).
    pub(crate) fn emit_bytes_truthy(&mut self) -> Ret<()> {
        let rl = self.pop_reg();
        let rp = self.pop_reg();
        let acc = self.asm.sc1;
        self.asm.addi(acc, ZERO, 0);
        self.asm.addi(self.a0, ZERO, 0); // i
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.bgeu(self.a0, rl, l_done);
        self.asm.add(self.a1, rp, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.or_(acc, acc, self.a1);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        let d = self.push_vreg();
        // d = (0 < acc) — the 0/1 normalization
        self.asm.r(0, acc, ZERO, 3, d, 0b0110011); // sltu d = (0 < acc)
        Ok(())
    }

    /// Top of stack: bytes descriptor (ptr, len) stays — the CTO(Address)
    /// runtime face. `cast_addr` → `cast_bytes` (the may-Nil 71 guard is
    /// the caller's) → `address_from_bytes`: len must be exactly 21 and
    /// the version byte decodable, else CastFail(71).
    pub(crate) fn emit_address_checks(&mut self) -> Ret<()> {
        let n = self
            .vstack
            .len()
            .checked_sub(2)
            .ok_or("rv64 lower: `as address` on a short stack")?;
        let rl = loc_reg(self.vstack[n + 1]);
        let rp = loc_reg(self.vstack[n]);
        self.asm.addi(self.a0, ZERO, field::Address::SIZE as i32);
        self.asm.bne(rl, self.a0, self.fault_cast);
        let sc = self.asm.sc1;
        self.asm.lbu(sc, rp, 0);
        // version ∈ {VERSION_PRIVAKEY 0, VERSION_CONTRACT 1,
        // VERSION_SCRIPTMH 5} — `Address::decode`'s is_supported
        let ok = self.asm.new_label();
        self.asm.beq(sc, ZERO, ok);
        self.asm.addi(self.a0, ZERO, 1);
        self.asm.beq(sc, self.a0, ok);
        self.asm.addi(self.a0, ZERO, 5);
        self.asm.beq(sc, self.a0, ok);
        self.asm.jal_(ZERO, self.fault_cast);
        self.place(ok);
        Ok(())
    }

    /// Top of stack: bytes descriptor (ptr, len) → BE uint, leading zero
    /// bytes trimmed, the non-zero remainder must fit `nb` bytes else
    /// CastFail(71) — the interpreter's `fit_be_bytes` (the exact face of
    /// `bytes_to_uint_width` / `to_u128`+range-check on Bytes/Address).
    pub(crate) fn emit_bytes_to_uint(&mut self, nb: u64) -> Ret<()> {
        let rl = self.pop_reg();
        let rp = self.pop_reg();
        let sc = self.asm.sc1;
        self.asm.addi(self.a0, ZERO, 0); // i
        let l_skip = self.asm.new_label();
        let l_chk = self.asm.new_label();
        let l_acc = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_skip);
        self.asm.bgeu(self.a0, rl, l_chk);
        self.asm.add(self.a1, rp, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.bne(self.a1, ZERO, l_chk);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_skip);
        self.place(l_chk);
        // remaining = len - i > nb → 71
        self.asm.sub(self.a1, rl, self.a0);
        self.asm.li(sc, nb);
        self.asm.bltu(sc, self.a1, self.fault_cast);
        // d = BE accumulate over [i, len)
        self.asm.addi(sc, ZERO, 0);
        self.place(l_acc);
        self.asm.bgeu(self.a0, rl, l_done);
        self.asm.add(self.a1, rp, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.slli_(sc, sc, 8);
        self.asm.or_(sc, sc, self.a1);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_acc);
        self.place(l_done);
        let d = self.push_vreg();
        if d != sc {
            self.asm.mv(d, sc);
        }
        Ok(())
    }

    /// The u128 flavor of [`Self::emit_bytes_to_uint`]: the (lo, hi) pair
    /// domain, ≤ 16 non-zero-prefix bytes. A register-shift accumulate
    /// would need four live scratches (counter, byte, lo, hi) — ImcRemap
    /// gives only three (a0, a1==sc2, sc1) — so the trimmed bytes are
    /// copied into a 16-byte ARENA scratch holding the value
    /// LITTLE-ENDIAN (trimmed byte j lands at scratch[remaining-1-j];
    /// `ld` is little-endian, so the two loads ARE the pair halves —
    /// `u128::from_be_bytes` on the interpreter side). S1 is never
    /// bumped: the scratch is unallocated, transient space.
    pub(crate) fn emit_bytes_to_uint128(&mut self) -> Ret<()> {
        let rl = self.pop_reg();
        let rp = self.pop_reg();
        self.asm.addi(self.a0, ZERO, 0); // i
        let l_skip = self.asm.new_label();
        let l_chk = self.asm.new_label();
        let l_copy = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_skip);
        self.asm.bgeu(self.a0, rl, l_chk);
        self.asm.add(self.a1, rp, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.bne(self.a1, ZERO, l_chk);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_skip);
        self.place(l_chk);
        // remaining = len - i > 16 → CastFail(71)
        self.asm.sub(self.a1, rl, self.a0);
        self.asm.addi(self.asm.sc1, ZERO, 16);
        self.asm.bltu(self.asm.sc1, self.a1, self.fault_cast);
        // arena room for the 16-byte scratch (the emit_cbytes128 face);
        // a0 still holds the first non-zero index
        self.asm.li(self.a1, self.arena_end);
        self.asm.addi(self.asm.sc1, S1, 16);
        self.asm.bgeu(self.asm.sc1, self.a1, self.fault_cast);
        // zero the scratch; dst walks DOWN from S1 + remaining - 1
        self.asm.sd(ZERO, S1, 0);
        self.asm.sd(ZERO, S1, 8);
        self.asm.sub(self.a1, rl, self.a0); // remaining (again — a1 held arena_end)
        self.asm.addi(self.asm.sc1, self.a1, -1);
        self.asm.add(self.asm.sc1, self.asm.sc1, S1); // dst = S1 + remaining - 1
        self.asm.add(self.a0, rp, self.a0); // src = rp + i
        self.place(l_copy);
        // a1 doubles as the S1-1 bound (consumed) then the byte
        self.asm.addi(self.a1, S1, -1);
        self.asm.beq(self.asm.sc1, self.a1, l_done);
        self.asm.lbu(self.a1, self.a0, 0);
        self.asm.sb(self.a1, self.asm.sc1, 0);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.addi(self.asm.sc1, self.asm.sc1, -1);
        self.asm.jal_(ZERO, l_copy);
        self.place(l_done);
        let dlo = self.push_vreg();
        self.asm.ld(dlo, S1, 0);
        let dhi = self.push_vreg();
        self.asm.ld(dhi, S1, 8);
        Ok(())
    }

    /// Top of stack: two bytes/address descriptors → EQ/NEQ by raw bytes
    /// (`value_content_eq`). An Address against a Bytes whose length is
    /// not 21 faults Arithmetic(90) (the type-mismatch fall-through of
    /// the shared value domain); differing lengths are simply unequal.
    /// Nil tags: exactly-one-Nil faults 90, two Nils compare equal (the
    /// caller emits those guards while both operands are live).
    pub(crate) fn emit_bytes_eq(&mut self, op: Bytecode, addr_l: bool, addr_r: bool) -> Ret<()> {
        let bl = self.pop_reg();
        let bp = self.pop_reg();
        let al = self.pop_reg();
        let ap = self.pop_reg();
        let sc = self.asm.sc1;
        if addr_l && !addr_r {
            self.asm.addi(sc, ZERO, field::Address::SIZE as i32);
            self.asm.bne(bl, sc, self.fault_arith);
        }
        if addr_r && !addr_l {
            self.asm.addi(sc, ZERO, field::Address::SIZE as i32);
            self.asm.bne(al, sc, self.fault_arith);
        }
        let l_cmp = self.asm.new_label();
        let l_diff = self.asm.new_label();
        let l_eq = self.asm.new_label();
        let l_tail = self.asm.new_label();
        self.asm.bne(al, bl, l_diff);
        // equal lengths: byte-compare; the first differing pair exits
        // early to l_diff (a non-zero indicator). Per iteration only three
        // scratches are live — i (a0), byte A (a1), byte B (sc) — an
        // accumulate (acc ^= A ^ B) would need a fourth AND cancels on
        // even repeat counts (0x11^0x44 twenty times folds to 0).
        self.asm.addi(self.a0, ZERO, 0); // i
        self.place(l_cmp);
        self.asm.bgeu(self.a0, al, l_eq);
        self.asm.add(self.a1, ap, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.add(sc, bp, self.a0);
        self.asm.lbu(sc, sc, 0);
        self.asm.bne(self.a1, sc, l_diff);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_cmp);
        // every pair compared equal: the indicator is zero
        self.place(l_eq);
        self.asm.addi(sc, ZERO, 0);
        self.asm.jal_(ZERO, l_tail);
        // a differing byte pair — or differing lengths (Bytes vs Bytes),
        // which are simply unequal — a non-zero difference indicator
        self.place(l_diff);
        self.asm.addi(sc, ZERO, -1);
        self.place(l_tail);
        let d = self.push_vreg();
        if op == Bytecode::EQ {
            // d = (sc == 0)
            self.asm.addi(self.a0, ZERO, 1);
            self.asm.r(0, self.a0, sc, 3, d, 0b0110011); // sltu d, sc, 1
        } else {
            // d = (sc != 0)
            self.asm.r(0, sc, ZERO, 3, d, 0b0110011); // sltu d, zero, sc
        }
        Ok(())
    }

    pub(crate) fn emit_cbytes(&mut self, w: u32) -> Ret<()> {
        debug_assert!(matches!(w, 1 | 8 | 16 | 32 | 64));
        let nb = if w == 1 { 1 } else { (w / 8) as i32 }; // 1 | 2 | 4 | 8
        let rp = self.pop_own_replace(); // holds the value, becomes the ptr
        let rl = self.push_vreg();
        self.asm.addi(rl, ZERO, nb);
        // arena room for nb bytes
        self.asm.li(self.a0, self.arena_end);
        self.asm.addi(self.a1, S1, nb);
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

    /// narrow-PUT face: the dynamic-width flavor of [`Self::emit_cbytes`] —
    /// the byte count comes from the slot's runtime width shadow (the
    /// interpreter's cast_bytes emits the CURRENT value's own width; a slot
    /// assigned mixed static widths can view at any of them). Shadow values
    /// are 1/2/4/8 (a flagged slot's join is sub-128), so the loop is the
    /// same shape with a register bound.
    pub(crate) fn emit_cbytes_dyn(&mut self, idx: u8) -> Ret<()> {
        let off = self.dyn_w_off[idx as usize];
        let rp = self.pop_own_replace(); // holds the value, becomes the ptr
        let rl = self.push_vreg();
        self.asm.lbu(rl, S0, off); // nb
        // arena room for nb bytes (runtime count)
        self.asm.li(self.a0, self.arena_end);
        self.asm.add(self.a1, S1, rl);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast);
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
}
