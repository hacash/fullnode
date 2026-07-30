type IRNRef<'a> = &'a Box<dyn IRNode>;

// (u16::MAX/2 - jpl as u16).into();
const JMP_INST_LEN: usize = 3; //u8 + u16
const BLOCK_CODES_MAX_LEN: usize = i16::MAX as usize - JMP_INST_LEN - 64;

fn is_stmt_block(n: IRNRef) -> bool {
    // Statement blocks (IRBLOCK) discard their own children's return values.
    // We must look through any IRNodeWrapOne layer so that wrapped blocks are
    // recognized; otherwise compile_while/compile_if would re-append a POP on
    // top of the POPs already emitted by compile_block_into, corrupting the
    // operand stack. See BUG-4 in the IR codegen audit.
    if let Some(wrap) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
        return is_stmt_block(&wrap.node);
    }
    if let Some(arr) = n.as_any().downcast_ref::<IRNodeArray>() {
        return arr.inst == Bytecode::IRBLOCK;
    }
    false
}


fn compile_block_into(inst: Bytecode, list: &[Box<dyn IRNode>], codes: &mut Vec<u8>) -> VmrtErr {
    let is_expr = inst == Bytecode::IRBLOCKR;
    if is_expr {
        match list.last() {
            None => return itr_err_fmt!(CompileError, "block expression cannot be empty"),
            Some(last) if !last.hasretval() => return itr_err_fmt!(CompileError, "block expression must return a value"),
            _ => {},
        }
    }
    for (idx, one) in list.iter().enumerate() {
        one.codegen_into(codes)?;
        if one.hasretval() {
            if is_expr && idx + 1 == list.len() {
                continue;
            }
            codes.push(POP as u8);
        }
    }
    Ok(())
}

fn compile_block(inst: Bytecode, list: &[Box<dyn IRNode>]) -> VmrtRes<Vec<u8>> {
    let mut codes = Vec::new();
    compile_block_into(inst, list, &mut codes)?;
    Ok(codes)
}

fn compile_list_into(list: &[Box<dyn IRNode>], codes: &mut Vec<u8>) -> VmrtErr {
    // Defense in depth: even when an IRLIST reaches codegen without going
    // through `parse_ir_node_must`, the tail PACK invariants (literal item
    // count, no zero count, PACKMAP parity) must still hold. The parse-side
    // check stays for early rejection on deserialized streams; this is the
    // mirror for in-process IR construction paths.
    validate_irlist_stack_tail(list)?;
    for one in list {
        one.codegen_into(codes)?;
    }
    Ok(())
}

fn compile_list(list: &[Box<dyn IRNode>]) -> VmrtRes<Vec<u8>> {
    let mut codes = Vec::new();
    compile_list_into(list, &mut codes)?;
    Ok(codes)
}


fn compile_double(btcd: Bytecode, x: IRNRef, y: IRNRef) -> VmrtRes<Option<Vec<u8>>> {
    // I-12: IRWHILE must be lowered through IRNodeDouble. If it ever leaks
    // into another node shape, the structural assumption that subx=cond and
    // suby=body is broken, so fail loud instead of silently miscompiling.
    Ok(match btcd {
        IRWHILE => Some(compile_while(x, y)?),
        _ => None
    })
}

/********** Patch-list loop lowering **********/

/// A list of unresolved jump-slot positions that belong to a single
/// while-loop scope. `LoopPatch::resolve` writes the final i16
/// displacements once the loop layout is known.
///
/// See `compile_while` for the layout diagram.
#[derive(Default)]
struct LoopPatch {
    breaks: Vec<usize>,    // body-offsets of the i16 displacement byte
    continues: Vec<usize>, // same, for continue targets
}

impl LoopPatch {
    /// Emit a `JMPSL 0 0` and record the displacement slot.
    fn emit_jump(&mut self, ctrl: Bytecode, body: &mut Vec<u8>) {
        body.push(JMPSL as u8);
        let slot = body.len(); // points at the i16 high byte
        body.push(0);
        body.push(0);
        match ctrl {
            IRBREAK => self.breaks.push(slot),
            IRCONTINUE => self.continues.push(slot),
            _ => unreachable!(),
        }
    }

    /// Shift all patch-slot values by `delta` — used when a sub-buffer
    /// that was lowered independently gets appended to the main body at
    /// a non-zero base.
    fn shift(&mut self, delta: usize) {
        for slot in self.breaks.iter_mut() {
            *slot += delta;
        }
        for slot in self.continues.iter_mut() {
            *slot += delta;
        }
    }

    /// Write the final i16 displacement into every recorded slot.
    ///
    /// Layout produced by `compile_while`:
    ///
    /// ```text
    /// [cond:cond_len][BRSLN +body_l][body:body.len()][JMPSL -alls_l]
    ///                                ^ body_start = cond_len + JIL
    /// ```
    ///
    /// Inside `body`, a patch slot at offset `slot` has just consumed the
    /// `JMPSL` opcode at `slot - 1`. The runtime `ostjump!` macro reads
    /// the i16 starting at `slot`, then `pc` advances to `slot + 2`.
    /// When computing a relative offset, the `pc` that the i16 will be
    /// added to is `slot + 2` (body-relative).
    fn resolve(&self, body: &mut [u8], cond_len: usize) -> VmrtErr {
        const JIL: usize = JMP_INST_LEN;
        let body_len = body.len();
        for &slot in &self.breaks {
            // target = body_len + JIL (just past the trailing JMPSL);
            //         pc = slot + 2  (after the i16 is consumed);
            let offset = (body_len + JIL) as i32 - (slot as i32 + 2);
            write_i16_offset(body, slot, offset, "break")?;
        }
        for &slot in &self.continues {
            // target = 0 (start of cond) in absolute coordinates;
            //         absolute pc = cond_len + JIL + slot + 2;
            let pc_abs = cond_len as i32 + JIL as i32 + slot as i32 + 2;
            let offset = -(pc_abs as i32);
            write_i16_offset(body, slot, offset, "continue")?;
        }
        Ok(())
    }
}

fn write_i16_offset(buf: &mut [u8], slot: usize, offset: i32, label: &str) -> VmrtErr {
    if offset < i16::MIN as i32 || offset > i16::MAX as i32 {
        return itr_err_fmt!(
            CompileError,
            "while loop {} jump out of i16 range: {}",
            label,
            offset
        );
    }
    let bytes = (offset as i16).to_be_bytes();
    buf[slot] = bytes[0];
    buf[slot + 1] = bytes[1];
    Ok(())
}

/// Recursively lower `node`, intercepting IRBREAK/IRCONTINUE leaf nodes
/// as JMPSL placeholders tracked by `patch`. Any nested IRWHILE opens its
/// own patch scope (handled by `compile_while`), so inner breaks/continues
/// target the inner loop and won't pollute the outer `patch`.
fn lower_in_loop(node: &dyn IRNode, body: &mut Vec<u8>, patch: &mut LoopPatch) -> VmrtErr {
    // Look through wrap: the wrap is a print-time aid.
    if let Some(wrap) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return lower_in_loop(&*wrap.node, body, patch);
    }
    // Leaf IRBREAK / IRCONTINUE → JMPSL placeholder.
    if let Some(leaf) = node.as_any().downcast_ref::<IRNodeLeaf>() {
        if matches!(leaf.inst, IRBREAK | IRCONTINUE) {
            patch.emit_jump(leaf.inst, body);
            return Ok(());
        }
    }
    // Container arrays: walk children. Mirrors compile_block_into /
    // compile_list_into but threads the patch list.
    if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
        return lower_array_in_loop(arr, body, patch);
    }
    // IRIF (statement if): lower each branch with the same patch list.
    // IRIFR (expression if): falls through to default codegen because
    //   break/continue are never syntactically valid inside an expression
    //   context; `parse_item` in stmt.rs rejects them when `expect_retval`.
    if let Some(tri) = node.as_any().downcast_ref::<IRNodeTriple>() {
        if tri.inst == IRIF {
            return lower_stmt_if_in_loop(tri, body, patch);
        }
    }
    // Nested IRWHILE: opens its own scope, so delegate to regular codegen.
    if let Some(dbl) = node.as_any().downcast_ref::<IRNodeDouble>() {
        if dbl.inst == IRWHILE {
            return dbl.codegen_into(body);
        }
    }
    // Everything else: regular codegen. The sub-tree cannot contain a
    // surface-language break/continue.
    node.codegen_into(body)
}

fn lower_array_in_loop(arr: &IRNodeArray, body: &mut Vec<u8>, patch: &mut LoopPatch) -> VmrtErr {
    match arr.inst {
        IRLIST => {
            validate_irlist_stack_tail(&arr.subs)?;
            for one in &arr.subs {
                lower_in_loop(&**one, body, patch)?;
            }
            Ok(())
        }
        IRBLOCK | IRBLOCKR => {
            let is_expr = arr.inst == IRBLOCKR;
            if is_expr {
                match arr.subs.last() {
                    None => return itr_err_fmt!(CompileError, "block expression cannot be empty"),
                    Some(last) if !last.hasretval() => {
                        return itr_err_fmt!(CompileError, "block expression must return a value")
                    }
                    _ => {}
                }
            }
            for (idx, one) in arr.subs.iter().enumerate() {
                lower_in_loop(&**one, body, patch)?;
                if one.hasretval() {
                    if is_expr && idx + 1 == arr.subs.len() {
                        continue;
                    }
                    body.push(POP as u8);
                }
            }
            Ok(())
        }
        _ => itr_err_fmt!(CompileError, "IRNodeArray: invalid opcode {:?}", arr.inst),
    }
}

/// Lower an IRIF (not IRIFR) inside a loop body.
///
/// We lower each branch into a **temporary** buffer with its own patch
/// scope, then splice everything into the main `body`, and move the
/// sub-patches into the outer `patch` after shifting by the base offset
/// where each branch was inserted.
fn lower_stmt_if_in_loop(
    tri: &IRNodeTriple,
    body: &mut Vec<u8>,
    outer_patch: &mut LoopPatch,
) -> VmrtErr {
    const JIL: usize = JMP_INST_LEN;
    const MAXL: usize = BLOCK_CODES_MAX_LEN;

    let mut cond = Vec::new();
    tri.subx.codegen_into(&mut cond)?;

    // Lower then-branch with a sub-patch.
    let mut then_patch = LoopPatch::default();
    let mut then_br = Vec::new();
    lower_in_loop(&*tri.suby, &mut then_br, &mut then_patch)?;
    if tri.suby.hasretval() && !is_stmt_block(&tri.suby) {
        then_br.push(POP as u8);
    }

    // Lower else-branch with a sub-patch.
    let mut else_patch = LoopPatch::default();
    let mut else_br = Vec::new();
    lower_in_loop(&*tri.subz, &mut else_br, &mut else_patch)?;
    if tri.subz.hasretval() && !is_stmt_block(&tri.subz) {
        else_br.push(POP as u8);
    }

    let then_l = then_br.len();
    let else_l = else_br.len() + JIL;
    if then_l > MAXL || else_l > MAXL {
        return itr_err_fmt!(CompileError, "compiled IR code is too long");
    }
    debug_assert!(then_l as i32 <= i16::MAX as i32);
    debug_assert!(else_l as i32 <= i16::MAX as i32);

    // Layout: cond | BRSL +else_l | else_br | JMPSL +then_l | then_br
    let else_br_start = body.len() + cond.len() + JIL;
    let then_br_start = else_br_start + else_br.len() + JIL;

    body.extend_from_slice(&cond);
    body.push(BRSL as u8);
    body.extend_from_slice(&(else_l as i16).to_be_bytes());
    body.extend_from_slice(&else_br);
    body.push(JMPSL as u8);
    body.extend_from_slice(&(then_l as i16).to_be_bytes());
    body.extend_from_slice(&then_br);

    // Shift sub-patches to the global body coordinate and fold into outer.
    else_patch.shift(else_br_start);
    then_patch.shift(then_br_start);
    outer_patch.breaks.extend(else_patch.breaks);
    outer_patch.breaks.extend(then_patch.breaks);
    outer_patch.continues.extend(else_patch.continues);
    outer_patch.continues.extend(then_patch.continues);
    Ok(())
}


fn compile_while(x: IRNRef, y: IRNRef) -> VmrtRes<Vec<u8>> {
    const JIL: usize = JMP_INST_LEN;
    const MAXL: usize = BLOCK_CODES_MAX_LEN;

    // Lower condition as a regular sub-expression.
    let mut cond = Vec::new();
    x.codegen_into(&mut cond)?;

    // Lower body through the patch-list lowering, which intercepts any
    // IRBREAK/IRCONTINUE as JMPSL placeholders.
    let mut patch = LoopPatch::default();
    let mut body = Vec::new();
    lower_in_loop(&**y, &mut body, &mut patch)?;

    // IRBLOCK already discards return values internally. Only append POP
    // when the body is NOT a statement block container.
    if y.hasretval() && !is_stmt_block(y) {
        body.push(POP as u8);
    }

    // Resolve patches now that the final body length is known.
    patch.resolve(&mut body, cond.len())?;

    // body_l = body (containing the trailing break-jumping JMPSL
    // instructions) + JIL for the trailing JMPSL.
    // alls_l = body_l + cond + JIL for BRSLN.
    let body_l = body.len() + JIL;
    let alls_l = body_l + cond.len() + JIL;

    if body_l > MAXL || alls_l > MAXL {
        return itr_err_fmt!(CompileError, "compiled IR code is too long");
    }

    debug_assert!(alls_l as i32 <= i16::MAX as i32);
    debug_assert!(body_l as i32 <= i16::MAX as i32);

    // Emit the loop frame.
    let mut codes = Vec::with_capacity(cond.len() + 1 + 2 + body.len() + 1 + 2);
    codes.extend_from_slice(&cond);
    codes.push(BRSLN as u8);
    codes.extend_from_slice(&(body_l as i16).to_be_bytes());
    codes.extend_from_slice(&body);
    codes.push(JMPSL as u8);
    codes.extend_from_slice(&(-(alls_l as i16)).to_be_bytes());
    Ok(codes)
}


/**************************************************/


fn compile_triple(btcd: Bytecode, x: IRNRef, y: IRNRef, z: IRNRef) -> VmrtRes<Option<Vec<u8>>> {
    Ok(match btcd {
        IRIF | IRIFR => Some(compile_if(btcd, x, y, z)?),
        _ => None
    })
}


fn compile_if(btcd: Bytecode, x: IRNRef, y: IRNRef, z: IRNRef) -> VmrtRes<Vec<u8>> {
    const JIL: usize = JMP_INST_LEN;
    const MAXL: usize = BLOCK_CODES_MAX_LEN;
    let mut cond = Vec::new();
    x.codegen_into(&mut cond)?;
    let mut if_br = Vec::new();
    y.codegen_into(&mut if_br)?;
    let is_expr = btcd == Bytecode::IRIFR;
    if is_expr && !y.hasretval() {
        return itr_err_fmt!(CompileError, "if expression branch must return a value");
    }
    // IRBLOCK already discards return values internally.
    if !is_expr && y.hasretval() && !is_stmt_block(y) {
        if_br.push(POP as u8); // pop inst
    }
    let mut el_br = Vec::new();
    z.codegen_into(&mut el_br)?;
    if is_expr && !z.hasretval() {
        return itr_err_fmt!(CompileError, "if expression branch must return a value");
    }
    if !is_expr && z.hasretval() && !is_stmt_block(z) {
        el_br.push(POP as u8); // pop inst
    }
    let if_l = if_br.len();
    let el_l = el_br.len() + JIL;
    // check code len: each branch must fit a signed-16 offset.
    if if_l > MAXL || el_l > MAXL {
        return itr_err_fmt!(CompileError, "compiled IR code is too long")
    }
    // Total emitted IR must also fit within the per-function code window so
    // that downstream u16/i16 indexing stays safe. Surface the same
    // CompileError class instead of letting the runtime verifier convert
    // this into a generic CodeTooLong later.
    let total_len = cond
        .len()
        .checked_add(JIL) // BRSL
        .and_then(|n| n.checked_add(el_br.len()))
        .and_then(|n| n.checked_add(JIL)) // JMPSL
        .and_then(|n| n.checked_add(if_br.len()));
    match total_len {
        Some(n) if n <= u16::MAX as usize => {}
        _ => return itr_err_fmt!(CompileError, "compiled IR code is too long"),
    }
    // Both forward jumps fit a signed-16 immediate; the MAXL bound above
    // guarantees this, so a debug assert keeps the invariant visible if
    // anyone widens MAXL or removes the branch checks.
    debug_assert!(if_l as i32 <= i16::MAX as i32);
    debug_assert!(el_l as i32 <= i16::MAX as i32);
    let mut codes = Vec::with_capacity(
        cond.len() + 1 + 2 + el_br.len() + 1 + 2 + if_br.len()
    );
    codes.extend_from_slice(&cond);
    codes.push(BRSL as u8);
    codes.extend_from_slice(&(el_l as i16).to_be_bytes());
    codes.extend_from_slice(&el_br);
    codes.push(JMPSL as u8);
    codes.extend_from_slice(&(if_l as i16).to_be_bytes());
    codes.extend_from_slice(&if_br);
    Ok(codes)
}