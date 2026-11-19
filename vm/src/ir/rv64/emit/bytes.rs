use super::super::*;

impl<'a> Lower<'a> {

    /* ---- arena / bytes helpers ---- */

    /// `for i in 0..len: *S1++ = src[i]` — clobbers the scratch pair, bumps S1.
    pub(crate) fn emit_memcpy(&mut self, src: u8, len: u8) {
        // B9: the bump invalidates any arena-top fact (a fact consumed by
        // an in-place CAT is read before the calls — see emit_cat).
        self.cat_top_kill();
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

    /// B11: the constant-length sibling of [`Self::emit_memcpy`] — a scalar
    /// buf_set source's canonical byte length is a compile-time constant
    /// (nb ∈ {1,2,4,8,16}), so the byte loop becomes one zero-extending
    /// load + one same-width store + the S1 bump (8 instructions → 3; the
    /// 16-byte form takes two words → 5). The load's zero-extension and
    /// the store's low-byte truncation are exact inverses, so the copy is
    /// byte-for-byte the loop's (lhu/lwu already in use, asm.rs; wide
    /// access at an unaligned arena top is safe in all three engines —
    /// container.md §0). Clobbers a0 (a strict subset of the loop's
    /// a0/a1); the caller's arena check already covered the bump.
    pub(crate) fn emit_memcpy_const(&mut self, src: u8, nb: u32) {
        self.cat_top_kill(); // B9: the S1 bump below (see emit_memcpy)
        match nb {
            1 => {
                self.asm.lbu(self.a0, src, 0);
                self.asm.sb(self.a0, S1, 0);
            }
            2 => {
                self.asm.lhu(self.a0, src, 0);
                self.asm.sh(self.a0, S1, 0);
            }
            4 => {
                self.asm.lwu(self.a0, src, 0);
                self.asm.sw(self.a0, S1, 0);
            }
            8 => {
                self.asm.ld(self.a0, src, 0);
                self.asm.sd(self.a0, S1, 0);
            }
            _ => {
                // 16: two words (little-endian loads/stores of the same
                // bytes — order between the words is the copy order)
                self.asm.ld(self.a0, src, 0);
                self.asm.sd(self.a0, S1, 0);
                self.asm.ld(self.a0, src, 8);
                self.asm.sd(self.a0, S1, 8);
            }
        }
        self.asm.addi(S1, S1, nb as i32);
    }

    /// Arena-exhaustion check for `S1 + len_reg` against the declared end.
    pub(crate) fn emit_arena_check(&mut self, len_reg: u8) {
        self.asm.li(self.a0, self.arena_end);
        self.asm.add(self.a1, S1, len_reg);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast); // arena exhausted
    }

    /// Top of stack: `a ++ b` → one bytes descriptor.
    ///
    /// B9 (impl/arena-mapflow): when the arena-top fact names the receive
    /// operand's descriptor (pa, la) — its bytes are the LAST arena
    /// allocation, pa + la == S1 — the receive-side copy is dropped and the
    /// right operand's bytes are appended directly over S1. Correctness:
    /// CAT's semantics is to PRODUCE a new value; the append writes
    /// [pa+la, pa+la+lb) which sits above every already-allocated region
    /// (the arena only grows, every allocated region ends at or below S1),
    /// so every holder of the old (pa, la) view — other slots, DUP'd stack
    /// pairs — reads exactly its own old bytes (alias holders index by
    /// their own len; nothing is OVERWRITTEN, unlike premise 24's buf_set
    /// case). The only premise is the compile-time S1 == pa + la proof.
    /// The fault faces keep their order and position: overflow (90) first,
    /// then the arena budget (71), then the copies — the deleted receive
    /// copy is face-free (a pure byte loop). The result is re-established
    /// as the new arena top (dstp + dstl == S1 after both bumps).
    pub(crate) fn emit_cat(&mut self) -> Ret<()> {
        // operands are read-only (memcpy sources, lengths); the two result
        // slots are fresh pool registers.
        let lb = self.pop_reg();
        let pb = self.pop_reg();
        let la = self.pop_reg();
        let pa = self.pop_reg();
        // the fact is consumed BEFORE the overflow-check join label below
        // (place(ok) kills it — the append decision must be made first)
        let recv_top = self.cat_top == Some((pa, la));
        if std::env::var("RV64_INV_CHECK").is_ok() && recv_top {
            assert_eq!(
                self.cat_top_gen, self.s1_gen,
                "INV B9: live arena-top fact with a stale bookkeeping epoch"
            );
        }
        // total = la + lb (checked)
        self.asm.r(0, lb, la, 0, self.a1, 0b0110011); // add a1 = la + lb
        let ok = self.asm.new_label();
        self.asm.bgeu(self.a1, la, ok);
        self.trap(EXIT_ARITH);
        self.place(ok);
        self.emit_arena_check(self.a1);
        // copies bump S1 by la then lb (the receive-side copy is the
        // arena-top fact's whole payoff: the bytes are already in place)
        if !recv_top {
            self.emit_memcpy(pa, la);
        }
        self.emit_memcpy(pb, lb);
        let dstp = self.push_vreg();
        let dstl = self.push_vreg();
        self.asm.r(0, lb, la, 0, dstl, 0b0110011); // add dstl, la + lb
        self.asm.r(0x20, dstl, S1, 0, dstp, 0b0110011); // sub dstp, S1 - dstl
        self.cat_top_establish(dstp, dstl);
        Ok(())
    }

    /* ---- C6 (impl/cat-u128fwd): CAT chain fusion ---- */

    /// Is `node` (post-WRAP) a CAT whose lowering takes the DYNAMIC arm —
    /// no empty-side fold (`x ++ ""` / `"" ++ x`), not `lit ++ lit`, and no
    /// P3-IDIOMS Root mark (a marked root lowers through
    /// `emit_fused_root`, never through its opcode arm — the recognizer
    /// only marks BOR/BXOR/ADD, but the defensive lookup costs one map
    /// probe)? Only such nodes fuse; every other child is a LEAF whose
    /// `eval_bytes` reproduces its own arm (fold or plain eval)
    /// byte-for-byte.
    fn cat_dyn_parts<'n>(
        node: &'n dyn IRNode,
        fuse: &FuseMap,
    ) -> Option<(&'n dyn IRNode, &'n dyn IRNode)> {
        let mut cur = node;
        while let Some(w) = cur.as_any().downcast_ref::<IRNodeWrapOne>() {
            cur = &*w.node;
        }
        if fuse.contains_key(&node_addr(cur)) {
            return None;
        }
        let n = cur.as_any().downcast_ref::<IRNodeDouble>()?;
        if n.inst != Bytecode::CAT {
            return None;
        }
        // mirror the CAT arm's own fold conditions exactly: a node whose
        // arm would fold stays whole
        let lx = bytes_literal(&*n.subx);
        let ly = bytes_literal(&*n.suby);
        if lx.as_ref().is_some_and(|a| a.is_empty()) || ly.as_ref().is_some_and(|b| b.is_empty()) {
            return None;
        }
        if lx.is_some() && ly.is_some() {
            return None;
        }
        Some((&*n.subx, &*n.suby))
    }

    /// C6 trigger + depth gate (a pure static decision). Fuse iff some
    /// child is a fusion-able CAT — a single-layer `a ++ b` keeps the
    /// per-node emission byte-for-byte — and the STAGED WALK fits the
    /// value-register pool: leaf i (in the layered eval order) evaluates
    /// with the 2·i earlier leaf pairs already on the stack, so its own
    /// peak working depth must keep the total within the pool. The peak is
    /// measured by a scratch lowering (the ImcRemap probe pattern: an
    /// empty plan materializes every operand as a Pool push — banked Ref
    /// reads and forwarding hits occupy the SAME entry counts, and the
    /// tag/nil machinery pushes nothing — so the probe's peak is never
    /// below the real one), which keeps the fusion from ever turning a
    /// compiling body into a depth_err reject. Deterministic on the IR +
    /// pool size, so the ImcRemap probe pass and the real lowering agree
    /// on every site: a fusion the probe accepts reaches depth
    /// base+2·i+W_i there, capping the probe max_depth (hence the real
    /// pool) at or above it; a site the probe refused exceeds 8 positions,
    /// which a clamped real pool refuses too.
    pub(crate) fn cat_fusion_gate(&self, node: &dyn IRNode) -> bool {
        let Some((x, y)) = Self::cat_dyn_parts(node, &self.fuse) else {
            return false;
        };
        if Self::cat_dyn_parts(x, &self.fuse).is_none()
            && Self::cat_dyn_parts(y, &self.fuse).is_none()
        {
            return false; // single-layer CAT: the per-node emission stays
        }
        let mut peaks = Vec::new();
        self.cat_leaf_peaks(node, &mut peaks);
        for (i, w) in peaks.iter().enumerate() {
            if self.vstack.len() + 2 * i + w > self.vregs.len() {
                return false;
            }
        }
        true
    }

    /// Per-leaf staged peak depths of a fusion-able CAT subtree, in eval
    /// order. Errors from the scratch lowering (a shape the probe context
    /// cannot lower) refuse the fusion — the unfused path lowers them.
    fn cat_leaf_peaks(&self, node: &dyn IRNode, out: &mut Vec<usize>) {
        match Self::cat_dyn_parts(node, &self.fuse) {
            Some((x, y)) => {
                self.cat_leaf_peaks(x, out);
                self.cat_leaf_peaks(y, out);
            }
            None => {
                if let Some(w) = self.leaf_eval_peak(node) {
                    out.push(w);
                } else {
                    // poison the gate: this subtree never fuses
                    out.push(usize::MAX);
                }
            }
        }
    }

    /// Scratch-lowering peak vstack depth of evaluating `leaf` from an
    /// empty stack (never below the real lowering's peak — see
    /// `cat_fusion_gate`). None when the scratch lowering errors.
    fn leaf_eval_peak(&self, leaf: &dyn IRNode) -> Option<usize> {
        let nilp = NilPlan {
            tracked: vec![false; self.slot.len()],
            occ: HashMap::new(),
            tag_bytes: false,
        };
        let order: Vec<usize> = (0..self.slot.len()).collect();
        let mut probe = Lower::new_mode_arena_nil(
            self.slot,
            self.needs_zero,
            Rv64Mode::Im,
            self.arena_end - ARENA_BASE,
            &nilp,
            &self.lists,
            &self.slot_w,
            &self.dyn_w,
            false,
            false,
            self.fin_park_levels,
            self.heap_seg,
            self.input_local,
            (
                self.mulw_routine,
                self.round_hu_routine,
                self.round_he_routine,
                self.mul128_routine,
                self.sqrt64_routine,
                self.sqrt64up_routine,
                self.sqrt128_routine,
                self.sqrt128up_routine,
            ),
        );
        probe.pair_root = self.pair_root.clone();
        probe.pair_skip = self.pair_skip.clone();
        probe.fuse = self.fuse.clone();
        match probe.eval_bytes(leaf) {
            Ok(()) => Some(probe.max_depth),
            Err(_) => None,
        }
    }

    /// C6 walk over a fusion-able CAT subtree: emit the evals + Nil(91)
    /// guards in the ORIGINAL per-layer order. Each layer keeps its
    /// `eval x, eval y, guard x, guard y` sequence and layers nest
    /// innermost-first, so the relative order of every evaluation and of
    /// every 91 guard is exactly the unfused emission's (the per-node
    /// `emit_cat` bodies sat strictly between their own layer's guards and
    /// the next layer's evals — deleting them only removes the copies and
    /// the checks fused into `emit_cat_fused`). A fusion-able child
    /// recurses; a leaf evaluates through the ordinary `eval_bytes` (the
    /// literal folds included). Only a LEAF can be may-Nil — the pre-pass
    /// records no occ.val on a CAT node (an empty-fold pass-through
    /// guards its own side internally) — and a leaf's tag position is
    /// absolute on the vstack, stable across the later pushes.
    fn emit_cat_chain_walk(&mut self, node: &dyn IRNode) -> Ret<()> {
        let (x, y) = Self::cat_dyn_parts(node, &self.fuse)
            .expect("emit_cat_chain_walk on a non-fusion-able CAT");
        let mut gx = None;
        let mut gy = None;
        if Self::cat_dyn_parts(x, &self.fuse).is_some() {
            self.emit_cat_chain_walk(x)?;
        } else {
            self.eval_bytes(x)?;
            if self.nil_val(x) {
                gx = Some(self.vstack.len() - 2);
            }
        }
        if Self::cat_dyn_parts(y, &self.fuse).is_some() {
            self.emit_cat_chain_walk(y)?;
        } else {
            self.eval_bytes(y)?;
            if self.nil_val(y) {
                gy = Some(self.vstack.len() - 2);
            }
        }
        // this layer's guards, x then y (the unfused arm's guard order:
        // x at len-4, y at len-2 after both pairs are up — the same
        // absolute positions)
        if let Some(d) = gx {
            self.nil_guard_tag(d, 91);
        }
        if let Some(d) = gy {
            self.nil_guard_tag(d, 91);
        }
        Ok(())
    }

    /// C6 fused emission for a CAT tree (some child is itself a dynamic
    /// CAT): all leaves are evaluated and guarded by
    /// [`Self::emit_cat_chain_walk`]; this half replaces every per-node
    /// `emit_cat` body with ONE overflow check + ONE arena check + one
    /// memcpy per leaf + ONE descriptor.
    ///
    /// Deleted inner checks — face argument. Each unfused inner node paid
    /// `add + bgeu + trap(90)` (overflow) and the `li + add + bgeu(71)`
    /// arena check. The overflow face is UNREACHABLE at every node: a live
    /// bytes value's length is bounded by the arena layout (types.rs — the
    /// arena is a fixed RW segment; a value must fit it, so len <
    /// arena_end = 0x8000 by default), and the input band and pool
    /// literals sit below the same 64 KiB layout cap, so with k leaves the
    /// true sum < k * arena_end < 2^64 (k is the compile-time leaf count
    /// of the frozen IR tree, bounded by the vstack pool depth the depth
    /// gate enforces) — the wrapped add can never wrap, exactly the face
    /// the per-node `bgeu` guarded. The fused emission keeps ONE check
    /// covering the total: `total >= k * arena_end → trap(90)`, with the
    /// bound materialized from the same compile-time k — unreachable under
    /// the same invariant, so it deletes only dead faces. The arena
    /// budget face (71) is allowed to shift position (the ctr.rs CAT
    /// precedent: "arena exhaustion → the CAT precedent (fault_cast)" —
    /// budget faces fire at the fused allocation instead of per
    /// intermediate); the reachable 91 guards keep their exact order.
/// Alias safety: the fused result is a FRESH arena allocation (S1 at
/// the walk's base) written once from sources that all live strictly
/// below the pre-copy S1 (arena values, input registers, pool
/// literals, bank homes) — the intermediate `a++b` view the unfused
/// path materialized is a consumed temporary with no possible
/// observer, and nothing is written in place. B9 exception: when the
/// head leaf carries the arena-top fact, its memcpy is skipped and the
/// result EXTENDS that leaf's region in place — emit_cat's alias
/// argument (append above every allocated region, old holders read by
/// their own len) applies unchanged; see the head_top note in the body.
    pub(crate) fn emit_cat_fused(&mut self, node: &dyn IRNode) -> Ret<()> {
        let base = self.vstack.len();
        self.emit_cat_chain_walk(node)?;
        let n = (self.vstack.len() - base) / 2;
        debug_assert!(n >= 3, "fused CAT chain with {n} leaves");
        // stage the leaf pairs bottom-first (pop top-first, like emit_cat)
        let mut leaves: Vec<(u8, u8)> = Vec::with_capacity(n);
        for _ in 0..n {
            let l = self.pop_reg();
            let p = self.pop_reg();
            leaves.push((p, l));
        }
        leaves.reverse();
        // B9 (impl/arena-mapflow): the HEAD leaf's bytes may end exactly at
        // the arena top (the arena-top fact named its descriptor when it
        // was evaluated, and nothing in the walk bumped S1 — any bump would
        // have killed the fact). Then leaf 0 is already in place at
        // [S1 - l0, S1) and its memcpy is dropped: leaves 1.. append over
        // S1 and the result's ptr = S1_after - total = the walk base - l0 =
        // leaf 0's ptr — the unchanged `sub dstp, S1 - dstl` below computes
        // exactly that. Alias safety is emit_cat's argument (append above
        // every allocated region; old holders read their own len), lifted
        // to the fused chain: the fused result OVERLAPS the head leaf's old
        // view instead of being a fresh allocation, and every other holder
        // of that descriptor still reads its own bytes untouched.
        let head_top = self.cat_top == Some(leaves[0]);
        // total = l1 + ... + ln, riding sc1 (disjoint from a0/a1 in every
        // mode, and emit_memcpy below clobbers only a0/a1/S1)
        let (_, l1) = leaves[0];
        self.asm.mv(self.asm.sc1, l1);
        for (_, li) in &leaves[1..] {
            // add sc1, li, sc1
            self.asm.r(0, self.asm.sc1, *li, 0, self.asm.sc1, 0b0110011);
        }
        // the single overflow check over the total (see the face note)
        let bound = (n as u64) * self.arena_end;
        let ok = self.asm.new_label();
        self.li_k(self.a0, bound);
        self.asm.bltu(self.asm.sc1, self.a0, ok);
        self.trap(EXIT_ARITH);
        self.place(ok);
        // the single arena check over the total
        self.emit_arena_check(self.asm.sc1);
        // copies bump S1 by exactly the total; the descriptor pushes stay
        // AFTER the loop — push_vreg hands the freed bottom pool registers,
        // which still name the not-yet-copied leaf sources (emit_cat has
        // the same discipline). dstl = total (kept in sc1 across the
        // copies: sc1 is disjoint from a0/a1 in every mode); dstp =
        // S1_after - total = the allocation base, the unfused shape's
        // `sub dstp, S1 - dstl`.
        for (i, (p, l)) in leaves.iter().enumerate() {
            if !(i == 0 && head_top) {
                self.emit_memcpy(*p, *l);
            }
        }
        let dstp = self.push_vreg();
        let dstl = self.push_vreg();
        self.asm.mv(dstl, self.asm.sc1);
        self.asm.r(0x20, dstl, S1, 0, dstp, 0b0110011); // sub dstp, S1 - dstl
        self.cat_top_establish(dstp, dstl);
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
        self.cat_top_kill(); // B9: this allocation bumps S1
        // arena room for 16 bytes
        self.asm.li(self.a0, self.arena_end);
        self.asm.addi(self.a1, S1, 16);
        self.asm.bgeu(self.a1, self.a0, self.fault_cast);
        // unrolled big-endian write FIRST (the half registers are still the
        // popped operands; the descriptor pushes below re-hand exactly these
        // pool registers, so a store after the mv/addi would read the
        // descriptor instead of the value) — byte k of the 128-bit value is
        // (hi:lo) >> 8*(15-k), truncated to 8 bits by the sb — pass A over
        // hi (k 0..8), pass B over lo (k 8..16). B10: the per-byte address
        // addi is redundant — S1 is the invariant base and the sb imm12
        // offset covers base+j (0..15), the heap_store_be_u64 direct-offset
        // pattern.
        let sc = self.asm.sc1;
        for (half, base) in [(hi, 0i32), (lo, 8)] {
            for j in 0..8u32 {
                self.asm.srli_(sc, half, 8 * (7 - j));
                self.asm.sb(sc, S1, base + j as i32);
            }
        }
        // descriptor (ptr, len): ptr = the old bump, len = 16, then bump
        let rp = self.push_vreg();
        let rl = self.push_vreg();
        self.asm.mv(rp, S1);
        self.asm.addi(rl, ZERO, 16);
        self.asm.addi(S1, S1, 16); // bump
        // B9: the fresh 16 bytes ARE the arena top (rp + 16 == S1)
        self.cat_top_establish(rp, rl);
        Ok(())
    }
}
