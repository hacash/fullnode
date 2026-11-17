use super::super::*;

impl<'a> Lower<'a> {


    pub(crate) fn eval(&mut self, node: &dyn IRNode) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.eval(&*w.node);
        }
        // P3-IDIOMS: a marked byte-assembly group root replaces the whole
        // subtree's lowering with one span-guarded wide load (doc 94).
        if let Some(FuseMark::Root { form, .. }) = self.fuse.get(&node_addr(node)) {
            return self.emit_fused_root(node, *form);
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            /* ---- literals ---- */
            P0 | P1 | P2 | P3 => {
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, (op as u8).wrapping_sub(P0 as u8) as i32);
            }
            PU8 => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1>()
                    .ok_or("rv64 lower: PU8 node shape unexpected")?;
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, p.para as i32);
            }
            PU16 => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam2>()
                    .ok_or("rv64 lower: PU16 node shape unexpected")?;
                let v = u16::from_be_bytes(p.para) as u64;
                let r = self.push_vreg();
                self.li_k(r, v);
            }
            PBUF | PBUFL => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParams>()
                    .ok_or("rv64 lower: PBUF node shape unexpected")?;
                let data = &p.para[if op == PBUF { 1 } else { 2 }..];
                let lbl = self.lit_label(data);
                let rp = self.push_vreg();
                let rl = self.push_vreg();
                self.lit_addr(rp, lbl);
                self.asm.li(rl, data.len() as u64);
            }
            PNBUF => {
                let rp = self.push_vreg();
                let rl = self.push_vreg();
                self.asm.mv(rp, ZERO);
                self.asm.addi(rl, ZERO, 0);
            }
            PTRUE | PFALSE => {
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, if op == PTRUE { 1 } else { 0 });
            }
            PNIL => {
                // A nil literal: one value entry carrying the Nil tag (the
                // frame tag byte must be written explicitly — a stale byte
                // from an earlier value at this depth could read as non-Nil).
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, 0);
                if self.tag_base >= 0 {
                    self.nil_set_tag_zero(self.vstack.len() - 1);
                }
            }
            /* ---- casts ---- */
            CU8 | CU16 | CU32 | CU64 => {
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: cast node shape unexpected")?;
                // T17: `list as uint` — the interpreter's cast of a Compo
                // faults CastFail(71) (extract_u32/to_uint reject it).
                if matches!(node_res_ty(&*sub.subx, self.slot)?, Ty::List) {
                    self.trap(71);
                    let _ = self.push_vreg();
                    return Ok(());
                }
                // Int literals arrive as CU32/CU64 wrapping PBUF BE bytes
                // (or PU8/PU16/P0..P3): fold the whole cast node at compile
                // time — ir_literal_value sees through the cast chain.
                let limit: Option<u64> = match op {
                    CU8 => Some(0xFF),
                    CU16 => Some(0xFFFF),
                    CU32 => Some(0xFFFF_FFFF),
                    _ => None, // CU64 takes the full u64 range
                };
                if let Ok(Some(v)) = crate::lang::ir_literal_value(node) {
                    if let Ok(n) = v.extract_u128() {
                        if n <= u64::MAX as u128 {
                            if let Some(lim) = limit {
                                if n > lim as u128 {
                                    return errf!(
                                        "rv64 lower: literal {n} overflows the cast target"
                                    );
                                }
                            }
                            let r = self.push_vreg();
                            self.li_k(r, n as u64);
                            return Ok(());
                        }
                    }
                }
                // T13: a bytes/address operand takes the interpreter's
                // `bytes_to_uint_width` face — BE-trim the leading zero
                // bytes, the remainder must fit the TARGET width, else
                // CastFail(71) (fit_be_bytes). A may-Nil bytes slot reads
                // len 0 when unassigned → 0, the interpreter's
                // to_u128(Nil) = 0.
                if matches!(
                    node_res_ty(&*sub.subx, self.slot)?,
                    Ty::Bytes | Ty::Address
                ) {
                    let nb = match op {
                        CU8 => 1,
                        CU16 => 2,
                        CU32 => 4,
                        _ => 8,
                    };
                    self.eval_bytes(&*sub.subx)?;
                    return self.emit_bytes_to_uint(nb as u64);
                }
                self.eval_int(&*sub.subx)?;
                if self.int_width(&*sub.subx) == Some(128) {
                    // task 3: narrowing a u128 — any bit above the target
                    // width is CastFail(71), the interpreter's cannot_cast
                    // face (`u64::try_from(v)` on the full u128)
                    let hi = self.pop_reg();
                    let lo = self.pop_reg();
                    self.asm.bne(hi, ZERO, self.fault_cast);
                    if let Some(lim) = limit {
                        self.li_k(self.a0, lim);
                        self.asm.bltu(self.a0, lo, self.fault_cast);
                    }
                    let d = self.push_vreg();
                    if d != lo {
                        self.asm.mv(d, lo);
                    }
                    return Ok(());
                }
                if let Some(limit) = limit {
                    let r = self.pop_own_replace();
                    self.li_k(self.a0, limit);
                    // trap iff r > limit (r == limit is IN range: 255 is a
                    // valid u8). The historical form was doubly wrong —
                    // bgeu(r, limit, ok) accepted overflows and trapped
                    // every valid value; nothing e2e-tested a runtime
                    // narrowing cast before the guard-semantics
                    // differential caught it.
                    self.asm.bltu(self.a0, r, self.fault_cast);
                }
            }
            CU128 => {
                // task 3: a u128 value is a (lo, hi) u64 pair — two vstack
                // entries, low half first. Literals fold (the interpreter's
                // cast_u128 of a uint literal is exact); a narrower operand
                // zero-extends (cast_to_uint_width only widens); a bool-ish
                // operand keeps the A2 compile-reject (the interpreter
                // faults 71 on `bool as u128`, the kernel rejects up front).
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: cast node shape unexpected")?;
                // T17: `list as u128` — CastFail(71) on the interpreter.
                if matches!(node_res_ty(&*sub.subx, self.slot)?, Ty::List) {
                    self.trap(71);
                    let _ = self.push_vreg();
                    let _ = self.push_vreg();
                    return Ok(());
                }
                if let Ok(Some(v)) = crate::lang::ir_literal_value(node) {
                    if let Ok(n) = v.extract_u128() {
                        let lo = self.push_vreg();
                        let hi = self.push_vreg();
                        self.li_k(lo, n as u64);
                        self.li_k(hi, (n >> 64) as u64);
                        return Ok(());
                    }
                }
                let sw = self.int_width(&*sub.subx);
                if matches!(
                    node_res_ty(&*sub.subx, self.slot)?,
                    Ty::Bytes | Ty::Address
                ) {
                    // T13: the bytes→u128 face (trim + fit 16, 71 on a
                    // longer non-zero prefix) — same helper, 16-byte target
                    self.eval_bytes(&*sub.subx)?;
                    return self.emit_bytes_to_uint128();
                }
                if sw.is_none() {
                    return errf!(
                        "rv64 lower: `as u128` is outside the translatable kernel subset \
                         (the operand has no static uint width — pin bool or comparison \
                         values with `as u64` first)"
                    );
                }
                self.eval_int(&*sub.subx)?;
                if sw == Some(128) {
                    return Ok(()); // already a (lo, hi) pair — identity cast
                }
                // zero-extend: the single value becomes (lo, hi=0)
                let src = self.pop_vreg();
                let lo = self.push_vreg();
                if let VLoc::Ref(r) = src {
                    if lo != r {
                        self.asm.mv(lo, r);
                    }
                }
                let hi = self.push_vreg();
                self.asm.addi(hi, ZERO, 0);
            }
            CBYTES => {
                // Task-1 A2: `as bytes` is width-exact in the interpreter
                // (cast_bytes → scalar_bytes): a uint emits ITS OWN width in
                // big-endian (u8 → 1 B, u16 → 2, u32 → 4, u64 → 8), bool
                // emits 1 canonical byte, bytes pass through. Literals fold
                // through the very same ir_literal_value + scalar_bytes the
                // interpreter uses; a u128 operand takes the 16-byte BE
                // runtime path (task 3 — hi || lo).
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: cast node shape unexpected")?;
                // T17: `list as bytes` — the interpreter's cast_bytes of a
                // Compo faults CastFail(71).
                if matches!(node_res_ty(&*sub.subx, self.slot)?, Ty::List) {
                    self.trap(71);
                    let _ = self.push_vreg();
                    let _ = self.push_vreg();
                    return Ok(());
                }
                if let Ok(Some(v)) = crate::lang::ir_literal_value(&*sub.subx) {
                    if matches!(
                        &v,
                        Value::Bool(_)
                            | Value::U8(_)
                            | Value::U16(_)
                            | Value::U32(_)
                            | Value::U64(_)
                            | Value::Bytes(_)
                    ) {
                        if let Some(be) = v.scalar_bytes() {
                            let lbl = self.lit_label(&be);
                            let rp = self.push_vreg();
                            let rl = self.push_vreg();
                            self.lit_addr(rp, lbl);
                            self.asm.li(rl, be.len() as u64);
                            return Ok(());
                        }
                    }
                }
                let m = self.nil_val(&*sub.subx);
                // T13: a bytes/address operand is the identity — the
                // interpreter's cast_bytes passes Bytes through unchanged
                // and an Address's scalar_bytes IS its 21 content bytes
                // (the kernel value is exactly that descriptor). Nil still
                // faults 71 (extract_bytes_with_error_code rejects Nil).
                match node_res_ty(&*sub.subx, self.slot)? {
                    Ty::Bytes | Ty::Address => {
                        self.eval_bytes(&*sub.subx)?;
                        if m {
                            // `nil as bytes` faults 71: the tag sits under
                            // the descriptor's ptr half
                            self.nil_guard_tag(self.vstack.len() - 2, 71);
                        }
                        return Ok(());
                    }
                    _ => {}
                }
                self.eval_int(&*sub.subx)?;
                let Some(w) = self.int_width(&*sub.subx) else {
                    return errf!(
                        "rv64 lower: `as bytes` is outside the translatable kernel subset \
                         (the operand has no static uint width — pin bool or comparison values \
                         with `as u64` before `as bytes`)"
                    );
                };
                if w == 128 {
                    // task 3: a u128 scalar emits 16 bytes BE (hi || lo) —
                    // the interpreter's scalar_bytes(U128)
                    let tag = self.vstack.len().checked_sub(2);
                    if m {
                        self.nil_guard_tag(
                            tag.ok_or("rv64 lower: u128 `as bytes` on a short stack")?,
                            71,
                        );
                    }
                    let hi = self.pop_reg();
                    let lo = self.pop_reg();
                    return self.emit_cbytes128(lo, hi);
                }
                if m {
                    // `nil as bytes` → CastFail(71) on the interpreter
                    self.nil_guard_tag(self.vstack.len() - 1, 71);
                }
                // narrow-PUT face: a mixed-width-write slot's view width is
                // the runtime shadow's value — the interpreter's cast_bytes
                // emits the CURRENT value's own width, which a plain
                // assignment can narrow below the slot's static join
                let mut view: &dyn IRNode = &*sub.subx;
                while let Some(wr) = view.as_any().downcast_ref::<IRNodeWrapOne>() {
                    view = &*wr.node;
                }
                if let Some(idx) = slot_get_index(view).filter(|&i| self.dyn_w_flag(i)) {
                    return self.emit_cbytes_dyn(idx);
                }
                self.emit_cbytes(w)?;
            }
            CTO => {
                // T13: the CTO family the surface emits — `as bool`
                // (truthiness) and `as address` (21B + version decode),
                // the address literal being `CTO(Address, PBUF 21B)`.
                // Semantics ground truth: cast.rs cast_to_ty / convert.rs
                // extract_bool / to_u128, address_from_bytes (len == 21 &&
                // version ∈ {0,1,5}, else CastFail 71).
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: CTO node shape unexpected")?;
                if p.para == crate::value::ValueTy::Bool as u8 {
                    let oty = node_res_ty(&*p.subx, self.slot)?;
                    match oty {
                        Ty::Int => {
                            // uint truthiness — Nil (a may-Nil slot) is
                            // false: extract_bool(Nil) = false and the
                            // zero-initialized slot reads 0
                            self.eval_int(&*p.subx)?;
                            if self.int_width(&*p.subx) == Some(128) {
                                let hi = self.pop_reg();
                                let lo = self.pop_reg();
                                let d = self.push_vreg();
                                self.asm.or_(d, lo, hi);
                                // d = (0 < d) — 0/1 normalization
                                self.asm.r(0, d, ZERO, 3, d, 0b0110011); // sltu
                                return Ok(());
                            }
                            let d = self.pop_own_replace();
                            self.asm.r(0, d, ZERO, 3, d, 0b0110011); // sltu d = d != 0
                            return Ok(());
                        }
                        Ty::Bytes | Ty::Address => {
                            // any non-zero byte → true (extract_bool);
                            // a may-Nil bytes slot reads len 0 → false,
                            // the interpreter's extract_bool(Nil) = false
                            self.eval_bytes(&*p.subx)?;
                            return self.emit_bytes_truthy();
                        }
                        other => {
                            return errf!(
                                "rv64 lower: `as bool` is outside the translatable kernel \
                                 subset (operand type {})", ty_name(other)
                            )
                        }
                    }
                }
                if p.para == crate::value::ValueTy::Address as u8 {
                    // literal operand (the address literal form): validate
                    // 21B + supported version at compile time and emit the
                    // content bytes directly
                    if let Some(b) = bytes_literal(&*p.subx) {
                        if !Self::literal_is_valid_address(&b) {
                            // the shared front end already rejects literal
                            // casts that cannot decode; this arm is the
                            // defensive mirror (fail-fast of the
                            // interpreter's deterministic 71)
                            return errf!(
                                "rv64 lower: `as address` literal is not a decodable \
                                 21-byte address"
                            );
                        }
                        let lbl = self.lit_label(&b);
                        let rp = self.push_vreg();
                        let rl = self.push_vreg();
                        self.lit_addr(rp, lbl);
                        self.asm.li(rl, b.len() as u64);
                        return Ok(());
                    }
                    match node_res_ty(&*p.subx, self.slot)? {
                        Ty::Address => {
                            // already an address value — the identity cast
                            self.eval_bytes(&*p.subx)?;
                        }
                        Ty::Bytes => {
                            let m = self.nil_val(&*p.subx);
                            self.eval_bytes(&*p.subx)?;
                            // cast_bytes(Nil) faults 71 BEFORE the length /
                            // version checks (extract_bytes_with_error_code)
                            if m {
                                self.nil_guard_tag(self.vstack.len() - 2, 71);
                            }
                            self.emit_address_checks()?;
                        }
                        other => {
                            // a uint/bool operand's scalar_bytes is 1/2/4/8/
                            // 16 bytes — never 21, the interpreter faults 71
                            // deterministically: compile-time fail-fast
                            return errf!(
                                "rv64 lower: `as address` is outside the translatable kernel \
                                 subset (an {} operand can never be a 21-byte address — the \
                                 interpreter faults CastFail 71)",
                                ty_name(other)
                            );
                        }
                    }
                    return Ok(());
                } else {
                    return errf!(
                        "rv64 lower: opcode CTO is outside the translatable kernel subset \
                         (target type id {})", p.para
                    );
                }
            }
            /* ---- locals ---- */
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
                // BUG-012: a may-Nil read publishes the slot's init flag as
                // the value's tag (the consumer faults, never the GET)
                let mnil = self.nil_val(node);
                self.emit_get_slot(idx, mnil)?;
            }
            // T11 (pure-compute subset extension): constant-index
            // `local_x(i)` folds to `GET i` — the interpreter's GETX peeks
            // a u16 index and REPLACES the stack top with the slot value,
            // which is exactly the GET value shape. Dynamic indices stay
            // compile rejects (the kernel frame is statically addressed).
            GETX => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: GETX node shape unexpected")?;
                let idx = local_x_slot_index(&*n.subx, "GETX")?;
                let mnil = self.nil_val(node);
                self.emit_get_slot(idx, mnil)?;
            }
            // T11: constant-index `local_x_put(i, v)` folds to `PUT i v`.
            // PUTX is never the frame prelude (the parser emits plain PUT),
            // so an input-slot target is always the read-only reject.
            PUTX => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: PUTX node shape unexpected")?;
                let idx = local_x_slot_index(&*n.subx, "PUTX")?;
                self.emit_put_slot(idx, &*n.suby, false)?;
            }
            PUT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: PUT node shape unexpected")?;
                self.emit_put_slot(n.para, &*n.subx, true)?;
            }
            XOP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: XOP node shape unexpected")?;
                let (lxop, idx) = decode_local_operand_mark(n.para);
                if matches!(self.slot.get(idx as usize), Some(SlotTy::Input)) {
                    // GT-c: asserted core substring "input parameter cannot be assigned"
                        // stays contiguous (rv64_gen_corpus.rs P2 message pin);
                        // the hint appends after it
                        return errf!(
                            "rv64 lower: the input parameter cannot be assigned; the call-data \
                             argument is read-only — copy it into a local first \
                             (e.g. `var t = data`) and assign to that"
                        );
                }
                // T17: the old value is a Compo — the interpreter's
                // local_binop extract faults CastFail(71) AFTER the RHS
                // evaluated (plan() admits only an int RHS, whose own
                // faults fire first, exactly like the interpreter).
                if matches!(self.slot.get(idx as usize), Some(SlotTy::List)) {
                    self.eval_int(&*n.subx)?;
                    for _ in 0..Self::entries_of_width(self.int_width(&*n.subx)) {
                        self.pop_vreg();
                    }
                    self.trap(71);
                    return Ok(());
                }
                // task-1 A3: the compound result keeps the slot/rhs common
                // width (the interpreter widens `slot OP= rhs` the same way)
                let w = self.arith_w(node);
                // BUG-012: the OLD value's Nil check runs before the op
                // (local_operand converts the old value first — 71)
                let xop_old_nil = self
                    .nil_occ
                    .get(&node_addr(node))
                    .map(|o| o.xop_old)
                    .unwrap_or(false);
                self.emit_xop_stmt(idx, lxop, &*n.subx, w, xop_old_nil)?;
            }
            /* ---- item 10: INC/DEC — `increase(n, x)` / `decrease(n, x)` ---
             * The interpreter (execute.rs INC/DEC → instruction.rs
             * unary_inc/dec → operand.rs Value::inc/dec) increments/
             * decrements the STACK-TOP COPY in place; the source slot is
             * NEVER written back (probed: `x=5; increase(1,x)` leaves x=5).
             * Faces, in interpreter order: any fault inside the operand's
             * own evaluation, then the is_uint check — a Nil operand faults
             * Arithmetic(90) here, NOT the XOP old-value CastFail(71) —
             * then the checked add/sub overflow (90, no wraparound). The
             * result keeps the operand's own width (u8 250+5 → 255, probed).
             */
            INC | DEC => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: INC/DEC node shape unexpected")?;
                let k = n.para as u64;
                // a definite static width is required for the checked form;
                // bool-ish/widthless flows are the D5 family boundary (the
                // interpreter's is_uint check would fault 90 at runtime)
                let wr = self.int_width(node);
                if !matches!(wr, Some(w) if w >= 8) {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel \
                         subset (the INC/DEC operand must be a definite uint value)"
                    );
                }
                let w = wr.unwrap();
                let old_nil = self.nil_val(&*n.subx);
                self.eval_int(&*n.subx)?;
                if old_nil {
                    // the tag rides the value's lowest entry (a u128 pair
                    // carries no tag on its high half — XOP arm precedent)
                    let low = if w == 128 { 2 } else { 1 };
                    self.nil_guard_tag(self.vstack.len() - low, 90);
                }
                if k == 0 {
                    // inc/dec by 0: every definite uint is unchanged
                    return Ok(());
                }
                if w == 128 {
                    // (lo, hi) pair: own BOTH halves (a banked slot's GET
                    // pushes Refs — the in-place update must not pollute the
                    // bank register), add/sub with the carry/borrow chain,
                    // overflow → 90 exactly at the checked-op boundary.
                    let hi = self.pop_own_replace();
                    let lo = self.pop_own_replace();
                    let t = self.push_vreg();
                    let c = self.push_vreg();
                    match op {
                        Bytecode::INC => {
                            self.asm.li(t, k);
                            self.asm.add(lo, lo, t);
                            // carry: lo_new < k (k > 0; wraps only on carry)
                            self.asm.r(0, t, lo, 2, c, 0b0110011); // sltu
                            // hi overflow ⟺ hi_old + carry == 0
                            self.asm.add(t, hi, c);
                            self.asm.beq(t, ZERO, self.fault_arith);
                            self.asm.add(hi, hi, c);
                        }
                        _ => {
                            self.asm.li(t, k);
                            self.asm.r(0, t, lo, 2, c, 0b0110011); // sltu c = lo < k
                            self.asm.sub(lo, lo, t);
                            // hi underflow ⟺ hi_old < borrow
                            self.asm.r(0, c, hi, 2, t, 0b0110011); // sltu t = hi < c
                            self.asm.bne(t, ZERO, self.fault_arith);
                            self.asm.sub(hi, hi, c);
                        }
                    }
                    let _ = self.pop_vreg(); // c
                    let _ = self.pop_vreg(); // t
                    return Ok(());
                }
                // single-register widths: own the top (pop_own_replace — a
                // Ref names the slot's bank register; in-place update on it
                // would pollute the slot the interpreter never touches)
                let d = self.pop_own_replace();
                match op {
                    Bytecode::INC => {
                        // w < 64: the u64 add cannot wrap (d < 2^w, k ≤ 255)
                        // — the width range check IS the overflow face.
                        // w == 64: overflow ⟺ d_new < k.
                        self.asm.addi(d, d, k as i32);
                        if w < 64 {
                            self.emit_range_check(d, w);
                        } else {
                            let t = self.push_vreg();
                            self.asm.li(t, k);
                            self.asm.r(0, t, d, 2, t, 0b0110011); // sltu
                            self.asm.bne(t, ZERO, self.fault_arith);
                            let _ = self.pop_vreg();
                        }
                    }
                    _ => {
                        // underflow ⟺ d < k, exact at every width; d ≥ k
                        // keeps the result inside the width domain
                        let t = self.push_vreg();
                        self.asm.li(t, k);
                        self.asm.r(0, t, d, 2, t, 0b0110011); // sltu
                        self.asm.bne(t, ZERO, self.fault_arith);
                        let _ = self.pop_vreg();
                        self.asm.addi(d, d, -(k as i32));
                    }
                }
            }
            /* ---- binary ints (checked) ---- */
            ADD | SUB | MUL | DIV | MOD | MAX | MIN | ABSDIFF | DIVUP | DIVEXACT | BSHL
            | BSHR | BXOR | BOR | BAND | EQ | NEQ | LT | GT | LE | GE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: binary node shape unexpected")?;
                // T17: EQ/NEQ over list values is the interpreter's
                // `value_content_eq` (ptr/structural equality); ordered
                // compares on a Compo fault Arithmetic(90) before touching
                // either value. Both admitted forms are plain list GETs, so
                // no operand needs an evaluation replay.
                if matches!(op, EQ | NEQ) {
                    if let (Some(sx), Some(sy)) = (
                        list_slot_get(&*n.subx, self.slot),
                        list_slot_get(&*n.suby, self.slot),
                    ) {
                        return self.emit_list_eq(op, sx, sy);
                    }
                    // fall through: the int/bytes paths below name a
                    // non-list operand (plan() keeps a mixed list compare
                    // a compile reject)
                }
                if matches!(op, LT | GT | LE | GE) {
                    if list_slot_get(&*n.subx, self.slot).is_some()
                        && list_slot_get(&*n.suby, self.slot).is_some()
                    {
                        // the interpreter faults Arithmetic(90) — the
                        // unconditional trap; the dead value-stack shape
                        // stays one int entry for downstream dead code
                        self.trap(90);
                        let _ = self.push_vreg();
                        return Ok(());
                    }
                }
                // T13: EQ/NEQ on bytes/address operands is the
                // `value_content_eq` face — byte equality (Address shares
                // the 21B-Bytes value domain; Address vs a non-21 Bytes
                // faults Arithmetic 90). Ordered compares on bytes fault
                // 90 on the interpreter (`lgc_ord_cmp` is uint-only) — the
                // int path below rejects them (`expected an int value`).
                if matches!(op, EQ | NEQ) {
                    let tx = node_res_ty(&*n.subx, self.slot)?;
                    let ty_ = node_res_ty(&*n.suby, self.slot)?;
                    let bx = matches!(tx, Ty::Bytes | Ty::Address);
                    let by = matches!(ty_, Ty::Bytes | Ty::Address);
                    if bx || by {
                        if !(bx && by) {
                            // int vs bytes/address: the interpreter faults
                            // 90 deterministically — compile-time fail-fast
                            return errf!(
                                "rv64 lower: {} is outside the translatable kernel \
                                 subset (cannot compare {} with {} — the interpreter \
                                 faults Arithmetic 90)",
                                if op == EQ { "EQ" } else { "NEQ" },
                                ty_name(tx),
                                ty_name(ty_)
                            );
                        }
                        let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                        self.eval_bytes(&*n.subx)?;
                        self.eval_bytes(&*n.suby)?;
                        // exactly-one-Nil faults 90; two Nils compare equal
                        // (their zero-initialized descriptors are both
                        // empty — len 0 == len 0, no differing bytes)
                        let n2 = self.vstack.len();
                        match (mx, my) {
                            (true, true) => self.nil_guard_eq_tags(n2 - 4, n2 - 2),
                            (true, false) => self.nil_guard_tag(n2 - 4, 90),
                            (false, true) => self.nil_guard_tag(n2 - 2, 90),
                            _ => {}
                        }
                        return self.emit_bytes_eq(op, tx == Ty::Address, ty_ == Ty::Address);
                    }
                }
                // task-1 A3: the interpreter computes the pair at the
                // operands' common width — the checked forms below emit
                // that width's overflow face
                let w = self.arith_w(node);
                // task 3: a 128-bit operand routes to the (lo, hi) pair
                // path. The node width alone is NOT enough: a comparison of
                // two u128 values is pure-bool (width 1) but still takes the
                // pair path; mixed-width comparisons stay numeric.
                let wx128 = self.int_width(&*n.subx) == Some(128);
                let wy128 = self.int_width(&*n.suby) == Some(128);
                if w == 128 || wx128 || wy128 {
                    return self.emit_bin128(op, n);
                }
                // constant-rhs absorption (`x / 16` → srli, `px >> 1` →
                // c.srli, `x % 16` → c.andi, checked guards folded away).
                // ADD/MUL also accept the constant on the left.
                if let Some(k) = int_literal(&*n.suby).filter(|k| Self::arith_k_handles(op, *k)) {
                    let lx = self.nil_val(&*n.subx);
                    self.eval_int(&*n.subx)?;
                    if lx {
                        // Nil operand → CastFail(71) before the op
                        self.nil_guard_tag(self.vstack.len() - 1, 71);
                    }
                    let d = self.pop_own_replace();
                    if !self.emit_arith_k(op, d, k, w) {
                        unreachable!("arith_k_handles said yes");
                    }
                    return Ok(());
                }
                if matches!(op, ADD | MUL) {
                    if let Some(k) =
                        int_literal(&*n.subx).filter(|k| Self::arith_k_handles(op, *k))
                    {
                        let ly = self.nil_val(&*n.suby);
                        self.eval_int(&*n.suby)?;
                        if ly {
                            self.nil_guard_tag(self.vstack.len() - 1, 71);
                        }
                        let d = self.pop_own_replace();
                        if !self.emit_arith_k(op, d, k, w) {
                            unreachable!("arith_k_handles said yes");
                        }
                        return Ok(());
                    }
                }
                let lx = self.nil_val(&*n.subx);
                let ly = self.nil_val(&*n.suby);
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                if lx || ly {
                    self.nil_guard_int_pair(op, lx, ly);
                }
                let (d, _ra, rb) = self.pop2_dest();
                self.emit_arith(op, d, rb, w)?;
            }
            POW => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: pow node shape unexpected")?;
                // task-1 A3: the interpreter's checked_pow faults when the
                // narrow-width result leaves the common width — a final
                // range check matches it exactly (for base >= 2 the partial
                // powers only grow; bases 0/1 cannot leave the range)
                let w = self.arith_w(node);
                // T6: a u128 operand routes to the (lo, hi) pair path (same
                // condition as the binary int arm — the node width alone
                // misses a u128 base with a widthless exponent mix)
                let px128 = self.int_width(&*n.subx) == Some(128);
                let py128 = self.int_width(&*n.suby) == Some(128);
                if w == 128 || px128 || py128 {
                    return self.emit_bin128(op, n);
                }
                // Constant small exponent → unrolled checked muls (the
                // common `x ** 2` shape: 3 insts instead of a ~10-inst loop).
                let const_exp = crate::lang::ir_literal_value(&*n.suby)
                    .ok()
                    .and_then(|v| v)
                    .and_then(|v| v.extract_u128().ok())
                    .filter(|&e| e <= 64);
                if let Some(e) = const_exp {
                    let mb = self.nil_val(&*n.subx);
                    self.eval_int(&*n.subx)?; // base
                    if mb {
                        self.nil_guard_tag(self.vstack.len() - 1, 71);
                    }
                    let d = self.pop_own_replace();
                    match e {
                        0 => self.asm.addi(d, ZERO, 1),
                        1 => {}
                        _ => {
                            self.asm.mv(self.a0, d); // preserved base
                            for _ in 1..e {
                                self.asm.r(1, self.a0, d, 3, self.a1, 0b0110011); // mulhu
                                self.asm.bne(self.a1, ZERO, self.fault_arith);
                                self.asm.r(1, self.a0, d, 0, d, 0b0110011); // mul
                            }
                        }
                    }
                    if w < 64 {
                        self.emit_range_check(d, w);
                    }
                } else {
                    let (mb, me) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                    self.eval_int(&*n.subx)?; // base
                    self.eval_int(&*n.suby)?; // exp
                    // BUG-021 family hardening (main-agent re-review: the
                    // bounds check must PRECEDE the guard-index arithmetic —
                    // deferred depth_err pushes can leave the stack short,
                    // and a wrapped base would emit a guard reading a wrong
                    // frame byte). Same checked_sub form as CLAMP.
                    let pow_base = match self.vstack.len().checked_sub(2) {
                        Some(b) => b,
                        None => {
                            return errf!(
                                "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                            );
                        }
                    };
                    if mb {
                        self.nil_guard_tag(pow_base, 71);
                    }
                    if me {
                        self.nil_guard_tag(pow_base + 1, 71);
                    }
                    let rb = self.pop_reg();
                    let la = self.pop_vreg();
                    let d = self.push_vreg();
                    // the interpreter's `exp_u32` face (instruction.rs
                    // pow_checked): an exponent above u32::MAX faults
                    // Arithmetic 90 BEFORE any multiplication — base 0/1
                    // included (three-way g1_pow_exp_u32_divergence). Only
                    // reachable at width 64 (narrower exponents cannot
                    // leave u32); the u128 path has the same pre-check in
                    // emit_u128_pow. a1 is free here — the loop below
                    // starts using it only after the branch.
                    if w == 64 {
                        self.asm.li(self.a1, 0x1_0000_0000);
                        self.asm.bgeu(rb, self.a1, self.fault_arith);
                    }
                    self.asm.mv(self.a0, loc_reg(la)); // preserve the base
                    self.asm.addi(d, ZERO, 1); // result
                    let l_loop = self.asm.new_label();
                    let l_end = self.asm.new_label();
                    self.place(l_loop);
                    self.asm.beq(rb, ZERO, l_end);
                    self.asm.r(1, self.a0, d, 3, self.a1, 0b0110011); // mulhu a1, d, base
                    self.asm.bne(self.a1, ZERO, self.fault_arith);
                    self.asm.r(1, self.a0, d, 0, d, 0b0110011); // mul d, d, base
                    self.asm.addi(rb, rb, -1);
                    self.asm.jal_(ZERO, l_loop);
                    self.place(l_end);
                    if w < 64 {
                        self.emit_range_check(d, w);
                    }
                }
            }
            CLAMP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: clamp node shape unexpected")?;
                if self.int_width(&*n.subx) == Some(128)
                    || self.int_width(&*n.suby) == Some(128)
                    || self.int_width(&*n.subz) == Some(128)
                {
                    // T6: a u128 X takes the pair path (the interpreter's
                    // arithmetic_args3 widens all three to the common width
                    // — x itself 128 — and the result keeps that width, so
                    // the static CLAMP width (= x's) stays consistent). A
                    // narrow x with u128 bounds would produce a u128 result
                    // the width model cannot represent — still a reject.
                    if self.int_width(&*n.subx) == Some(128) {
                        return self.emit_clamp128(n);
                    }
                    return errf!(
                        "rv64 lower: CLAMP is outside the translatable kernel subset at u128 \
                         width (a u128 bound with a narrower value — pin the value with `as \
                         u128` so the result width is uniform)"
                    );
                }
                self.eval_int(&*n.subx)?; // x
                self.eval_int(&*n.suby)?; // lo
                self.eval_int(&*n.subz)?; // hi
                // Nil operands → CastFail(71) (the interpreter converts
                // all three args before clamp_checked), x → lo → hi
                let (mx, mlo, mhi) = (
                    self.nil_val(&*n.subx),
                    self.nil_val(&*n.suby),
                    self.nil_val(&*n.subz),
                );
                // BUG-021 family hardening: under a shrunken pool the
                // deferred depth_err pushes leave the vstack short — bail
                // with the depth reject the campaign retry keys on (the
                // raw subtraction underflows in debug and wraps into a
                // wild tag offset in release)
                let base = match self.vstack.len().checked_sub(3) {
                    Some(b) => b,
                    None => {
                        return errf!(
                            "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                        );
                    }
                };
                if mx {
                    self.nil_guard_tag(base, 71);
                }
                if mlo {
                    self.nil_guard_tag(base + 1, 71);
                }
                if mhi {
                    self.nil_guard_tag(base + 2, 71);
                }
                let hi = self.pop_reg();
                let lo = self.pop_reg();
                let x = self.pop_vreg();
                let d = self.push_vreg();
                if let VLoc::Ref(r) = x {
                    if d != r {
                        self.asm.mv(d, r);
                    }
                }
                // clamp_checked rejects lo > hi BEFORE clamping
                // (interpreter Arithmetic 90, instruction.rs clamp_checked);
                // the unchecked min/max chain alone silently returned lo —
                // BUG-010. hi < lo <=> lo > hi.
                self.asm.bltu(hi, lo, self.fault_arith);
                self.emit_min(d, hi); // d = min(x, hi)
                self.emit_max(d, lo); // d = max(d, lo)
            }
            /* ---- kehuobu 8: mul_div / mul_div_up ---- */
            //
            // Verified subset: widths <= 32 (x·y < 2^64 → one mul + divu /
            // remu, quotient range-checked back to the widened width). The
            // u64 and u128 widths (T4) lower through the u256 library: the
            // 256-bit product ÷ the shared 256/128 soft-division routine.
            MULDIV | MULDIVUP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: mul_div node shape unexpected")?;
                let w = self.triop_arith_width(n, op)?;
                if w > 32 {
                    // T4: the u64 / u128 common widths. Interpreter ground
                    // truth (execute.rs MULDIV/MULDIVUP → triop_arithmetic →
                    // arithmetic_args3 + instruction.rs
                    // muldiv_with_round_checked): the three to_uint operands
                    // fault CastFail 71 on a non-uint (Nil) value in x, y, z
                    // order, zero-extend to the three-way max width, then
                    // z == 0 → Arithmetic 90, the 256-bit product ÷ z must
                    // give a u128 quotient (hi >= z → 90), the ceil form
                    // bumps a nonzero remainder (carry out of u128 → 90),
                    // and the quotient casts back to the common width (the
                    // first operand's width AFTER the arithmetic_args3
                    // widening; → 90). Splitting into two statements is NOT
                    // the same semantics — the wide intermediate is
                    // interpreter behavior.
                    let (wx, wy, wz) = (
                        self.int_width(&*n.subx),
                        self.int_width(&*n.suby),
                        self.int_width(&*n.subz),
                    );
                    let (lx, ly, lz) = (
                        self.nil_val(&*n.subx),
                        self.nil_val(&*n.suby),
                        self.nil_val(&*n.subz),
                    );
                    self.eval_int(&*n.subx)?;
                    self.eval_int(&*n.suby)?;
                    self.eval_int(&*n.subz)?;
                    let (ex, ey, ez) = (
                        Self::entries_of_width(wx),
                        Self::entries_of_width(wy),
                        Self::entries_of_width(wz),
                    );
                    let base = self.vstack.len() - ex - ey - ez;
                    if lx {
                        self.nil_guard_tag(base, 71);
                    }
                    if ly {
                        self.nil_guard_tag(base + ex, 71);
                    }
                    if lz {
                        self.nil_guard_tag(base + ex + ey, 71);
                    }
                    self.emit_muldiv_wide(w, op == MULDIVUP, wx, wy, wz)?;
                } else {
                    let lx = self.nil_val(&*n.subx);
                    let ly = self.nil_val(&*n.suby);
                    let lz = self.nil_val(&*n.subz);
                    self.eval_int(&*n.subx)?;
                    self.eval_int(&*n.suby)?;
                    self.eval_int(&*n.subz)?;
                    let base = self.vstack.len() - 3;
                    if lx {
                        self.nil_guard_tag(base, 71);
                    }
                    if ly {
                        self.nil_guard_tag(base + 1, 71);
                    }
                    if lz {
                        self.nil_guard_tag(base + 2, 71);
                    }
                    self.emit_muldiv(w, op == MULDIVUP)?;
                }
            }
            /* ---- T2: add_mod / mul_mod at the u64 and u128 widths ---- */
            //
            // Interpreter ground truth (execute.rs ADDMOD/MULMOD →
            // triop_arithmetic → arithmetic_args3, then instruction.rs
            // addmod_checked/mulmod_checked): the three operands go through
            // to_uint (a non-uint/Nil operand faults CastFail 71, in x, y, z
            // order), are zero-extended to their three-way max width (never
            // fails), then z == 0 faults Arithmetic 90 and the modulus
            // result — (x+y) mod z / (x·y) mod z, computed overflow-free
            // (add_mod_u128's gap branch / mul_mod_u128's double-and-add) —
            // is cast back to that common width (always fits: < z). Widths
            // up to 64 lower inline on one register; the 128-bit common
            // width lowers through the (lo, hi) pair machinery.
            ADDMOD | MULMOD => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: add_mod node shape unexpected")?;
                let w = self.triop_arith_width(n, op)?;
                let (wx, wy, wz) = (
                    self.int_width(&*n.subx),
                    self.int_width(&*n.suby),
                    self.int_width(&*n.subz),
                );
                let (lx, ly, lz) = (
                    self.nil_val(&*n.subx),
                    self.nil_val(&*n.suby),
                    self.nil_val(&*n.subz),
                );
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                self.eval_int(&*n.subz)?;
                // Nil tags ride the LOW entry of each operand (an u128
                // operand is a two-entry pair), 71 = the to_uint face.
                let (ex, ey, ez) = (
                    Self::entries_of_width(wx),
                    Self::entries_of_width(wy),
                    Self::entries_of_width(wz),
                );
                let base = self.vstack.len() - ex - ey - ez;
                if lx {
                    self.nil_guard_tag(base, 71);
                }
                if ly {
                    self.nil_guard_tag(base + ex, 71);
                }
                if lz {
                    self.nil_guard_tag(base + ex + ey, 71);
                }
                if w == 128 {
                    let (zlo, zhi) = self.pop_u128_pair(wz);
                    let (ylo, yhi) = self.pop_u128_pair(wy);
                    let (xlo, xhi) = self.pop_u128_pair(wx);
                    if op == ADDMOD {
                        self.emit_addmod128(xlo, xhi, ylo, yhi, zlo, zhi)?;
                    } else {
                        self.emit_mulmod128(xlo, xhi, ylo, yhi, zlo, zhi)?;
                    }
                } else {
                    let z = self.pop_reg();
                    let y = self.pop_reg();
                    let x = self.pop_reg();
                    if op == ADDMOD {
                        self.emit_addmod64(x, y, z);
                    } else {
                        self.emit_mulmod64(x, y, z)?;
                    }
                }
            }
            /* ---- T3: mul_add / mul_sub at every width up to 128 ---- */
            //
            // Interpreter ground truth (execute.rs MULADD/MULSUB →
            // triop_arithmetic → arithmetic_args3 + instruction.rs
            // muladd_checked/mulsub_checked → mul_xy_addsub_z_fit_u128): the
            // three to_uint operands fault CastFail 71 on a non-uint (Nil)
            // value in x, y, z order, zero-extend to the three-way max
            // width, then the FULL 256-bit intermediate is observable: n =
            // x·y (u256), n ±= z at 256 bits (the sub's underflow → 90), the
            // result must clear bits 128..255 (else 90) and casts back to
            // the common width (overflow → 90). Lowered through the u256
            // library at EVERY width — the interpreter never narrows the
            // intermediate, and splitting into two statements changes the
            // observable width faces.
            MULADD | MULSUB => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: mul_add node shape unexpected")?;
                let w = self.triop_arith_width(n, op)?;
                let (wx, wy, wz) = (
                    self.int_width(&*n.subx),
                    self.int_width(&*n.suby),
                    self.int_width(&*n.subz),
                );
                let (lx, ly, lz) = (
                    self.nil_val(&*n.subx),
                    self.nil_val(&*n.suby),
                    self.nil_val(&*n.subz),
                );
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                self.eval_int(&*n.subz)?;
                let (ex, ey, ez) = (
                    Self::entries_of_width(wx),
                    Self::entries_of_width(wy),
                    Self::entries_of_width(wz),
                );
                let base = self.vstack.len() - ex - ey - ez;
                if lx {
                    self.nil_guard_tag(base, 71);
                }
                if ly {
                    self.nil_guard_tag(base + ex, 71);
                }
                if lz {
                    self.nil_guard_tag(base + ex + ey, 71);
                }
                self.emit_muladdsub(w, op == MULADD, wx, wy, wz)?;
            }
            /* ---- T26: the FIN* family (fitsh calls them by registry
             * name — `mul_div_floor(x, y, z)` etc.; see rt/fin.rs) ---- */
            FIN2 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Double>()
                    .ok_or("rv64 lower: fin2 node shape unexpected")?;
                self.emit_fin2(n)?;
            }
            FIN3 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Triple>()
                    .ok_or("rv64 lower: fin3 node shape unexpected")?;
                self.emit_fin3(n)?;
            }
            FIN4 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Quad>()
                    .ok_or("rv64 lower: fin4 node shape unexpected")?;
                self.emit_fin4(n)?;
            }
            FINP3 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Triple>()
                    .ok_or("rv64 lower: finp3 node shape unexpected")?;
                self.emit_finp3(n)?;
            }
            FINP4 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Quad>()
                    .ok_or("rv64 lower: finp4 node shape unexpected")?;
                self.emit_finp4(n)?;
            }
            FINPOW3 => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Triple>()
                    .ok_or("rv64 lower: finpow3 node shape unexpected")?;
                self.emit_finpow3(n)?;
            }
            /* ---- unary / logical ---- */
            NOT => {
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: not node shape unexpected")?;
                self.eval_int(&*sub.subx)?;
                let r = self.pop_own_replace();
                self.asm.i(1, r, 3, r, 0b0010011); // sltiu r, r, 1
            }
            AND | OR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: logic node shape unexpected")?;
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                let (d, _ra, rb) = self.pop2_dest();
                if op == AND {
                    self.asm.and_(d, d, rb);
                } else {
                    self.asm.or_(d, d, rb);
                }
            }
            /* ---- bytes ---- */
            SIZE => {
                let sub = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: size node shape unexpected")?;
                if let Some(b) = bytes_literal(&*sub.subx) {
                    let r = self.push_vreg();
                    self.asm.li(r, b.len() as u64);
                } else {
                    self.eval_bytes(&*sub.subx)?;
                    let rl = self.pop_reg();
                    let lp = self.pop_vreg();
                    let d = self.push_vreg();
                    if let VLoc::Ref(r) = lp {
                        let _ = r; // ptr dead: result is the length
                    }
                    if d != rl {
                        self.asm.mv(d, rl);
                    }
                }
            }
            CAT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: cat node shape unexpected")?;
                // Fold at compile time: lit ++ lit → one pool literal;
                // x ++ "" / "" ++ x → x (zero copy, no arena traffic).
                let lx = bytes_literal(&*n.subx);
                let ly = bytes_literal(&*n.suby);
                if lx.as_ref().is_some_and(|a| a.is_empty()) {
                    // concat still extracts the OTHER side: a Nil operand
                    // faults BytesHandle(91) even against the empty fold
                    let m = self.nil_val(&*n.suby);
                    self.eval_bytes(&*n.suby)?;
                    if m {
                        self.nil_guard_tag(self.vstack.len() - 2, 91);
                    }
                } else if ly.as_ref().is_some_and(|b| b.is_empty()) {
                    let m = self.nil_val(&*n.subx);
                    self.eval_bytes(&*n.subx)?;
                    if m {
                        self.nil_guard_tag(self.vstack.len() - 2, 91);
                    }
                } else if let (Some(a), Some(b)) = (lx, ly) {
                    let mut cat = a;
                    cat.extend_from_slice(&b);
                    let lbl = self.lit_label(&cat);
                    let rp = self.push_vreg();
                    let rl = self.push_vreg();
                    self.lit_addr(rp, lbl);
                    self.asm.li(rl, cat.len() as u64);
                } else {
                    let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                    self.eval_bytes(&*n.subx)?;
                    self.eval_bytes(&*n.suby)?;
                    // concat extracts x first, then y — Nil → 91 each
                    if mx {
                        self.nil_guard_tag(self.vstack.len() - 4, 91);
                    }
                    if my {
                        self.nil_guard_tag(self.vstack.len() - 2, 91);
                    }
                    self.emit_cat()?;
                }
            }
            BYTE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: byte node shape unexpected")?;
                let (mx, my) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_bytes(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                // interpreter order: the index's u16 cast fires first
                // (72), then the bytes extract (91)
                if my {
                    self.nil_guard_tag(self.vstack.len() - 1, 72);
                }
                if mx {
                    self.nil_guard_tag(self.vstack.len() - 3, 91);
                }
                self.emit_byte_at()?;
            }
            CUT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: cut node shape unexpected")?;
                let (mx, my, mz) =
                    (self.nil_val(&*n.subx), self.nil_val(&*n.suby), self.nil_val(&*n.subz));
                self.eval_bytes(&*n.subx)?;
                self.eval_int(&*n.suby)?; // ost
                self.eval_int(&*n.subz)?; // len
                // interpreter order: len's u16 cast, ost's, then the
                // bytes extract
                if mz {
                    self.nil_guard_tag(self.vstack.len() - 1, 72);
                }
                if my {
                    self.nil_guard_tag(self.vstack.len() - 2, 72);
                }
                if mx {
                    self.nil_guard_tag(self.vstack.len() - 4, 91);
                }
                self.emit_slice(SliceKind::Cut)?;
            }
            LEFT | RIGHT | LDROP | RDROP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: slice node shape unexpected")?;
                let m = self.nil_val(&*n.subx);
                self.eval_bytes(&*n.subx)?;
                if m {
                    // count is a ≤255 literal: only the bytes side can be Nil
                    self.nil_guard_tag(self.vstack.len() - 2, 91);
                }
                let r = self.push_vreg();
                self.asm.addi(r, ZERO, n.para as i32);
                let kind = match op {
                    LEFT => SliceKind::Left,
                    RIGHT => SliceKind::Right,
                    LDROP => SliceKind::LDrop,
                    _ => SliceKind::RDrop,
                };
                self.emit_slice(kind)?;
            }
            /* ---- value-position control: blocks and choose ---- */
            IRLIST | IRBLOCK | IRBLOCKR => {
                let arr = node
                    .as_any()
                    .downcast_ref::<IRNodeArray>()
                    .ok_or("rv64 lower: block node shape unexpected")?;
                let keep_last = matches!(op, IRBLOCKR | IRLIST);
                let last = arr.subs.len().saturating_sub(1);
                for (i, sub) in arr.subs.iter().enumerate() {
                    let k = keep_last && i == last && sub.hasretval();
                    self.eval(&**sub)?;
                    if sub.hasretval() && !k {
                        // task 3: the pop count follows the value's ENTRY
                        // count — Bytes and u128 are two, everything else one
                        for _ in 0..self.value_entries(&**sub)? {
                            self.pop_vreg();
                        }
                    }
                }
            }
            IRIFR => {
                // value-position if: branch-LAZY on BOTH engines — the
                // bytecode channel compiles IRIFR through compile_if
                // (compile.rs compile_triple handles IRIF|IRIFR), so the
                // untaken arm never evaluates on either side. Both arms
                // evaluate into the SAME pool registers (the vstack hands
                // out by depth, and each arm starts at the same depth), so
                // at the join the stack is rebased to exactly one value.
                // The compile-time vstack accumulates both arms' pushes;
                // pop each arm's results and re-push the (identical)
                // locations.
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                let ty = node_res_ty(&*n.suby, self.slot)?;
                let vals = if matches!(ty, Ty::Bytes | Ty::Address) {
                    2
                } else {
                    Self::entries_of_width(self.int_width(&*n.suby))
                };
                // task 3: the arms must occupy the same number of vstack
                // entries, or the join re-push mismatches (BUG-017 family)
                let ty_no = node_res_ty(&*n.subz, self.slot)?;
                let vals_no = if matches!(ty_no, Ty::Bytes | Ty::Address) {
                    2
                } else {
                    Self::entries_of_width(self.int_width(&*n.subz))
                };
                if vals != vals_no {
                    // the pinned BUG-017 family text (the campaign classifier
                    // keys on it; IRIFR is the value-if of the same choose
                    // family)
                    return errf!(
                        "rv64 lower: choose arms must have the same result type \
                         (yes {vals} entries, no {vals_no})"
                    );
                }
                let l_else = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_else)?;
                self.eval(&*n.suby)?;
                // take (do NOT re-push): arm2 must evaluate at the same
                // stack level so its value lands in the same register
                let a_locs = self.take_arm_values(vals);
                // BUG-020: per-path tag materialization at the join. Both
                // arms evaluate at the SAME vstack depth and the join
                // re-push lands the value exactly where the taken arm's
                // own pushes sat — a may-Nil arm (bare GET / nested
                // choose / value-if / block tail: the only shapes
                // nil_plan marks may-Nil) already wrote its REAL tag
                // there at runtime, so there is nothing to copy (source
                // == result cell, unlike the CHOOSE select whose arms
                // sit at distinct depths). A MECHANISM-LESS arm (proven
                // non-Nil: a faulting op like `255 + t0`) wrote nothing,
                // leaving the prologue-zeroed cell — which reads as
                // "Nil" — under a consumer guard that fires whenever
                // my||mn. Each path therefore publishes non-Nil (1)
                // exactly when ITS arm is mechanism-less and the other
                // arm makes the join may-Nil; my=mn=false emits nothing
                // (no guard can read the tag — all-non-Nil census shapes
                // stay byte-identical).
                let my = self.nil_val(&*n.suby);
                let mn = self.nil_val(&*n.subz);
                if mn && !my {
                    let base = self.vstack.len();
                    for i in 0..vals {
                        self.nil_set_tag_one(base + i);
                    }
                }
                self.asm.jal_(ZERO, l_end);
                self.place(l_else);
                self.eval(&*n.subz)?;
                let b_locs = self.take_arm_values(vals);
                if my && !mn {
                    let base = self.vstack.len();
                    for i in 0..vals {
                        self.nil_set_tag_one(base + i);
                    }
                }
                self.place(l_end);
                // both arms now hold their value in the same pool
                // registers (refs were materialized inside their own arm
                // region); a future pool change cannot hide a divergence
                debug_assert_eq!(a_locs, b_locs, "value-if arms must share registers");
                // bottom-first re-push: the bytes pair goes back as
                // (ptr, len) — the old `.rev()` order swapped them (BUG-011)
                for l in a_locs.into_iter() {
                    self.span_open_at(self.vstack.len());
                    self.vstack.push(l);
                }
            }
            CHOOSE => {
                // ternary `? :` / choose(): EAGER — the bytecode channel
                // evaluates ALL THREE operands before selecting
                // (compile_triple lowers only IRIF|IRIFR lazily; CHOOSE
                // falls to the eager IRNodeTriple::codegen_into path, and
                // the interpreter's CHOOSE just picks among the three
                // already-evaluated stack values). Evaluate cond → yes →
                // no in the bytecode stack order, then branch-select into
                // the cond's positions: an arm's traps fire exactly where
                // the interpreter fires them (D9 fix).
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                // T17: list arms — the result is a container value with no
                // vstack representation; only the condition is evaluated
                // here (both arms are plain GETs, so the eager evaluation
                // order is unobservable). The PUT bind path selects the
                // descriptor; a dropped choose evaluates just the cond.
                if expr_is_list_valued(&*n.suby, self.slot)
                    || expr_is_list_valued(&*n.subz, self.slot)
                {
                    self.eval_int(&*n.subx)?;
                    for _ in 0..Self::entries_of_width(self.int_width(&*n.subx)) {
                        self.pop_vreg();
                    }
                    return Ok(());
                }
                let ty = node_res_ty(&*n.suby, self.slot)?;
                let ty_no = node_res_ty(&*n.subz, self.slot)?;
                // task 3: the arm ENTRY counts must match too (a u128 arm is
                // two entries) — a yes-u128/no-u64 choose would mis-select
                let vals = if matches!(ty, Ty::Bytes | Ty::Address) {
                    2
                } else {
                    Self::entries_of_width(self.int_width(&*n.suby))
                };
                let vals_no = if matches!(ty_no, Ty::Bytes | Ty::Address) {
                    2
                } else {
                    Self::entries_of_width(self.int_width(&*n.subz))
                };
                if ty != ty_no || vals != vals_no {
                    // BUG-017 (facet c): the arm value count (1 int / 2
                    // bytes) is read off the YES arm; mismatched arms made
                    // the cond index underflow the vstack (bytes-first
                    // panicked) or mis-pop the stack (int-first leaked one
                    // entry and rejected late with an unrelated text). The
                    // vstack is statically typed per position — reject the
                    // shape up front, cleanly.
                    return errf!(
                        "rv64 lower: choose arms must have the same result type (yes {:?}, no {:?})",
                        ty, ty_no
                    );
                }
                self.eval_int(&*n.subx)?; // cond
                self.eval(&*n.suby)?; // yes
                self.eval(&*n.subz)?; // no
                // BUG-021: the 8-value register pool may be exhausted
                // mid-arm — push_vreg's depth_err deferral then leaves the
                // vstack SHORT by design (emission continues so the final
                // reject carries one clean diagnostic), and this direct
                // index is the lowerer's only unchecked vstack read: on a
                // shrunk stack it must bail out with the depth_err reject
                // text instead of the release-profile wrapped-index OOB
                // panic that killed whole mass runs (the campaign's
                // "nesting exceeds" downsizing retry keys on the text).
                let cond_idx = self
                    .vstack
                    .len()
                    .checked_sub(1 + 2 * vals)
                    .and_then(|i| self.vstack.get(i).copied());
                let c_reg = match cond_idx {
                    Some(l) => loc_reg(l),
                    None => {
                        return errf!(
                            "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                        );
                    }
                };
                // Materialize each arm's values bottom-first into their own
                // consecutive pool positions (the BUG-011 take_arm_values
                // treatment). The selection arm kept the old LIFO pops and
                // indexed the reversed bytes (len, ptr) against the
                // bottom-first dests, so a bytes×bytes choose copied
                // (len, ptr) into (ptr, len) — the interpreter returned the
                // taken arm's bytes while all three rv64 modes swapped the
                // pair (BUG-017). The registers keep holding the values
                // after the take (the vstack pops are balanced), so the
                // select below still reads them.
                let no_locs = self.take_arm_values(vals);
                let yes_locs = self.take_arm_values(vals);
                let _ = self.pop_vreg(); // cond (dead after the select)
                let mut dests = Vec::with_capacity(vals);
                for _ in 0..vals {
                    dests.push(self.push_vreg());
                }
                // Nil tags ride along with the selection (BUG-012): the
                // result's tag = the taken arm's tag. Each path is gated on
                // ITS OWN arm's may-Nil verdict (BUG-019: the shared
                // `my || mn` gate made a mechanism-less arm's stale tag
                // byte — prologue zero reads as "Nil" — get copied over the
                // live result, so the consumer's guard invented a 71). A
                // mechanism-less arm (proven non-Nil ⇒ its evaluation never
                // writes tag bytes) publishes a non-Nil tag (1) instead:
                // whenever my||mn the consumer's guard WILL read the dest
                // tag, so the byte must be valid on every selectable path.
                // With my=mn=false nothing is emitted (no guard can read
                // the tag — byte-identical to the all-non-Nil census
                // shapes).
                let entry = self.vstack.len() - vals;
                let my = self.nil_val(&*n.suby);
                let mn = self.nil_val(&*n.subz);
                let l_yes = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.asm.bne(c_reg, ZERO, l_yes);
                for (i, &d) in dests.iter().enumerate() {
                    let s = loc_reg(no_locs[i]);
                    if d != s {
                        self.asm.mv(d, s);
                    }
                }
                if my || mn {
                    for i in 0..vals {
                        if mn {
                            self.nil_copy_tag(entry + 1 + vals + i, entry + i);
                        } else {
                            self.nil_set_tag_one(entry + i);
                        }
                    }
                }
                self.asm.jal_(ZERO, l_end);
                self.place(l_yes);
                for (i, &d) in dests.iter().enumerate() {
                    let s = loc_reg(yes_locs[i]);
                    if d != s {
                        self.asm.mv(d, s);
                    }
                }
                if my || mn {
                    for i in 0..vals {
                        if my {
                            self.nil_copy_tag(entry + 1 + i, entry + i);
                        } else {
                            self.nil_set_tag_one(entry + i);
                        }
                    }
                }
                self.place(l_end);
            }
            /* ---- P3-V2IMPL: the admitted list consumers ---- */
            ITEMGET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: itemget node shape unexpected")?;
                // T16: map receiver / Bytes-element list receiver.
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    return self.emit_map_itemget(n);
                }
                let s = self.list_any_slot_of(&*n.subx, op)?;
                if matches!(self.slot.get(s), Some(SlotTy::ListBytes)) {
                    return self.emit_list_bytes_itemget(&*n.suby, s);
                }
                let kind = self.lists.kind[s];
                let static_len = match kind {
                    ListKind::Empty => Some(0u64),
                    ListKind::Rodata { n } | ListKind::FrameStatic { n } => Some(n),
                    ListKind::Growable { .. } | ListKind::Arena { .. } => None,
                    ListKind::NotList => {
                        return errf!(
                            "rv64 lower: opcode ITEMGET is outside the translatable kernel subset"
                        );
                    }
                };
                // constant index: in-range loads are guard-free (the frame/
                // table offsets are compile-time); out-of-range (including
                // the u32 index space) is a B-family compile reject — the
                // depth-9 / literal-cast-overflow fail-fast precedent
                let key_lit = crate::lang::ir_literal_value(&*n.suby)
                    .ok()
                    .flatten()
                    .and_then(|v| v.extract_u128().ok());
                if let Some(k) = key_lit {
                    if k >= 1u128 << 32 {
                        return errf!(
                            "rv64 lower: ITEMGET constant index {k} exceeds the u32 index space"
                        );
                    }
                    // Static-length classes (Empty/Rodata/FrameStatic) prove
                    // the bounds at compile time: in-range loads stay
                    // guard-free, out-of-range rejects above.
                    if let Some(sl) = static_len {
                        if k >= sl as u128 {
                            return errf!(
                                "rv64 lower: ITEMGET constant index {k} is out of bounds \
                                 for a list of length {sl}"
                            );
                        }
                    }
                    let step = self.list_step(s);
                    // T19: a u128 element loads as a (lo, hi) pair — two
                    // fresh pool registers, low half first
                    let r = self.push_vreg();
                    let r2 = if step == 16 { Some(self.push_vreg()) } else { None };
                    // B2 task 4: the element-count capacity is the block
                    // budget divided by the element stride
                    let capacity = (LIST_BLOCK_MAX / step as u64) as u128;
                    match kind {
                        ListKind::Rodata { .. } => {
                            let lbl = self.list_rodata_lbl(s)?;
                            self.lit_addr(self.asm.sc1, lbl);
                            // k * step < capacity * step = LIST_BLOCK_MAX
                            // fits the imm12 window
                            let imm = (k as i32) * (step as i32);
                            match r2 {
                                Some(hi) => self.list_load_pair(r, hi, self.asm.sc1, imm),
                                None => self.list_load_inst(r, self.asm.sc1, imm, step),
                            }
                            let _ = capacity;
                        }
                        // Growable: the length is a RUNTIME value — the
                        // constant index needs the live `k < len` guard
                        // (BUG-028: the guard-less load read stale frame
                        // bytes past the list end and silently swallowed
                        // the interpreter's CompoNoFindItem(85) trap).
                        ListKind::Growable { .. } => {
                            let l85 = self.nil_thunk(85);
                            if k >= capacity {
                                // the block-budget invariant keeps
                                // len ≤ capacity, so any k ≥ capacity faults
                                // unconditionally — no load
                                self.asm.jal_(ZERO, l85);
                            } else {
                                let sc1 = self.asm.sc1;
                                let sc2 = self.asm.sc2;
                                self.asm.li(sc2, k as u64); // li temp = sc1 (free here)
                                let cell = self.slot_offset(s as u8);
                                self.asm.ld(sc1, S0, cell); // len
                                self.asm.bgeu(sc2, sc1, l85); // k >= len → 85
                                let off = self.list_data_off[s] + (k as i32) * (step as i32);
                                match r2 {
                                    Some(hi) => self.list_load_pair(r, hi, S0, off),
                                    None => self.list_load_inst(r, S0, off, step),
                                }
                            }
                        }
                        // P2: the arena object's length is a runtime value
                        // behind the descriptor — same live guard, then
                        // the tagged-cell payload load (the 24B stride
                        // offset exceeds imm12, so the address is
                        // materialized)
                        ListKind::Arena { .. } => {
                            let l85 = self.nil_thunk(85);
                            if k >= CTR_LIST_CAP as u128 {
                                self.asm.jal_(ZERO, l85);
                            } else {
                                let sc1 = self.asm.sc1;
                                let sc2 = self.asm.sc2;
                                // a0, not a1: ImcRemap maps a1 and sc2 to
                                // the same x31, and sc2 holds k here
                                let a0 = self.a0;
                                self.asm.li(sc2, k as u64);
                                let cell = self.slot_offset(s as u8);
                                self.asm.ld(sc1, S0, cell); // obj
                                self.asm.ld(a0, sc1, 0); // len
                                self.asm.bgeu(sc2, a0, l85); // k >= len → 85
                                let off = CTR_LIST_HDR as i32 + (k as i32) * CTR_ELEM_BYTES as i32;
                                self.asm.li(a0, off as u64);
                                self.asm.add(sc1, sc1, a0);
                                self.ctr_load_elem(r, r2, sc1);
                            }
                        }
                        _ => {
                            let off = self.list_data_off[s] + (k as i32) * (step as i32);
                            match r2 {
                                Some(hi) => self.list_load_pair(r, hi, S0, off),
                                None => self.list_load_inst(r, S0, off, step),
                            }
                        }
                    }
                    return Ok(());
                }
                // variable index: Nil → 72 (extract_u32), ≥ 2³² → 72,
                // ≥ len → 85 — in exactly that order (list.rs:36-40 reads
                // through extract_u32 first)
                self.eval_int(&*n.suby)?;
                let kd = self.vstack.len() - 1;
                if self.nil_val(&*n.suby) {
                    self.nil_guard_tag(kd, 72);
                }
                let kr = loc_reg(self.vstack[kd]);
                // P2: an arena slot's guards read the length through the
                // descriptor (the shared header word)
                if matches!(kind, ListKind::Arena { .. }) {
                    self.emit_list_bounds_guards_arena(kr, s)?;
                } else {
                    self.emit_list_bounds_guards(kr, s, static_len)?;
                }
                if static_len != Some(0) {
                    // (an Empty list's always-85 guard exits above; there
                    // is no element block to load from) The result MUST go
                    // to a FRESH pool register: the key may have been
                    // pushed as a bank Ref (ImcRemap promoted locals), and
                    // overwriting its home register in place would clobber
                    // the promoted slot. T19: a u128 element loads as a
                    // (lo, hi) pair — two fresh registers, low half first.
                    let rodata = match kind {
                        ListKind::Rodata { .. } => Some(self.list_rodata_lbl(s)?),
                        _ => None,
                    };
                    self.pop_vreg();
                    let r = self.push_vreg();
                    let r2 = if self.list_step(s) == 16 {
                        Some(self.push_vreg())
                    } else {
                        None
                    };
                    if matches!(kind, ListKind::Arena { .. }) {
                        self.emit_list_load_arena(r, r2, kr, s)?;
                    } else {
                        self.emit_list_load(r, r2, kr, s, rodata)?;
                    }
                }
            }
            LENGTH => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: length node shape unexpected")?;
                // T16: map length reads the object header.
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    return self.emit_map_length(n);
                }
                let s = self.list_any_slot_of(&*n.subx, op)?;
                // folded to the u64-width shadow / constant (the pinned
                // D5/D7 width family: `as bytes` yields 8B, the interpreter
                // U32 4B — pinned divergence rows, not failures)
                let r = self.push_vreg();
                match self.lists.kind[s] {
                    ListKind::Empty => self.asm.addi(r, ZERO, 0),
                    ListKind::Rodata { n } | ListKind::FrameStatic { n } => {
                        self.asm.addi(r, ZERO, n as i32)
                    }
                    ListKind::Growable { .. } => {
                        let cell = self.slot_offset(s as u8);
                        self.asm.ld(r, S0, cell);
                    }
                    // P2: the cell holds the descriptor — the length is
                    // the shared object's header word
                    ListKind::Arena { .. } => {
                        let cell = self.slot_offset(s as u8);
                        self.asm.ld(r, S0, cell);
                        self.asm.ld(r, r, 0);
                    }
                    ListKind::NotList => {
                        return errf!(
                            "rv64 lower: opcode LENGTH is outside the translatable kernel subset"
                        );
                    }
                }
            }
            HASKEY => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: haskey node shape unexpected")?;
                // T16: map receiver.
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    return self.emit_map_haskey(n);
                }
                let s = self.list_any_slot_of(&*n.subx, op)?;
                // key extract_u32 first (Nil → 72, ≥ 2³² → 72 — even
                // though an out-of-bounds key merely folds to false,
                // list.rs:30-33 runs through extract_u32)
                self.eval_int(&*n.suby)?;
                let kd = self.vstack.len() - 1;
                if self.nil_val(&*n.suby) {
                    self.nil_guard_tag(kd, 72);
                }
                let kr = loc_reg(self.vstack[kd]);
                let l72 = self.nil_thunk(72);
                let sc1 = self.asm.sc1;
                self.asm.li(sc1, 1u64 << 32);
                self.asm.bgeu(kr, sc1, l72);
                // the 0/1 result MUST go to a fresh pool register (the key
                // may be a bank Ref — see the ITEMGET note above)
                self.pop_vreg();
                let r = self.push_vreg();
                match self.lists.kind[s] {
                    ListKind::Empty => self.asm.addi(r, ZERO, 0),
                    ListKind::Rodata { n } | ListKind::FrameStatic { n } => {
                        self.asm.addi(sc1, ZERO, n as i32);
                        // sltu r, kr, sc1 (r = idx < len)
                        self.asm.r(0, sc1, kr, 3, r, 0b0110011);
                    }
                    ListKind::Growable { .. } => {
                        let cell = self.slot_offset(s as u8);
                        self.asm.ld(sc1, S0, cell);
                        self.asm.r(0, sc1, kr, 3, r, 0b0110011);
                    }
                    // P2: the length rides behind the descriptor
                    ListKind::Arena { .. } => {
                        let cell = self.slot_offset(s as u8);
                        self.asm.ld(sc1, S0, cell);
                        self.asm.ld(sc1, sc1, 0);
                        self.asm.r(0, sc1, kr, 3, r, 0b0110011);
                    }
                    ListKind::NotList => {
                        return errf!(
                            "rv64 lower: opcode HASKEY is outside the translatable kernel subset"
                        );
                    }
                }
            }
            /* ---- B2 task 5: bytes-side writes and splices ---- */
            BUFSET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: bufset node shape unexpected")?;
                let s = self.list_slot_of_bytes(&*n.subx, op)?;
                return self.emit_bufset(s, &*n.suby, &*n.subz);
            }
            ITEMCAT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: itemcat node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                return self.emit_itemcat(s);
            }
            /* ---- task 7: heap reads over the frame-reserved block ---- */
            HREADU => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1>()
                    .ok_or("rv64 lower: heap read node shape unexpected")?;
                // marker: 2-bit uint type | 6-bit segment (the interpreter's
                // read_u); the u8 immediate cannot reach uty 4 — the guard
                // below is the defensive twin of the runtime "uint type not
                // supported" face
                let mark = p.para as u64;
                let uty = mark >> 6;
                let seg = (mark & 0x3f) as usize;
                let Some(len) = heap_uint_len(uty) else {
                    return errf!(
                        "rv64 lower: opcode HREADU is outside the translatable kernel subset \
                         (uint type {uty} not supported)"
                    );
                };
                let idx = len * seg;
                if idx + len > self.heap_bytes as usize {
                    // every reachable execution faults (L ≤ the
                    // reservation); the shortcut also keeps the baked lbu
                    // offsets inside the 12-bit window
                    self.trap(44);
                    return Ok(());
                }
                // sum through a0 (distinct from a1/sc1/sc2 in every mode —
                // a1 IS sc2 under ImcRemap)
                self.li_k(self.a0, (idx + len) as u64);
                self.heap_bounds(self.a0);
                let d = self.push_vreg();
                self.heap_load_be(d, self.heap_off + idx as i32, len);
            }
            HREADUL => {
                let p = node
                    .as_any()
                    .downcast_ref::<IRNodeParam2>()
                    .ok_or("rv64 lower: heap read node shape unexpected")?;
                // marker: 3-bit uint type | 13-bit segment (the interpreter's
                // read_ul); uty 4 is the u128 read (16 bytes, a (lo, hi)
                // vstack pair), uty 5..7 map to no uint type — the
                // interpreter faults HeapError at runtime, the kernel
                // rejects up front
                let mark = u16::from_be_bytes(p.para) as u64;
                let uty = mark >> 13;
                let seg = (mark & 0x1fff) as usize;
                let Some(len) = heap_uint_len(uty) else {
                    return errf!(
                        "rv64 lower: opcode HREADUL is outside the translatable kernel subset \
                         (uint type {uty} not supported)"
                    );
                };
                let idx = len * seg;
                if idx + len > self.heap_bytes as usize {
                    self.trap(44);
                    return Ok(());
                }
                self.li_k(self.a0, (idx + len) as u64);
                self.heap_bounds(self.a0);
                if uty == 4 {
                    // low half first (the u128 vstack convention): lo is
                    // bytes[8..16] BE, hi is bytes[0..8] BE
                    let lo = self.push_vreg();
                    self.heap_load_be(lo, self.heap_off + (idx + 8) as i32, 8);
                    let hi = self.push_vreg();
                    self.heap_load_be(hi, self.heap_off + idx as i32, 8);
                } else {
                    let d = self.push_vreg();
                    self.heap_load_be(d, self.heap_off + idx as i32, len);
                }
            }
            HREAD => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: heap read node shape unexpected")?;
                if node_res_ty(&*n.subx, self.slot)? != Ty::Int
                    || node_res_ty(&*n.suby, self.slot)? != Ty::Int
                {
                    return errf!(
                        "rv64 lower: opcode HREAD is outside the translatable kernel subset \
                         (the read start and length must be u64 values)"
                    );
                }
                let mev_st = self.nil_val(&*n.subx);
                let mev_len = self.nil_val(&*n.suby);
                self.eval(&*n.subx)?; // start
                self.eval(&*n.suby)?; // len
                let ld = self.vstack.len() - 1;
                let sd_ = self.vstack.len() - 2;
                // interpreter face order: len's extract_u16 first, then the
                // start's extract_u32 (both CastParamFail 72 on Nil or over
                // the width's ceiling)
                let l72 = self.nil_thunk(72);
                if mev_len {
                    self.nil_guard_tag(ld, 72);
                }
                let len_r = loc_reg(self.vstack[ld]);
                let sc = self.asm.sc1;
                self.li_k(sc, 0xFFFF);
                self.asm.bltu(sc, len_r, l72);
                if mev_st {
                    self.nil_guard_tag(sd_, 72);
                }
                let st_r = loc_reg(self.vstack[sd_]);
                let sc2 = self.asm.sc2;
                self.li_k(sc2, 0xFFFF_FFFF);
                self.asm.bltu(sc2, st_r, l72);
                let len_r = self.pop_reg(); // len (top) first
                let st_r = self.pop_reg();
                // bounds: start + len ≤ current length (no u64 overflow —
                // both faces above)
                self.asm.add(self.a0, st_r, len_r);
                self.heap_bounds(self.a0);
                // bytes descriptor over the frame block: ptr = S0 + heap_off
                // + start (S0 is live for the whole body — no calls rebind
                // it). Both results move through a0/a1 first: the pushes may
                // re-hand the just-popped operand registers.
                self.asm.mv(self.a1, len_r);
                self.asm.addi(self.a0, S0, self.heap_off);
                self.asm.add(self.a0, self.a0, st_r);
                let dp = self.push_vreg();
                self.asm.mv(dp, self.a0);
                let dl = self.push_vreg();
                self.asm.mv(dl, self.a1);
            }
            /* ---- task 7: heap writes ---- */
            HWRITE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: heap write node shape unexpected")?;
                if node_res_ty(&*n.subx, self.slot)? != Ty::Int {
                    return errf!(
                        "rv64 lower: opcode HWRITE is outside the translatable kernel subset \
                         (the write offset must be a u64 value)"
                    );
                }
                let mev_off = self.nil_val(&*n.subx);
                self.eval(&*n.subx)?; // offset
                self.eval(&*n.suby)?; // value
                // interpreter face order: the offset's extract_u32 (72)
                // runs inside emit_heap_write BEFORE the value's
                // extract_bytes (44)
                self.emit_heap_write(&*n.suby, mev_off, true, None)?;
            }
            HWRITEX => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: heap write node shape unexpected")?;
                self.eval(&*n.subx)?;
                self.emit_heap_write(&*n.subx, false, false, Some(n.para as usize))?;
            }
            HWRITEXL => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam2Single>()
                    .ok_or("rv64 lower: heap write node shape unexpected")?;
                self.eval(&*n.subx)?;
                self.emit_heap_write(
                    &*n.subx,
                    false,
                    false,
                    Some(u16::from_be_bytes(n.para) as usize),
                )?;
            }
            // T12 (pure-compute subset extension): the type-test family —
            // statically folded answers, may-Nil operands read their tag.
            TNIL | TIS | TID | TLIST | TMAP => self.emit_type_test(op, node)?,
            // T9: local[idx] OP rhs — the fused local logic comparison
            XLG => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: XLG node shape unexpected")?;
                self.emit_xlg(n, node)?
            }
            // T1: floor / ceil integer sqrt (software, width-exact)
            SQRT | SQRTUP => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: sqrt node shape unexpected")?;
                self.emit_sqrt(op, n)?
            }
            // T17: the element pops — the shared object's front/back
            // element (empty → 81), a scalar value on the vstack.
            TAKEFIRST | TAKELAST => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_take(s, op == TAKEFIRST)?;
            }
            // T17: KEYS/VALUES on a list — the interpreter's
            // CompoOpNotMatch(83) (map_ref fails before a list is built)
            KEYS | VALUES => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: compo op node shape unexpected")?;
                let _ = self.list_slot_of(&*n.subx, op)?;
                self.trap(83);
            }
            // defensive: these are plan-rejected before lowering ever runs
            // (B2 task 5: an ITEMSET/compound in a VALUE position — its
            // container result cannot be consumed by the subset; T17
            // INSERT/REMOVE/MERGE bind through the PUT source path)
            NEWLIST | PACKLIST | APPEND | CLEAR | ITEMSET | INSERT | REMOVE | MERGE => {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (a list value cannot be used as a value)"
                )
            }
            other => {
                return errf!(
                    "rv64 lower: opcode {:?} is outside the translatable kernel subset",
                    other
                )
            }
        }
        Ok(())
    }


    pub(crate) fn eval_int(&mut self, node: &dyn IRNode) -> Ret<()> {
        let ty = node_res_ty(node, self.slot)?;
        if ty != Ty::Int {
            return errf!("rv64 lower: expected an int value, got bytes");
        }
        self.eval(node)
    }

    pub(crate) fn eval_bytes(&mut self, node: &dyn IRNode) -> Ret<()> {
        let ty = node_res_ty(node, self.slot)?;
        // T13: an Address value lowers as its 21 content bytes — the same
        // descriptor every consumer of this entry point already operates
        // on (the interpreter's bytes-family ops all coerce via
        // extract_bytes/scalar_bytes, which is exactly the 21 bytes)
        if ty != Ty::Bytes && ty != Ty::Address {
            return errf!("rv64 lower: expected a bytes value, got int");
        }
        self.eval(node)
    }
}
