use super::super::*;

impl<'a> Lower<'a> {

    /* ---- arena / bytes helpers ---- */

    /// `for i in 0..len: *S1++ = src[i]` — clobbers the scratch pair, bumps S1.
    pub(crate) fn emit_memcpy(&mut self, src: u8, len: u8) {
        self.asm.addi(self.a0, ZERO, 0);
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.bgeu(self.a0, len, l_done);
        self.asm.add(self.a1, src, self.a0);
        self.asm.lbu(self.a1, self.a1, 0);
        self.asm.sb(self.a1, S1, 0);
        self.asm.addi(S1, S1, 1);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
    }

    /// Arena-exhaustion check for `S1 + len_reg` against the declared end.
    pub(crate) fn emit_arena_check(&mut self, len_reg: u8) {
        self.asm.li(self.a0, self.arena_end);
        self.asm.add(self.a1, S1, len_reg);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast); // arena exhausted
    }

    /// Top of stack: `a ++ b` → one bytes descriptor.
    pub(crate) fn emit_cat(&mut self) -> Ret<()> {
        // operands are read-only (memcpy sources, lengths); the two result
        // slots are fresh pool registers.
        let lb = self.pop_reg();
        let pb = self.pop_reg();
        let la = self.pop_reg();
        let pa = self.pop_reg();
        // total = la + lb (checked)
        self.asm.r(0, lb, la, 0, self.a1, 0b0110011); // add a1 = la + lb
        let ok = self.asm.new_label();
        self.asm.bgeu(self.a1, la, ok);
        self.trap(EXIT_ARITH);
        self.place(ok);
        self.emit_arena_check(self.a1);
        // copies bump S1 by la then lb
        self.emit_memcpy(pa, la);
        self.emit_memcpy(pb, lb);
        let dstp = self.push_vreg();
        let dstl = self.push_vreg();
        self.asm.r(0, lb, la, 0, dstl, 0b0110011); // add dstl, la + lb
        self.asm.r(0x20, dstl, S1, 0, dstp, 0b0110011); // sub dstp, S1 - dstl
        Ok(())
    }

    /// Top of stack: bytes, idx → u8 at idx (traps out of range).
    pub(crate) fn emit_byte_at(&mut self) -> Ret<()> {
        let idx = self.pop_reg();
        let rl = self.pop_reg();
        let lp = self.pop_vreg();
        self.asm.bgeu(idx, rl, self.fault_cast);
        let rp = loc_reg(lp);
        let d = self.push_vreg();
        self.asm.add(d, rp, idx);
        self.asm.lbu(d, d, 0);
        Ok(())
    }

    /* ---- task 7: heap read/write over the frame-reserved block ---- */

    /// HeapError(44) thunk (lazy — heap-free bodies never emit it).
    pub(crate) fn heap_fault(&mut self) -> usize {
        self.nil_thunk(44)
    }

    /// The current heap length into `dst`: `grown_flag × heap_bytes`. The
    /// interpreter's length is `datas.len()`: 0 until a grow executes, the
    /// grown byte count after — the admitted single HGROW runs at most once
    /// per path, so the prologue-zeroed flag is monotone 0 → 1. Bodies
    /// without a reservation have the constant length 0 (every non-empty
    /// access traps 44, `read(0, 0)` returns empty — the interpreter's own
    /// length-0 faces).
    pub(crate) fn heap_len_reg(&mut self, dst: u8) {
        if self.heap_bytes == 0 {
            self.li_k(dst, 0);
            return;
        }
        self.li_k(dst, self.heap_bytes as u64);
        debug_assert!(dst != self.asm.sc1, "the flag load would clobber dst");
        let sc = self.asm.sc1;
        self.asm.ld(sc, S0, self.heap_flag_off);
        // mul dst = dst * flag (flag ∈ {0, 1}: only the grow arm writes it)
        // — f7 = 0000001 (MUL; f7 = 0 is ADD, the emit_cat add form)
        self.asm.r(1, sc, dst, 0, dst, 0b0110011);
    }

    /// Bounds face of every heap access: trap HeapError(44) when
    /// `sum_reg > length` (the interpreter's `checked_right`:
    /// `start + len > datas.len()` → 44; `==` is IN bounds, and a zero sum
    /// passes even on the empty heap). `sum` must be overflow-free: the
    /// dynamic start is u32-checked and the dynamic length u16-checked
    /// before this runs; the immediate forms fit the 2040B window.
    pub(crate) fn heap_bounds(&mut self, sum: u8) {
        let l44 = self.heap_fault();
        // a1 (not sc1 — heap_len_reg loads the flag through sc1; sc2 may
        // carry the caller's sum)
        let l = self.a1;
        self.heap_len_reg(l);
        self.asm.bltu(l, sum, l44);
    }

    /// Load `len` big-endian bytes at `S0 + off` into `dst` (the
    /// interpreter's BE scalar materialization). Callers guarantee the cell
    /// is in bounds, so `off + len` sits inside the 2040B frame window.
    pub(crate) fn heap_load_be(&mut self, dst: u8, off: i32, len: usize) {
        self.asm.lbu(dst, S0, off);
        let sc = self.asm.sc2;
        for i in 1..len {
            self.asm.slli_(dst, dst, 8);
            self.asm.lbu(sc, S0, off + i as i32);
            self.asm.or_(dst, dst, sc);
        }
    }

    /// Store the `len`-byte big-endian form of the u64 in `val` at
    /// `S0 + off` (the interpreter's `extract_bytes` scalar: u8 → 1 B …
    /// u64 → 8 B, a bool's canonical 0/1 byte).
    pub(crate) fn heap_store_be_u64(&mut self, off: i32, val: u8, len: usize) {
        let sc = self.asm.sc2;
        for i in 0..len {
            let sh = 8 * (len - 1 - i);
            if sh > 0 {
                self.asm.srli_(sc, val, sh as u32);
            } else {
                self.asm.mv(sc, val);
            }
            self.asm.sb(sc, S0, off + i as i32);
        }
    }

    /// Copy `len_reg` bytes from `src` to the address held in `a0` (the
    /// caller sets the destination cursor first). The loop needs FOUR live
    /// registers — dst cursor (a0), src cursor (sc2), byte (sc1), counter —
    /// and under ImcRemap a1 IS sc2, so the counter takes a fresh pool
    /// register pushed and popped inside this helper.
    pub(crate) fn heap_copy_bytes(&mut self, src: u8, len_reg: u8) {
        let cnt = self.push_vreg();
        self.asm.mv(cnt, len_reg);
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        self.asm.mv(sc2, src);
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(cnt, ZERO, l_done);
        self.asm.lbu(sc1, sc2, 0);
        self.asm.sb(sc1, self.a0, 0);
        self.asm.addi(sc2, sc2, 1);
        self.asm.addi(self.a0, self.a0, 1);
        self.asm.addi(cnt, cnt, -1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        self.pop_vreg();
    }

    /// Shared tail of the heap write faces: the value (top of stack) and —
    /// for the dynamic form — the offset below it are both evaluated. Traps
    /// in the interpreter's order: offset extract_u32 → 72 (`mev_off` Nil
    /// tag), value extract_bytes → 44 (Nil), bounds → 44. `off_dyn`: the
    /// dynamic HWRITE offset (its 72 faces run here); `off_imm`: the write
    /// start as a constant (the HWRITEX/HWRITEXL immediate forms — no 72
    /// offset faces).
    pub(crate) fn emit_heap_write(
        &mut self,
        val: &dyn IRNode,
        mev_off: bool,
        off_dyn: bool,
        off_imm: Option<usize>,
    ) -> Ret<()> {
        let vd = self.vstack.len() - 1;
        let mev_val = self.nil_val(val);
        let oty = node_res_ty(val, self.slot)?;
        // the offset's vstack depth (dynamic form only): below the value's
        // own entries (2 for a bytes descriptor or a u128 pair, 1 for the
        // other uint widths); the immediate forms carry no offset entry
        let od = if off_dyn {
            let vent = match oty {
                Ty::Bytes | Ty::Address => 2,
                Ty::Int => self.int_width(val).map(|w| if w == 128 { 2 } else { 1 }).unwrap_or(1),
                _ => 1,
            };
            Some(self.vstack.len() - 1 - vent)
        } else {
            None
        };
        let l72 = self.nil_thunk(72);
        if let Some(od) = od {
            if mev_off {
                self.nil_guard_tag(od, 72);
            }
            let off_r = loc_reg(self.vstack[od]);
            let sc = self.asm.sc1;
            self.li_k(sc, 0xFFFF_FFFF);
            self.asm.bltu(sc, off_r, l72);
        }
        match oty {
            Ty::Int => {
                let Some(w) = self.int_width(val) else {
                    return errf!(
                        "rv64 lower: opcode HWRITE is outside the translatable kernel subset \
                         (the written value's width is not static — the interpreter extracts \
                         its canonical bytes at the value's own width, a bool mixed with ints \
                         has none)"
                    );
                };
                // extract_bytes(Nil) → the interpreter's do_write maps the
                // cast failure onto HeapError(44) — NOT the 71 cast face
                if mev_val {
                    self.nil_guard_tag(vd, 44);
                }
                // task 3: a u128 value is a (lo, hi) pair — extract_bytes
                // is the 16-byte big-endian form (hi first, then lo)
                let is_u128 = w == 128;
                let val_r = loc_reg(self.vstack[vd]);
                let val_lo = if is_u128 { loc_reg(self.vstack[vd - 1]) } else { ZERO };
                let len = if w == 1 {
                    1usize
                } else if is_u128 {
                    16
                } else {
                    (w / 8) as usize
                };
                if off_dyn {
                    let off_r = loc_reg(self.vstack[od.unwrap()]);
                    // bounds: off + len ≤ current length (sum through a0 —
                    // distinct from a1/sc1/sc2 in every mode)
                    self.li_k(self.a0, len as u64);
                    self.asm.add(self.a0, self.a0, off_r);
                    self.heap_bounds(self.a0);
                    self.asm.addi(self.a0, S0, self.heap_off);
                    self.asm.add(self.a0, self.a0, off_r);
                    if is_u128 {
                        self.heap_store_be_u64_reg(self.a0, val_r, 8);
                        self.heap_store_be_u64_reg_at(self.a0, val_lo, 8, 8);
                    } else {
                        self.heap_store_be_u64_reg(self.a0, val_r, len);
                    }
                } else {
                    let ost = off_imm.expect("heap write needs an offset");
                    if ost + len > self.heap_bytes as usize {
                        // every reachable execution faults (L ≤ the
                        // reservation); the shortcut also keeps the baked
                        // sb offsets inside the 12-bit window
                        self.trap(44);
                        self.pop_vreg(); // value (hi)
                        if is_u128 {
                            self.pop_vreg(); // value lo
                        }
                        return Ok(());
                    }
                    // bounds: the store's S0-relative offsets stay inside
                    // the window on every reachable path
                    self.li_k(self.a0, (ost + len) as u64);
                    self.heap_bounds(self.a0);
                    if is_u128 {
                        self.heap_store_be_u64(self.heap_off + ost as i32, val_r, 8);
                        self.heap_store_be_u64(self.heap_off + ost as i32 + 8, val_lo, 8);
                    } else {
                        self.heap_store_be_u64(self.heap_off + ost as i32, val_r, len);
                    }
                }
                self.pop_vreg(); // value (hi)
                if is_u128 {
                    self.pop_vreg(); // value lo
                }
                if off_dyn {
                    self.pop_vreg(); // dynamic offset
                }
                Ok(())
            }
            Ty::Bytes | Ty::Address => {
                if mev_val {
                    self.nil_guard_tag(vd, 44);
                }
                let len_r = loc_reg(self.vstack[vd]);
                let ptr_r = loc_reg(self.vstack[vd - 1]);
                if off_dyn {
                    let off_r = loc_reg(self.vstack[od.unwrap()]);
                    self.asm.add(self.a0, off_r, len_r);
                    self.heap_bounds(self.a0);
                    self.asm.addi(self.a0, S0, self.heap_off);
                    self.asm.add(self.a0, self.a0, off_r);
                    self.heap_copy_bytes(ptr_r, len_r);
                } else {
                    let ost = off_imm.expect("heap write needs an offset");
                    if ost > self.heap_bytes as usize {
                        // every reachable execution faults: L ≤ the
                        // reservation < ost (the copy never runs; a zero-
                        // length value at ost > reservation faults too)
                        self.trap(44);
                        self.pop_vreg(); // value len
                        self.pop_vreg(); // value ptr
                        return Ok(());
                    }
                    self.li_k(self.a0, ost as u64);
                    self.asm.add(self.a0, self.a0, len_r);
                    self.heap_bounds(self.a0);
                    self.asm.addi(self.a0, S0, self.heap_off);
                    self.heap_copy_bytes(ptr_r, len_r);
                }
                self.pop_vreg(); // value len
                self.pop_vreg(); // value ptr
                if off_dyn {
                    self.pop_vreg(); // dynamic offset
                }
                Ok(())
            }
            other => errf!(
                "rv64 lower: opcode HWRITE is outside the translatable kernel subset \
                 (the written value must be bytes or a uint scalar — the interpreter \
                 extracts its canonical bytes; {:?} values are not in the kernel subset)",
                other
            ),
        }
    }

    /// `heap_store_be_u64` into a register-held address (`dst` = base);
    /// `at` skips bytes already emitted (the u128 pair's low half).
    pub(crate) fn heap_store_be_u64_reg(&mut self, dst: u8, val: u8, len: usize) {
        self.heap_store_be_u64_reg_at(dst, val, len, 0)
    }

    pub(crate) fn heap_store_be_u64_reg_at(&mut self, dst: u8, val: u8, len: usize, at: usize) {
        let sc = self.asm.sc2;
        for i in 0..len {
            let sh = 8 * (len - 1 - i);
            if sh > 0 {
                self.asm.srli_(sc, val, sh as u32);
            } else {
                self.asm.mv(sc, val);
            }
            self.asm.sb(sc, dst, (at + i) as i32);
        }
    }

    /* ---- P3-IDIOMS: fused byte-assembly groups (doc 94) ---- */

    /// Wide load at ptr + off (imm when it fits the 12-bit I-immediate,
    /// otherwise a scratch-materialized address). f3: 3=ld, 6=lwu, 1=lhu.
    pub(crate) fn emit_wide_at(&mut self, d: u8, rp: u8, off: u64, f3: u8) {
        if off <= 2047 {
            match f3 {
                3 => self.asm.ld(d, rp, off as i32),
                6 => self.asm.lwu(d, rp, off as i32),
                _ => self.asm.lhu(d, rp, off as i32),
            }
        } else {
            let sc1 = self.asm.sc1;
            self.li_k(sc1, off);
            self.asm.add(sc1, rp, sc1);
            match f3 {
                3 => self.asm.ld(d, sc1, 0),
                6 => self.asm.lwu(d, sc1, 0),
                _ => self.asm.lhu(d, sc1, 0),
            }
        }
    }

    /// LE group: one span guard (lemma 1: c0+n-1 ≥ len covers the union
    /// of the N per-byte guards, same 71 thunk) + one zero-extending
    /// wide load (lemma 2: bit-identical to the |/^ tree value).
    pub(crate) fn emit_fused_le(&mut self, recv: &dyn IRNode, c0: u64, n: u64) -> Ret<()> {
        self.eval_bytes(recv)?;
        let rl = self.pop_reg();
        let lp = self.pop_vreg();
        let rp = loc_reg(lp);
        let sc1 = self.asm.sc1;
        self.li_k(sc1, c0 + n - 1);
        self.asm.bgeu(sc1, rl, self.fault_cast);
        let d = self.push_vreg();
        let (f3, w) = match n {
            8 => (3u8, 8u64),
            4 => (6u8, 4u64),
            _ => (1u8, 2u64),
        };
        self.emit_wide_at(d, rp, c0, f3);
        debug_assert!(w == n);
        Ok(())
    }

    /// BE mul-add group (lemma 3 + 4). `base`: None = const offsets
    /// (single guard on c0+n-1); Some = var base (guard on the base like
    /// the original leaf-0 path, then the overflow-free subtraction span
    /// guard `len - t0 < n → 71`), then the wide load and the shift
    /// recombination.
    pub(crate) fn emit_fused_be(
        &mut self,
        recv: &dyn IRNode,
        base: Option<&dyn IRNode>,
        c0: u64,
        n: u64,
    ) -> Ret<()> {
        self.eval_bytes(recv)?;
        let (rp, rl, t0) = match base {
            None => {
                let rl = self.pop_reg();
                let lp = self.pop_vreg();
                (loc_reg(lp), rl, None)
            }
            Some(b) => {
                self.eval_int(b)?;
                let t0 = self.pop_reg();
                let rl = self.pop_reg();
                let lp = self.pop_vreg();
                (loc_reg(lp), rl, Some(t0))
            }
        };
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        match t0 {
            None => {
                // const offsets: one guard covers all leaves (lemma 1)
                self.li_k(sc1, c0 + n - 1);
                self.asm.bgeu(sc1, rl, self.fault_cast);
            }
            Some(t0) => {
                // var base: leaf-0 guard first (identical to the original
                // first byte()'s guard), then len - t0 ≥ n via subtraction
                // (safe: t0 < len proven by the first guard — no wrap)
                self.asm.bgeu(t0, rl, self.fault_cast);
                self.li_k(sc1, n);
                self.asm.sub(sc2, rl, t0);
                self.asm.bltu(sc2, sc1, self.fault_cast);
            }
        }
        let d = self.push_vreg();
        match t0 {
            None => {
                let (f3, off) = match n {
                    4 => (6u8, c0),
                    _ => (1u8, c0),
                };
                self.emit_wide_at(d, rp, off, f3);
            }
            Some(t0) => {
                self.asm.add(sc2, rp, t0);
                match n {
                    4 => self.asm.lwu(d, sc2, 0),
                    _ => self.asm.lhu(d, sc2, 0),
                }
            }
        }
        // shift recombination (lemma 3): x = the zero-extended LE load.
        // 64-bit shifts do NOT clear bytes above bit 32 — every byte is
        // masked out explicitly (only sc1/sc2, never a pool register).
        match n {
            4 => {
                // y = b0<<24 | b1<<16 | b2<<8 | b3
                self.asm.srli_(sc1, d, 8); // b1 | b2<<8 | b3<<16
                self.asm.andi(sc1, sc1, 0xFF); // b1
                self.asm.slli_(sc1, sc1, 16); // b1<<16
                self.asm.srli_(sc2, d, 16); // b2 | b3<<8
                self.asm.andi(sc2, sc2, 0xFF); // b2
                self.asm.slli_(sc2, sc2, 8); // b2<<8
                self.asm.or_(sc1, sc1, sc2); // b1<<16 | b2<<8
                self.asm.srli_(sc2, d, 24); // b3 (32-bit value: top byte)
                self.asm.or_(sc1, sc1, sc2); // | b3
                self.asm.andi(sc2, d, 0xFF); // b0
                self.asm.slli_(sc2, sc2, 24); // b0<<24
                self.asm.or_(d, sc1, sc2); // full swap into d
            }
            2 => {
                // y = ((x & 0xFF) << 8) | (x >> 8)  (x>>8 < 2^8 here)
                self.asm.andi(sc1, d, 0xFF);
                self.asm.srli_(sc2, d, 8);
                self.asm.slli_(sc1, sc1, 8);
                self.asm.or_(d, sc1, sc2);
            }
            _ => unreachable!("BE groups are N in {{2,4}} only"),
        }
        Ok(())
    }

    /// Dispatch for a marked group root: re-derive the group from the
    /// tree (same deterministic recognition as fuse_scan — the mark
    /// guarantees it succeeds) and emit.
    pub(crate) fn emit_fused_root(&mut self, node: &dyn IRNode, form: FuseForm) -> Ret<()> {
        let root = fuse_unwrap(node);
        match form {
            FuseForm::LeChain | FuseForm::LeExpr => {
                let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64, &dyn IRNode)> = Vec::new();
                fuse_flatten_le(root, &mut terms)
                    .ok_or("rv64 lower: internal: fused LE group shape lost")?;
                let (recv, _, _, _) = terms[0];
                let (c0, n) = match self.fuse.get(&node_addr(node)) {
                    Some(FuseMark::Root { c0, n, .. }) => (*c0, *n),
                    _ => return errf!("rv64 lower: internal: fused group mark lost"),
                };
                self.emit_fused_le(recv, c0, n)
            }
            FuseForm::BeConst | FuseForm::BeVar => {
                let mut terms: Vec<(&dyn IRNode, &dyn IRNode, u64)> = Vec::new();
                fuse_flatten_be(root, &mut terms)
                    .ok_or("rv64 lower: internal: fused BE group shape lost")?;
                let (recv, idx0, _) = terms[0];
                let (c0, n) = match self.fuse.get(&node_addr(node)) {
                    Some(FuseMark::Root { c0, n, .. }) => (*c0, *n),
                    _ => return errf!("rv64 lower: internal: fused group mark lost"),
                };
                let base = match form {
                    FuseForm::BeVar => Some(idx0),
                    _ => None,
                };
                self.emit_fused_be(recv, base, c0, n)
            }
        }
    }

    /// Top of stack: descriptor + count operand(s).
    pub(crate) fn emit_slice(&mut self, kind: SliceKind) -> Ret<()> {
        // Stack layout: Cut = [ptr, len, ost, n] (4 deep), others =
        // [ptr, len, n] (3 deep). Pop order must mirror the pushes.
        let n = self.pop_reg(); // len (Cut) / count (top)
        let ost = match kind {
            SliceKind::Cut => self.pop_reg(),
            _ => n,
        };
        let rl = self.pop_reg();
        let lp = self.pop_vreg();
        let rp = loc_reg(lp);
        match kind {
            SliceKind::Cut => {
                // ost + n must not wrap and ost+n must be <= rl
                self.asm.r(0, n, ost, 0, self.a0, 0b0110011); // add scratch = ost + n
                self.asm.bltu(self.a0, ost, self.fault_cast); // wrapped
                self.asm.bltu(rl, self.a0, self.fault_cast); // rl < ost+n
            }
            _ => {
                self.asm.bltu(rl, n, self.fault_cast); // n > rl
            }
        }
        let dstp = self.push_vreg();
        let dstl = self.push_vreg();
        match kind {
            SliceKind::Cut => {
                self.asm.add(dstp, rp, ost);
                self.asm.mv(dstl, n);
            }
            SliceKind::Left => {
                if dstp != rp {
                    // the ptr survives: a bank-ref source needs the copy
                    self.asm.mv(dstp, rp);
                }
                self.asm.mv(dstl, n);
            }
            SliceKind::Right => {
                self.asm.add(dstp, rp, rl);
                self.asm.r(0x20, n, dstp, 0, dstp, 0b0110011); // sub dstp, dstp, n
                self.asm.mv(dstl, n);
            }
            SliceKind::LDrop => {
                self.asm.add(dstp, rp, n);
                self.asm.r(0x20, n, rl, 0, dstl, 0b0110011); // sub dstl, rl, n
            }
            SliceKind::RDrop => {
                self.asm.r(0x20, n, rl, 0, dstl, 0b0110011); // sub dstl, rl, n
                if dstp != rp {
                    // the prefix ptr survives: a bank-ref source needs the copy
                    // (same shape as SliceKind::Left; BUG-009 fix — dstp was
                    // never written, so Im/Imc read a stale register)
                    self.asm.mv(dstp, rp);
                }
            }
        }
        Ok(())
    }

    /// Top of stack: uint of width `w` → its scalar bytes descriptor in the
    /// arena (the interpreter's `scalar_bytes`: `w/8` big-endian bytes for
    /// real widths, 1 canonical byte for the pure-bool width `w = 1`;
    /// `w = 64` is the historical 8-byte form, byte for byte).
    /// task 3: a u128 scalar emits 16 bytes big-endian (hi || lo) — the
    /// interpreter's `scalar_bytes(U128)`. Both halves are already popped
    /// into registers; the bytes land in the arena like every other cast
    /// and the (ptr, len) descriptor is pushed ptr-first, matching the
    /// literal-fold shape.
    pub(crate) fn emit_cbytes128(&mut self, lo: u8, hi: u8) -> Ret<()> {
        // arena room for 16 bytes
        self.asm.li(self.a0, self.arena_end);
        self.asm.addi(self.a1, S1, 16);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast);
        // unrolled big-endian write FIRST (the half registers are still the
        // popped operands; the descriptor pushes below re-hand exactly these
        // pool registers, so a store after the mv/addi would read the
        // descriptor instead of the value) — byte k of the 128-bit value is
        // (hi:lo) >> 8*(15-k), truncated to 8 bits by the sb — pass A over
        // hi (k 0..8), pass B over lo (k 8..16)
        let sc = self.asm.sc1;
        for (half, base) in [(hi, 0i32), (lo, 8)] {
            for j in 0..8u32 {
                self.asm.srli_(sc, half, 8 * (7 - j));
                self.asm.addi(self.a0, S1, base + j as i32);
                self.asm.sb(sc, self.a0, 0);
            }
        }
        // descriptor (ptr, len): ptr = the old bump, len = 16, then bump
        let rp = self.push_vreg();
        let rl = self.push_vreg();
        self.asm.mv(rp, S1);
        self.asm.addi(rl, ZERO, 16);
        self.asm.addi(S1, S1, 16); // bump
        Ok(())
    }
}
