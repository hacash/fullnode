use super::super::*;

impl<'a> Lower<'a> {

    /// Branch to `label` when `node` evaluates nonzero (`when_true`) or zero.
    /// Fuses comparisons into single conditional branches (the 0/1
    /// materialization disappears), including the `addi rd, rd, -K; c.beqz`
    /// compare-with-small-constant form. Operand evaluation order and fault
    /// behavior are identical to `eval`.
    pub(crate) fn branch_cond(&mut self, node: &dyn IRNode, when_true: bool, label: usize) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.branch_cond(&*w.node, when_true, label);
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            NOT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: not node shape unexpected")?;
                self.branch_cond(&*n.subx, !when_true, label)
            }
            AND | OR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: logic node shape unexpected")?;
                if op == AND && !when_true && is_pure_cond(node, &self.tracked) {
                    // fault-free conjunction: any false operand jumps — a
                    // branch chain, no materialization at all
                    self.branch_cond(&*n.subx, false, label)?;
                    self.branch_cond(&*n.suby, false, label)?;
                    return Ok(());
                }
                if op == OR {
                    // C16: a membership chain `x == K1 || x == K2 || ...`
                    // (shared x, small distinct constants, N ≥ 3) collapses
                    // into one bitmask test
                    if self.try_or_membership_mask(node, when_true, label)? {
                        return Ok(());
                    }
                    // C15: mirror of the AND chain — a fault-free disjunction
                    // with any true operand jumps; no materialization at all.
                    // The interpreter's OR is EAGER (binop_btw pops both
                    // operands, lgc_or extracts bool from both), so skipping
                    // the second operand's evaluation is only face-neutral
                    // because is_pure_cond guarantees neither operand carries
                    // a fault face — the same guard the AND chain relies on.
                    if when_true && is_pure_cond(node, &self.tracked) {
                        self.branch_cond(&*n.subx, true, label)?;
                        self.branch_cond(&*n.suby, true, label)?;
                        return Ok(());
                    }
                }
                // eager (identical fault behavior to `eval`)
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                let (d, ra, rb, sw) = self.pop2_dest(op, 64);
                if sw {
                    // E3: banked-left AND/OR — same swapped form as the
                    // arithmetic path (AND/OR are commutative)
                    self.emit_arith_commut(op, d, ra, rb, 64)?;
                } else if op == AND {
                    self.asm.and_(d, d, rb);
                } else {
                    self.asm.or_(d, d, rb);
                }
                let _ = self.pop_vreg(); // d (dead after the branch)
                if when_true {
                    self.asm.bne(d, ZERO, label);
                } else {
                    self.asm.beq(d, ZERO, label);
                }
                Ok(())
            }
            EQ | NEQ | LT | GT | LE | GE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: binary node shape unexpected")?;
                // T13: EQ/NEQ with a bytes/address operand materializes the
                // comparison (the eval bytes-eq path carries the fault
                // faces) and branches on the 0/1 result — the literal-K and
                // register-compare fast paths below are int-only
                if matches!(op, EQ | NEQ) {
                    let tx = node_res_ty(&*n.subx, self.slot)?;
                    let ty_ = node_res_ty(&*n.suby, self.slot)?;
                    if matches!(tx, Ty::Bytes | Ty::Address)
                        || matches!(ty_, Ty::Bytes | Ty::Address)
                    {
                        self.eval(node)?;
                        let rc = self.pop_reg();
                        if when_true {
                            self.asm.bne(rc, ZERO, label);
                        } else {
                            self.asm.beq(rc, ZERO, label);
                        }
                        return Ok(());
                    }
                }
                // task 3: either operand u128 → the 128-bit numeric compare.
                // Mixed widths stay legal (the interpreter's ordered compare
                // and content equality are numeric over u128); the narrower
                // side zero-extends into its pair.
                let wx = self.int_width(&*n.subx);
                let wy = self.int_width(&*n.suby);
                if wx == Some(128) || wy == Some(128) {
                    let (ex, ey) = (Self::entries_of_width(wx), Self::entries_of_width(wy));
                    let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                    self.eval_int(&*n.subx)?;
                    self.eval_int(&*n.suby)?;
                    if mx || my {
                        self.nil_guard_u128_cmp(op, mx, my, ex, ey);
                    }
                    let b = self.pop_u128_pair(wy);
                    let a = self.pop_u128_pair(wx);
                    // D8: the comparison fuses straight into the branch —
                    // no 0/1 byte materialization (the dead d vreg and its
                    // pop disappear with it)
                    self.emit_u128_cmp_branch(op, when_true, label, a, b);
                    return Ok(());
                }
                // (f3, swapped): the branch to emit when the comparison is
                // TRUE, in terms of (a, b) = (subx, suby). f3 ^ 1 inverts.
                let (f3, swap) = match op {
                    EQ => (0u32, false),
                    NEQ => (1, false),
                    LT => (6, false), // bltu a, b
                    GT => (6, true),  // a > b  <=>  bltu b, a
                    LE => (7, true),  // a <= b <=>  bgeu b, a
                    _ => (7, false),  // a >= b <=>  bgeu a, b
                };
                let f3 = if when_true { f3 } else { f3 ^ 1 };
                let lx = int_literal(&*n.subx);
                let ly = int_literal(&*n.suby);
                let lit = if lx.is_some() && ly.is_none() {
                    lx.map(|k| (true, k)) // literal on the left
                } else {
                    ly.map(|k| (false, k)) // literal on the right (or both)
                };
                if let Some((lit_left, k)) = lit {
                    // comparisons never fault and literals are pure: skip the
                    // literal operand entirely
                    let sub: &dyn IRNode = if lit_left { &*n.suby } else { &*n.subx };
                    let eq_like = matches!(op, EQ | NEQ);
                    // == K / != K with a small constant folds into
                    // addi r, r, -K; c.beqz/c.bnez (wrapping equality)
                    if eq_like && (0..=31).contains(&k) {
                        let msub = self.nil_val(sub);
                        self.eval_int(sub)?;
                        if msub {
                            // Nil vs literal: exactly-one-Nil → 90
                            self.nil_guard_tag(self.vstack.len() - 1, 90);
                        }
                        let l = *self
                            .vstack
                            .last()
                            .ok_or("rv64 lower: eq-like branch on empty stack")?;
                        let f = if (op == EQ) == when_true { 0 } else { 1 };
                        match l {
                            VLoc::Pool(r) => {
                                self.pop_vreg();
                                // the temp is dead after the branch: fold K
                                // into it (addi -K; c.beqz/c.bnez)
                                if k != 0 {
                                    self.asm.addi(r, r, -(k as i32));
                                }
                                self.asm.branch(f, r, ZERO, label);
                            }
                            VLoc::Ref(r) if k == 0 => {
                                self.pop_vreg();
                                // compare with zero needs no constant at all
                                self.asm.branch(f, r, ZERO, label);
                            }
                            VLoc::Ref(r) => {
                                // ref registers are read-only here: li + full
                                // cmp. The temp is pushed WHILE the ref still
                                // occupies its stack slot, so it can never be
                                // the same register (a pop-first push at an
                                // empty stack re-hands vregs[0] and the li
                                // would clobber the ref's value — a real bug
                                // forwarding introduced).
                                let t = self.push_vreg();
                                self.li_k(t, k);
                                self.pop_vreg(); // the ref (value still in r)
                                let (a_reg, b_reg) =
                                    if lit_left { (t, r) } else { (r, t) };
                                if swap {
                                    self.asm.branch(f3, b_reg, a_reg, label);
                                } else {
                                    self.asm.branch(f3, a_reg, b_reg, label);
                                }
                                let _ = self.pop_vreg(); // t (dead after the branch)
                            }
                        }
                        return Ok(());
                    }
                    // general: evaluate the non-literal side, materialize K
                    let msub = self.nil_val(sub);
                    self.eval_int(sub)?;
                    if msub {
                        self.nil_guard_tag(self.vstack.len() - 1, 90);
                    }
                    let t = self.push_vreg();
                    self.li_k(t, k);
                    // pop both (the registers keep their values; nothing is
                    // emitted in between), then branch in (a, b) order
                    let kreg = self.pop_reg();
                    let v = self.pop_reg();
                    // emitted order: swap mirrors (a,b) -> (b,a) for GT/LE
                    let (a_reg, b_reg) = if lit_left { (kreg, v) } else { (v, kreg) };
                    if swap {
                        self.asm.branch(f3, b_reg, a_reg, label);
                    } else {
                        self.asm.branch(f3, a_reg, b_reg, label);
                    }
                    return Ok(());
                }
                let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                if mx || my {
                    self.nil_guard_int_pair(op, mx, my);
                }
                let rb = self.pop_reg();
                let ra = self.pop_reg();
                if swap {
                    self.asm.branch(f3, rb, ra, label);
                } else {
                    self.asm.branch(f3, ra, rb, label);
                }
                Ok(())
            }
            _ => {
                self.eval_int(node)?;
                // task 3: a u128 condition value is a (lo, hi) pair — its
                // truthiness is (lo | hi) != 0, the interpreter's uint
                // truthiness over the full 128 bits
                if self.int_width(node) == Some(128) {
                    let hi = self.pop_reg();
                    let lo = self.pop_reg();
                    let d = self.push_vreg();
                    self.asm.or_(d, lo, hi);
                    if when_true {
                        self.asm.bne(d, ZERO, label);
                    } else {
                        self.asm.beq(d, ZERO, label);
                    }
                    let _ = self.pop_vreg();
                    return Ok(());
                }
                let rc = self.pop_reg();
                if when_true {
                    self.asm.bne(rc, ZERO, label);
                } else {
                    self.asm.beq(rc, ZERO, label);
                }
                Ok(())
            }
        }
    }

    /// C16 (impl/ctrl-dse): try to lower an OR chain of `x == K` comparisons
    /// sharing ONE non-literal x with small distinct constants as a single
    /// membership bitmask: `t = mask >> x; t & 1` (with an `x < 64` window
    /// check so a wrapped modular shift can never falsely match). The
    /// interpreter evaluates x at EVERY arm eagerly; the arms are back-to-
    /// back comparisons with no intervening statements, so all N evaluations
    /// observe the same value and fire the same deterministic fault at the
    /// first arm's position — one evaluation + one Nil guard preserves the
    /// fault face exactly. Returns true when the statement was consumed.
    /// Trigger: N ≥ 3 arms, every arm is EQ with an int-literal side K ≤ 63
    /// and a structurally identical int (non-u128) x, key set ≤ 64.
    pub(crate) fn try_or_membership_mask(
        &mut self,
        node: &dyn IRNode,
        when_true: bool,
        label: usize,
    ) -> Ret<bool> {
        // flatten the OR tree into its comparison leaves
        let mut leaves: Vec<&dyn IRNode> = Vec::new();
        fn flatten_or<'n>(node: &'n dyn IRNode, out: &mut Vec<&'n dyn IRNode>) {
            if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
                flatten_or(&*w.node, out);
                return;
            }
            if Bytecode::try_from_u8(node.bytecode()).ok() == Some(Bytecode::OR) {
                if let Some(d) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    flatten_or(&*d.subx, out);
                    flatten_or(&*d.suby, out);
                    return;
                }
            }
            out.push(node);
        }
        flatten_or(node, &mut leaves);
        if leaves.len() < 3 {
            return Ok(false);
        }
        // every leaf must be `EQ(x, K)` / `EQ(K, x)` with K ≤ 63, all naming
        // the same x
        let mut keys: Vec<u64> = Vec::new();
        let mut x: Option<&dyn IRNode> = None;
        let mut x_key: Option<Vec<u8>> = None;
        for leaf in &leaves {
            let d = match leaf.as_any().downcast_ref::<IRNodeDouble>() {
                Some(d) if Bytecode::try_from_u8(leaf.bytecode()).ok() == Some(Bytecode::EQ) => d,
                _ => return Ok(false),
            };
            let lx = int_literal(&*d.subx);
            let ly = int_literal(&*d.suby);
            let (xnode, k) = match (lx, ly) {
                (None, Some(k)) => (&*d.subx, k),
                (Some(k), None) => (&*d.suby, k),
                _ => return Ok(false), // both/neither literal — not an arm
            };
            if k > 63 {
                return Ok(false);
            }
            // structural identity of the shared x: the serialized prefix
            // form is a pure function of the subtree
            let key = xnode.serialize();
            match &x_key {
                None => {
                    x_key = Some(key);
                    x = Some(xnode);
                }
                Some(prev) if *prev == key => {}
                _ => return Ok(false), // a different x — not one membership test
            }
            keys.push(k);
        }
        let x = x.ok_or("rv64 lower: internal: empty membership chain")?;
        // x must be a plain int value (a u128 x is a (lo, hi) pair; the
        // interpreter's bytes/address EQ carries content-equality faces)
        if matches!(node_res_ty(x, self.slot)?, Ty::Bytes | Ty::Address | Ty::List | Ty::Map)
            || self.int_width(x) == Some(128)
        {
            return Ok(false);
        }
        // the mask: bit K set for every key
        let mut mask: u64 = 0;
        for k in &keys {
            mask |= 1u64 << k;
        }
        // ---- emit ----
        // x is evaluated ONCE, at the first arm's position (the interpreter's
        // first evaluation); the exactly-one-Nil 90 face fires there too —
        // a Nil x faults against every literal arm alike.
        let mx = self.nil_val(x);
        self.eval_int(x)?;
        if mx {
            self.nil_guard_tag(self.vstack.len() - 1, 90);
        }
        let xv = loc_reg(*self.vstack.last().ok_or("rv64 lower: membership x missing")?);
        // scratch registers ride ABOVE x (a pop-first push would re-hand x's
        // own pool register — the control.rs eq-branch re-hand hazard)
        let t = self.push_vreg();
        let m = self.push_vreg();
        self.li_k(m, mask);
        // the shift amount is taken mod 64 on RISC-V: an x ≥ 64 can never be
        // a member (every K ≤ 63), so window-check before shifting. NOT a
        // member goes to `label` when the caller branches on false, else to
        // the fall-through continuation.
        let lim = self.push_vreg();
        self.asm.addi(lim, ZERO, 64);
        if when_true {
            let l_not = self.asm.new_label();
            self.asm.bgeu(xv, lim, l_not);
            self.asm.r(0, xv, m, 5, t, 0b0110011); // srl t, m, xv
            self.asm.andi(t, t, 1);
            self.asm.bne(t, ZERO, label);
            self.place(l_not);
        } else {
            self.asm.bgeu(xv, lim, label);
            self.asm.r(0, xv, m, 5, t, 0b0110011); // srl t, m, xv
            self.asm.andi(t, t, 1);
            self.asm.beq(t, ZERO, label);
        }
        let _ = self.pop_vreg(); // lim
        let _ = self.pop_vreg(); // m
        let _ = self.pop_vreg(); // t
        let _ = self.pop_vreg(); // x
        Ok(true)
    }

    /// Store an int value register into a local slot (banked home or frame).
    pub(crate) fn store_int_slot(&mut self, slot: u8, v: u8) {        if let Some(r) = self.bank_int(slot) {
            self.kill_fwd_reg(r);
            if r != v {
                self.asm.mv(r, v);
            }
            self.fwd_int[slot as usize] = None;
        } else if self.store_is_dead(slot) {
            self.fwd_int[slot as usize] = None;
        } else {
            let off = self.slot_offset(slot);
            self.asm.sd(v, S0, off);
            self.fwd_int[slot as usize] = Some(v);
        }
    }

    /// Lower a matched constant-select run as one bounds-checked byte-table
    /// lookup: `t = x - base; if t >= span skip; t = &table; v = t[x];
    /// slot = v`. Holes and the skipped path both leave the slot at D, which
    /// the preceding default store guarantees.
    pub(crate) fn emit_const_select(
        &mut self,
        read_slot: u8,
        write_slot: u8,
        arms: &[SelArm],
        d: u64,
    ) -> Ret<()> {
        let base = arms.iter().map(|a| a.key).min().unwrap();
        let top = arms.iter().map(|a| a.key).max().unwrap();
        let span = (top - base + 1) as usize;
        let mut table = vec![d as u8; span];
        for a in arms {
            table[(a.key - base) as usize] = a.val as u8;
        }
        let lbl = self.lit_label(&table);
        // operand: a pure slot read evaluated once (the chain re-read it
        // per arm; slot reads cannot fault, so hoisting is sound)
        let mut extra = 0usize;
        let xv = if let Some(r) = self.bank_int_read(read_slot) {
            r
        } else {
            let off = self.slot_offset(read_slot);
            let r = self.push_vreg();
            self.asm.ld(r, S0, off);
            extra += 1;
            r
        };
        let t = self.push_vreg();
        if base == 0 {
            self.asm.mv(t, xv);
        } else {
            self.asm.addi(t, xv, -(base as i32));
        }
        let sr = self.push_vreg();
        self.asm.addi(sr, ZERO, span as i32); // c.li (span ≤ 31)
        let skip = self.asm.new_label();
        self.asm.bgeu(t, sr, skip);
        let _ = self.pop_vreg(); // sr dead past the branch
        // the index stays in t; the table pointer needs its own register
        // (reusing t here would clobber the index and add the RAW x)
        let p = self.push_vreg();
        self.lit_addr(p, lbl);
        self.asm.add(p, p, t);
        let v = self.push_vreg();
        self.asm.lbu(v, p, 0);
        self.store_int_slot(write_slot, v);
        // narrow-PUT face: every select arm stores a u8-width table byte —
        // the interpreter's arm PUT makes the slot a u8 value
        self.shadow_put(write_slot, Some(8));
        let _ = self.pop_vreg(); // v
        let _ = self.pop_vreg(); // p
        let _ = self.pop_vreg(); // t
        for _ in 0..extra {
            let _ = self.pop_vreg(); // xv (if loaded)
        }
        self.place(skip);
        Ok(())
    }

    /// The single `PUT slot ← expr` statement of a then-arm, or None.
    pub(crate) fn single_put_arm<'n>(&self, arm: &'n dyn IRNode) -> Option<&'n IRNodeParam1Single> {
        if let Some(w) = arm.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.single_put_arm(&*w.node);
        }
        match Bytecode::try_from_u8(arm.bytecode()) {
            Ok(Bytecode::PUT) => arm.as_any().downcast_ref::<IRNodeParam1Single>(),
            Ok(Bytecode::IRBLOCK) | Ok(Bytecode::IRBLOCKR) | Ok(Bytecode::IRLIST) => {
                let arr = arm.as_any().downcast_ref::<IRNodeArray>()?;
                if arr.subs.len() == 1 {
                    self.single_put_arm(&*arr.subs[0])
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Try to lower `if <cmp> { slot = <other> }` (else-less) as one fused
    /// max/min update: `slot = max(slot, other)` for `if other > slot` /
    /// `if slot < other`, min for the opposite direction. The compared
    /// operand is materialized ONCE (the generic path re-materializes it
    /// inside the arm). Returns true when the statement was consumed.
    pub(crate) fn try_maxmin_update(&mut self, node: &dyn IRNode) -> Ret<bool> {
        let n = match node.as_any().downcast_ref::<IRNodeTriple>() {
            Some(n)
                if Bytecode::try_from_u8(node.bytecode()).ok() == Some(Bytecode::IRIF)
                    && is_zero_byte_stmt(&*n.subz) =>
            {
                n
            }
            _ => return Ok(false),
        };
        let cond = Bytecode::try_from_u8(n.subx.bytecode()).ok();
        if !matches!(cond, Some(Bytecode::GT) | Some(Bytecode::LT)) {
            return Ok(false);
        }
        let c = match n.subx.as_any().downcast_ref::<IRNodeDouble>() {
            Some(c) => c,
            None => return Ok(false),
        };
        let put = match self.single_put_arm(&*n.suby) {
            Some(p) => p,
            None => return Ok(false),
        };
        let slot = put.para;
        if matches!(self.slot.get(slot as usize), Some(SlotTy::Input) | None) {
            return Ok(false);
        }
        // task 3: the fused max/min store writes ONE register — a u128 slot
        // (or a u128 operand) takes the generic path
        if self.slot_w.get(slot as usize).copied().flatten() == Some(128)
            || self.int_width(&*put.subx) == Some(128)
        {
            return Ok(false);
        }
        // the arm operand must be the SAME value the slot was compared with:
        // a matching literal, or a read of the same slot-free expression is
        // too weak — require literal-on-both-sides or GET-of-slot on one
        // side only (the operand is the other side, literal or expr).
        // one side must read exactly this slot; the other is the operand
        let x_is_slot = slot_get_index(&*c.subx) == Some(slot);
        let y_is_slot = slot_get_index(&*c.suby) == Some(slot);
        if x_is_slot == y_is_slot {
            return Ok(false); // both or neither — not the update shape
        }
        let (slot_on_left, operand, cmp) = if x_is_slot {
            (true, &*c.suby, cond.unwrap())
        } else {
            (false, &*c.subx, cond.unwrap())
        };
        // the arm must store the very same operand (literal equal, or the
        // identical GET)
        let arm_matches = match int_literal(operand) {
            Some(k) => int_literal(&*put.subx) == Some(k),
            None => {
                slot_get_index(operand).is_some()
                    && slot_get_index(&*put.subx) == slot_get_index(operand)
                    && int_literal(&*put.subx).is_none()
            }
        };
        if !arm_matches {
            return Ok(false);
        }
        // direction: `slot < K` or `K > slot` → max(slot, K); `slot > K` or
        // `K < slot` → min(slot, K)
        let is_max = match (cmp, slot_on_left) {
            (Bytecode::LT, true) | (Bytecode::GT, false) => true,
            (Bytecode::GT, true) | (Bytecode::LT, false) => false,
            _ => return Ok(false),
        };
        // ---- emit ----
        // operand first (identical fault order to the original condition:
        // the IRIF evaluates subx, whose left child evaluates first). The
        // operand stays ON the stack while the slot's home is loaded — a
        // pop-first fresh push would re-hand (and the ld clobber) the very
        // register the operand lives in (a real bug the first cut shipped).
        // Nil guards (BUG-012): the ordered comparison faults 90 on ANY
        // Nil side — the slot side reads its init flag directly.
        let opnode: &dyn IRNode = if slot_on_left { &*c.suby } else { &*c.subx };
        let slot_get: &dyn IRNode = if slot_on_left { &*c.subx } else { &*c.suby };
        let (m_slot, m_op) = (self.nil_val(slot_get), self.nil_val(opnode));
        let slot_first = slot_on_left; // comparison evaluation order
        if slot_first && m_slot {
            self.nil_guard_flag(slot, 90);
        }
        self.eval_int(opnode)?;
        if m_op {
            self.nil_guard_tag(self.vstack.len() - 1, 90);
        }
        if !slot_first && m_slot {
            self.nil_guard_flag(slot, 90);
        }
        let t = loc_reg(
            *self.vstack.last().ok_or("rv64 lower: maxmin operand missing")?,
        );
        // slot's current value: bank write home (segment boundary = this
        // PUT), copying across from the read home when they differ
        if let Some(rw) = self.bank_int(slot) {
            let rr = self.bank_int_read(slot).unwrap_or(rw);
            self.kill_fwd_reg(rw);
            self.fwd_int[slot as usize] = None;
            if rr != rw {
                self.asm.mv(rw, rr);
            }
            if is_max {
                self.emit_max(rw, t);
            } else {
                self.emit_min(rw, t);
            }
            // narrow-PUT face: the max/min result keeps the common width —
            // grow by the operand's own static width. C13: the fused
            // comparison/arithmetic above keeps every fault face; the shadow
            // grow and the init-flag set are auxiliary machine that dies with
            // a proven-dead store (store_is_dead: last_read timeline +
            // loop-carried / BUG-022 backedge exemptions, identical to the
            // frame arm below).
            if !self.store_is_dead(slot) {
                self.shadow_grow(slot, self.int_width(opnode));
                if self.tracked.get(slot as usize).copied() == Some(true) {
                    // the guards proved both inputs non-Nil
                    self.nil_set_flag_one(slot);
                }
            }
        } else {
            let off = self.slot_offset(slot);
            // the operand is still pushed, so this fresh register can
            // never be the operand's
            let d = self.push_vreg();
            self.asm.ld(d, S0, off);
            if is_max {
                self.emit_max(d, t);
            } else {
                self.emit_min(d, t);
            }
            if self.store_is_dead(slot) {
                self.fwd_int[slot as usize] = None;
            } else {
                self.asm.sd(d, S0, off);
                self.fwd_int[slot as usize] = Some(d);
                // narrow-PUT face: grow by the operand's own static width
                self.shadow_grow(slot, self.int_width(opnode));
                if self.tracked.get(slot as usize).copied() == Some(true) {
                    self.nil_set_flag_one(slot);
                }
            }
            let _ = self.pop_vreg(); // d
        }
        let _ = self.pop_vreg(); // the operand
        Ok(true)
    }

    /// Try to lower a maximal constant-select run starting at `subs[start]`.
    /// Returns the number of subs consumed, or None (fall back to per-stmt
    /// lowering). All conditions are static on the frozen IR.
    pub(crate) fn try_const_select_run(
        &mut self,
        subs: &[Box<dyn IRNode>],
        start: usize,
    ) -> Ret<Option<usize>> {
        let mut arms: Vec<SelArm> = Vec::new();
        let mut rw: Option<(u8, u8)> = None;
        let mut consumed = 0usize;
        let mut j = start;
        while j < subs.len() {
            if is_zero_byte_stmt(&*subs[j]) {
                if arms.is_empty() {
                    return Ok(None); // runs start on a real arm
                }
                j += 1;
                consumed += 1;
                continue;
            }
            match match_sel_arm(&*subs[j]) {
                Some((g, w, a)) => {
                    match rw {
                        None => rw = Some((g, w)),
                        Some(x) if x != (g, w) => break,
                        _ => {}
                    }
                    arms.push(a);
                    j += 1;
                    consumed += 1;
                }
                None => break,
            }
        }
        if arms.len() < 4 {
            return Ok(None);
        }
        let mut keys: Vec<u64> = arms.iter().map(|a| a.key).collect();
        keys.sort_unstable();
        if keys.windows(2).any(|w| w[0] == w[1]) {
            return Ok(None); // first-hit-wins vs table conflict
        }
        let base = keys[0];
        let span = (keys[keys.len() - 1] - base + 1) as usize;
        if span > 31 || base > 2047 {
            return Ok(None);
        }
        let d = match default_predecessor(subs, start, rw.unwrap().1) {
            Some(d) => d,
            None => return Ok(None),
        };
        // Nil guard (BUG-012): every arm compares the read slot against a
        // literal — a Nil slot faults 90 at the FIRST comparison, before
        // any arm can store. The preceding default PUT already set the
        // write slot's flag.
        if self.tracked.get(rw.unwrap().0 as usize).copied() == Some(true) {
            self.nil_guard_flag(rw.unwrap().0, 90);
        }
        self.emit_const_select(rw.unwrap().0, rw.unwrap().1, &arms, d)?;
        Ok(Some(consumed))
    }
}
