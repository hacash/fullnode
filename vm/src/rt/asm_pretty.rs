/// Bytecode form-2 (GAS-style) print/parse. Codec-safe.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JumpStyle {
    Labels,
    Immediate,
}

#[derive(Clone)]
enum JumpArg {
    Label(String),
    Dest(usize),
}

pub fn disassemble_bytecode_asm(codes: &[u8]) -> Ret<String> {
    let insts = decode_bytecode(codes)?;
    let dests: std::collections::HashSet<usize> = insts
        .iter()
        .filter_map(|inst| jump_dest_of(inst.op, inst.offset + 1, &inst.imm))
        .collect();
    let mut res = String::new();
    for inst in &insts {
        if dests.contains(&inst.offset) {
            res.push_str(&format!(".L{}:\n", inst.offset));
        }
        let ops = print_operands(inst.op, &inst.imm, inst.offset, JumpStyle::Labels)?;
        if ops.is_empty() {
            res.push_str(&format!("{}{}\n", ASM_INDENT, inst.op.metadata().intro));
        } else {
            res.push_str(&format!(
                "{}{} {}\n",
                ASM_INDENT,
                inst.op.metadata().intro,
                ops
            ));
        }
    }
    if dests.contains(&codes.len()) {
        res.push_str(&format!(".L{}:\n", codes.len()));
    }
    if res.ends_with('\n') {
        res.pop();
    }
    Ok(res)
}

fn print_operands(op: Bytecode, imm: &[u8], offset: usize, jumps: JumpStyle) -> Ret<String> {
    match op {
        PU8 => {
            let v = *imm
                .first()
                .ok_or_else(|| Error::normal("truncated push_u8"))?;
            Ok(format!("{}", v))
        }
        PU16 => {
            if imm.len() < 2 {
                return errf!("truncated push_u16");
            }
            Ok(format!("{}", u16::from_be_bytes([imm[0], imm[1]])))
        }
        PBUF => {
            if imm.is_empty() {
                return errf!("truncated push_buf");
            }
            if imm.len() == 1 {
                Ok(String::new())
            } else {
                Ok(format!("0x{}", hex::encode(&imm[1..])))
            }
        }
        PBUFL => {
            if imm.len() < 2 {
                return errf!("truncated push_buf_long");
            }
            if imm.len() == 2 {
                Ok(String::new())
            } else {
                Ok(format!("0x{}", hex::encode(&imm[2..])))
            }
        }
        GET | PUT | ALLOC => {
            let v = *imm
                .first()
                .ok_or_else(|| Error::normal("truncated local slot"))?;
            Ok(format!("{}", v))
        }
        XOP => {
            let mark = *imm
                .first()
                .ok_or_else(|| Error::normal("truncated local_operand"))?;
            let (lx, idx) = decode_local_operand_mark(mark);
            Ok(format!("{}, {}", idx, lx.symbol()))
        }
        XLG => {
            let mark = *imm
                .first()
                .ok_or_else(|| Error::normal("truncated local_logic"))?;
            let (lg, idx) = decode_local_logic_mark(mark);
            Ok(format!("{}, {}", idx, lg.symbol()))
        }
        JMPS | BRS | JMPSL | BRSL | BRSLN | JMPL | BRL => {
            if jumps == JumpStyle::Immediate {
                return Ok(print_imm_as_ints(op, imm));
            }
            let Some(dest) = jump_dest_of(op, offset + 1, imm) else {
                return errf!("truncated jump immediate");
            };
            Ok(format!(".L{}", dest))
        }
        CALLTHIS | CALLSELF | CALLSUPER | CALLSELFVIEW | CALLSELFPURE => {
            Ok(format!("0x{}", hex::encode(imm)))
        }
        CALLEXT | CALLEXTVIEW | CALLUSEVIEW | CALLUSEPURE | CODE_CALL => {
            if imm.is_empty() {
                return errf!("truncated lib-call immediate");
            }
            Ok(format!("{}, 0x{}", imm[0], hex::encode(&imm[1..])))
        }
        CALL => print_generic_call(imm),
        ACTION => Ok(act_operand(imm, &ACTION_DEFS)?),
        ACTVIEW => Ok(act_operand(imm, &ACTION_VIEW_DEFS)?),
        ACTENV => Ok(act_operand(imm, &ACTION_ENV_DEFS)?),
        NTENV => Ok(native_operand(imm, NativeEnv::try_from_u8, NativeEnv::name)?),
        NTCTL => Ok(native_operand(imm, NativeCtl::try_from_u8, NativeCtl::name)?),
        NTFUNC => Ok(native_operand(imm, NativeFunc::try_from_u8, NativeFunc::name)?),
        _ => Ok(print_imm_as_ints(op, imm)),
    }
}

fn print_imm_as_ints(op: Bytecode, imm: &[u8]) -> String {
    let param = op.metadata().param as usize;
    if param == 2 && imm.len() == 2 {
        return format!("{}", u16::from_be_bytes([imm[0], imm[1]]));
    }
    imm.iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn act_operand(imm: &[u8], defs: &[ActDefTy]) -> Ret<String> {
    let id = *imm
        .first()
        .ok_or_else(|| Error::normal("truncated action id"))?;
    match search_act_by_id(id, defs) {
        Some(def) => Ok(def.1.to_string()),
        None => Ok(id.to_string()),
    }
}

fn native_operand<T>(
    imm: &[u8],
    from_u8: fn(u8) -> VmrtRes<T>,
    name: fn(&T) -> &'static str,
) -> Ret<String> {
    let id = *imm
        .first()
        .ok_or_else(|| Error::normal("truncated native id"))?;
    match from_u8(id) {
        Ok(v) => Ok(name(&v).to_string()),
        Err(_) => Ok(id.to_string()),
    }
}

fn print_generic_call(imm: &[u8]) -> Ret<String> {
    let spec = decode_call_body(imm).map_err(|e| Error::normal(e.to_string()))?;
    match spec {
        CallSpec::Invoke {
            target,
            effect,
            selector,
        } => Ok(format!(
            "{}, {}, 0x{}",
            effect_name(effect),
            target_name(target),
            hex::encode(selector)
        )),
        CallSpec::Splice { .. } => errf!("generic CALL decoded as splice"),
    }
}

fn effect_name(effect: EffectMode) -> &'static str {
    match effect {
        EffectMode::Edit => "edit",
        EffectMode::View => "view",
        EffectMode::Pure => "pure",
    }
}

fn target_name(target: CallTarget) -> String {
    match target {
        CallTarget::This => "this".to_string(),
        CallTarget::Self_ => "self".to_string(),
        CallTarget::Upper => "upper".to_string(),
        CallTarget::Super => "super".to_string(),
        CallTarget::Ext(n) => format!("ext({})", n),
        CallTarget::Use(n) => format!("use({})", n),
    }
}

pub fn assemble_bytecode_asm(text: &str) -> Ret<Vec<u8>> {
    check_top_level_bytecode_indent(text)?;
    assemble_form2(text)
}

pub(crate) fn assemble_bytecode_asm_nested(text: &str) -> Ret<Vec<u8>> {
    check_asm_indent_from_min(text)?;
    assemble_form2(text)
}

fn assemble_form2(text: &str) -> Ret<Vec<u8>> {
    let toks = lex_asm(text)?;
    let mut cur = TokCursor::new(text, &toks);
    assemble_form2_from(&mut cur)
}

fn assemble_form2_from(cur: &mut TokCursor) -> Ret<Vec<u8>> {
    struct Item {
        op: Bytecode,
        imm: Vec<u8>,
        jump: Option<JumpArg>,
    }

    let mut labels: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut pending_labels: Vec<String> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    let mut offsets = Vec::new();
    let mut pc = 0usize;

    while !cur.is_empty() {
        if matches!(cur.peek(), Some(AsmTok::RBrace)) {
            break;
        }
        if matches!(cur.peek(), Some(AsmTok::Label(_))) {
            let name = match cur.next().map(|t| &t.tok) {
                Some(AsmTok::Label(s)) => s.clone(),
                _ => unreachable!(),
            };
            if matches!(cur.peek(), Some(AsmTok::Colon)) {
                cur.next();
            }
            pending_labels.push(name);
            continue;
        }
        if matches!(cur.peek(), Some(AsmTok::Ident(_))) {
            let name = match cur.next().map(|t| &t.tok) {
                Some(AsmTok::Ident(s)) => s.clone(),
                _ => unreachable!(),
            };
            let Some(op) = Bytecode::parse_intro(&name) else {
                return errf!("bytecode {} not found", name);
            };
            let (imm, jump) = parse_asm_operands(cur, op)?;
            for lab in pending_labels.drain(..) {
                if labels.insert(lab.clone(), pc).is_some() {
                    return errf!("duplicate label .{}", lab);
                }
            }
            offsets.push(pc);
            pc += inst_byte_size(op, &imm);
            items.push(Item { op, imm, jump });
            continue;
        }
        return errf!("assembler: unexpected token {:?}", cur.peek());
    }
    for lab in pending_labels.drain(..) {
        if labels.insert(lab.clone(), pc).is_some() {
            return errf!("duplicate label .{}", lab);
        }
    }

    let mut out = Vec::with_capacity(pc);
    for (idx, item) in items.iter().enumerate() {
        out.push(item.op as u8);
        if let Some(jump) = &item.jump {
            let dest = match jump {
                JumpArg::Label(name) => *labels
                    .get(name)
                    .ok_or_else(|| Error::normal(format!("unknown label .{}", name)))?,
                JumpArg::Dest(d) => *d,
            };
            let encoded = encode_jump_imm(item.op, offsets[idx] + 1, dest)?;
            out.extend_from_slice(&encoded);
        } else {
            out.extend_from_slice(&item.imm);
        }
    }
    Ok(out)
}

fn inst_byte_size(op: Bytecode, imm: &[u8]) -> usize {
    match op {
        PBUF | PBUFL => 1 + imm.len(),
        _ => 1 + op.metadata().param as usize,
    }
}

fn encode_jump_imm(op: Bytecode, imm_off: usize, dest: usize) -> Ret<Vec<u8>> {
    match op {
        JMPS | BRS => {
            let need = dest as isize - imm_off as isize - 1;
            if need < i8::MIN as isize || need > i8::MAX as isize {
                return errf!(
                    "jump dest {} does not fit {:?} i8 offset from {}",
                    dest,
                    op,
                    imm_off
                );
            }
            Ok(vec![need as i8 as u8])
        }
        JMPSL | BRSL | BRSLN => {
            let need = dest as isize - imm_off as isize - 2;
            if need < i16::MIN as isize || need > i16::MAX as isize {
                return errf!(
                    "jump dest {} does not fit {:?} i16 offset from {}",
                    dest,
                    op,
                    imm_off
                );
            }
            Ok((need as i16).to_be_bytes().to_vec())
        }
        JMPL | BRL => {
            if dest > u16::MAX as usize {
                return errf!("jump dest {} exceeds u16", dest);
            }
            Ok((dest as u16).to_be_bytes().to_vec())
        }
        _ => errf!("not a jump opcode {:?}", op),
    }
}

fn parse_asm_operands(cur: &mut TokCursor, op: Bytecode) -> Ret<(Vec<u8>, Option<JumpArg>)> {
    match op {
        PU8 => Ok((vec![parse_u8_operand(cur)?], None)),
        PU16 => {
            let n = parse_u16_operand(cur)?;
            Ok((n.to_be_bytes().to_vec(), None))
        }
        PBUF => {
            let payload = parse_optional_hex(cur)?;
            if payload.len() > u8::MAX as usize {
                return errf!(
                    "push_buf payload {} exceeds 255; use push_buf_long",
                    payload.len()
                );
            }
            let mut imm = vec![payload.len() as u8];
            imm.extend_from_slice(&payload);
            Ok((imm, None))
        }
        PBUFL => {
            let payload = parse_optional_hex(cur)?;
            if payload.len() > u16::MAX as usize {
                return errf!("push_buf_long payload too long");
            }
            let mut imm = (payload.len() as u16).to_be_bytes().to_vec();
            imm.extend_from_slice(&payload);
            Ok((imm, None))
        }
        GET | PUT | ALLOC => Ok((vec![parse_u8_operand(cur)?], None)),
        XOP => {
            let idx = parse_u8_operand(cur)?;
            cur.skip_commas();
            let sym = parse_op_symbol(cur)?;
            let lx = lxop_from_symbol(&sym)?;
            let mark =
                encode_local_operand_mark(lx, idx).map_err(|e| Error::normal(e.to_string()))?;
            Ok((vec![mark], None))
        }
        XLG => {
            let idx = parse_u8_operand(cur)?;
            cur.skip_commas();
            let sym = parse_op_symbol(cur)?;
            let lg = lxlg_from_symbol(&sym)?;
            let mark = encode_local_logic_mark(lg, idx).map_err(|e| Error::normal(e.to_string()))?;
            Ok((vec![mark], None))
        }
        JMPS | BRS | JMPSL | BRSL | BRSLN | JMPL | BRL => {
            cur.skip_commas();
            match cur.peek() {
                Some(AsmTok::Label(_)) => {
                    let name = match cur.next().map(|t| &t.tok) {
                        Some(AsmTok::Label(s)) => s.clone(),
                        _ => unreachable!(),
                    };
                    let w = op.metadata().param as usize;
                    Ok((vec![0u8; w], Some(JumpArg::Label(name))))
                }
                Some(AsmTok::Integer(_)) => {
                    let n = parse_usize_operand(cur)?;
                    let w = op.metadata().param as usize;
                    Ok((vec![0u8; w], Some(JumpArg::Dest(n))))
                }
                other => errf!("expected jump dest, got {:?}", other),
            }
        }
        CALLTHIS | CALLSELF | CALLSUPER | CALLSELFVIEW | CALLSELFPURE => {
            let sel = parse_selector(cur)?;
            Ok((sel.to_vec(), None))
        }
        CALLEXT | CALLEXTVIEW | CALLUSEVIEW | CALLUSEPURE | CODE_CALL => {
            let lib = parse_u8_operand(cur)?;
            cur.skip_commas();
            let sel = parse_selector(cur)?;
            let mut imm = vec![lib];
            imm.extend_from_slice(&sel);
            Ok((imm, None))
        }
        CALL => {
            let spec = parse_generic_call_spec(cur)?;
            let CallSpec::Invoke {
                target,
                effect,
                selector,
            } = spec
            else {
                return errf!("generic call cannot be splice");
            };
            Ok((encode_call_body(target, effect, selector).to_vec(), None))
        }
        ACTION => Ok((vec![parse_act_id(cur, &ACTION_DEFS)?], None)),
        ACTVIEW => Ok((vec![parse_act_id(cur, &ACTION_VIEW_DEFS)?], None)),
        ACTENV => Ok((vec![parse_act_id(cur, &ACTION_ENV_DEFS)?], None)),
        NTENV => Ok((
            vec![parse_native_id(cur, |s| NativeEnv::from_name(s).map(|v| v.0))?],
            None,
        )),
        NTCTL => Ok((
            vec![parse_native_id(cur, |s| NativeCtl::from_name(s).map(|v| v.0))?],
            None,
        )),
        NTFUNC => Ok((
            vec![parse_native_id(cur, |s| NativeFunc::from_name(s).map(|v| v.0))?],
            None,
        )),
        _ => {
            let param = op.metadata().param as usize;
            if param == 0 {
                Ok((vec![], None))
            } else if param == 2 {
                match cur.peek() {
                    Some(AsmTok::Integer(_)) => {
                        let n = parse_u16_operand(cur)?;
                        Ok((n.to_be_bytes().to_vec(), None))
                    }
                    _ => {
                        let a = parse_u8_operand(cur)?;
                        cur.skip_commas();
                        let b = parse_u8_operand(cur)?;
                        Ok((vec![a, b], None))
                    }
                }
            } else {
                let mut imm = Vec::with_capacity(param);
                for k in 0..param {
                    if k > 0 {
                        cur.skip_commas();
                    }
                    imm.push(parse_u8_operand(cur)?);
                }
                Ok((imm, None))
            }
        }
    }
}

fn parse_u8_operand(cur: &mut TokCursor) -> Ret<u8> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Integer(n)) => {
            if *n > u8::MAX as u128 {
                return errf!("integer {} exceeds 255", n);
            }
            Ok(*n as u8)
        }
        other => errf!("expected integer 0-255, got {:?}", other),
    }
}

fn parse_u16_operand(cur: &mut TokCursor) -> Ret<u16> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Integer(n)) => {
            if *n > u16::MAX as u128 {
                return errf!("integer {} exceeds 65535", n);
            }
            Ok(*n as u16)
        }
        other => errf!("expected integer 0-65535, got {:?}", other),
    }
}

fn parse_usize_operand(cur: &mut TokCursor) -> Ret<usize> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Integer(n)) => {
            if *n > usize::MAX as u128 {
                return errf!("integer {} exceeds usize", n);
            }
            Ok(*n as usize)
        }
        other => errf!("expected integer dest, got {:?}", other),
    }
}

fn parse_hex_operand(cur: &mut TokCursor) -> Ret<Vec<u8>> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Hex(b)) => Ok(b.clone()),
        other => errf!("expected 0x hex bytes, got {:?}", other),
    }
}

fn parse_optional_hex(cur: &mut TokCursor) -> Ret<Vec<u8>> {
    cur.skip_commas();
    match cur.peek() {
        Some(AsmTok::Hex(_)) => parse_hex_operand(cur),
        _ => Ok(vec![]),
    }
}

fn parse_selector(cur: &mut TokCursor) -> Ret<FnSign> {
    let bytes = parse_hex_operand(cur)?;
    if bytes.len() != FN_SIGN_WIDTH {
        return errf!(
            "selector must be {} bytes, got {}",
            FN_SIGN_WIDTH,
            bytes.len()
        );
    }
    let mut sel = [0u8; FN_SIGN_WIDTH];
    sel.copy_from_slice(&bytes);
    Ok(sel)
}

fn parse_op_symbol(cur: &mut TokCursor) -> Ret<String> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Op(s)) => Ok(s.clone()),
        Some(AsmTok::Ident(s)) => Ok(s.clone()),
        other => errf!("expected operator symbol, got {:?}", other),
    }
}

fn lxop_from_symbol(s: &str) -> Ret<LxOp> {
    Ok(match s {
        "+=" => LxOp::Add,
        "-=" => LxOp::Sub,
        "*=" => LxOp::Mul,
        "/=" => LxOp::Div,
        _ => return errf!("unknown local_operand symbol {}", s),
    })
}

fn lxlg_from_symbol(s: &str) -> Ret<LxLg> {
    Ok(match s {
        "&&" => LxLg::And,
        "||" => LxLg::Or,
        "==" => LxLg::Eq,
        "!=" => LxLg::Ne,
        ">" => LxLg::Gt,
        ">=" => LxLg::Ge,
        "<" => LxLg::Lt,
        "<=" => LxLg::Le,
        _ => return errf!("unknown local_logic symbol {}", s),
    })
}

fn parse_act_id(cur: &mut TokCursor, defs: &[ActDefTy]) -> Ret<u8> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Ident(name)) => defs
            .iter()
            .find(|d| d.1 == name)
            .map(|d| d.0)
            .ok_or_else(|| Error::normal(format!("unknown action name {}", name))),
        Some(AsmTok::Integer(n)) => {
            if *n > u8::MAX as u128 {
                return errf!("action id {} exceeds 255", n);
            }
            Ok(*n as u8)
        }
        other => errf!("expected action name or id, got {:?}", other),
    }
}

fn parse_native_id(cur: &mut TokCursor, from_name: fn(&str) -> Option<u8>) -> Ret<u8> {
    cur.skip_commas();
    match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Ident(name)) => match from_name(name) {
            Some(id) => Ok(id),
            None => errf!("unknown native name {}", name),
        },
        Some(AsmTok::Integer(n)) => {
            if *n > u8::MAX as u128 {
                return errf!("native id {} exceeds 255", n);
            }
            Ok(*n as u8)
        }
        other => errf!("expected native name or id, got {:?}", other),
    }
}

fn parse_generic_call_spec(cur: &mut TokCursor) -> Ret<CallSpec> {
    cur.skip_commas();
    let effect = match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Ident(s)) => match s.as_str() {
            "edit" => EffectMode::Edit,
            "view" => EffectMode::View,
            "pure" => EffectMode::Pure,
            other => return errf!("unknown call effect {}", other),
        },
        other => return errf!("expected call effect, got {:?}", other),
    };
    cur.skip_commas();
    let target = parse_call_target(cur)?;
    cur.skip_commas();
    let selector = parse_selector(cur)?;
    Ok(CallSpec::invoke(target, effect, selector))
}

fn parse_call_target(cur: &mut TokCursor) -> Ret<CallTarget> {
    let ident = match cur.peek() {
        Some(AsmTok::Ident(s)) => s.clone(),
        other => return errf!("expected call target, got {:?}", other),
    };
    if ident == "ext" || ident == "use" {
        cur.next();
        match cur.next().map(|t| &t.tok) {
            Some(AsmTok::LParen) => {}
            other => return errf!("expected '(' after {}, got {:?}", ident, other),
        }
        let n = match cur.next().map(|t| &t.tok) {
            Some(AsmTok::Integer(v)) => {
                if *v > u8::MAX as u128 {
                    return errf!("lib index {} exceeds 255", v);
                }
                *v as u8
            }
            other => return errf!("expected lib index, got {:?}", other),
        };
        match cur.next().map(|t| &t.tok) {
            Some(AsmTok::RParen) => {}
            other => return errf!("expected ')' after lib index, got {:?}", other),
        }
        return Ok(if ident == "ext" {
            CallTarget::Ext(n)
        } else {
            CallTarget::Use(n)
        });
    }
    cur.next();
    Ok(match ident.as_str() {
        "this" => CallTarget::This,
        "self" => CallTarget::Self_,
        "upper" => CallTarget::Upper,
        "super" => CallTarget::Super,
        other => return errf!("unknown call target {}", other),
    })
}

pub(crate) fn print_ir_operands(op: Bytecode, imm: &[u8]) -> Ret<String> {
    print_operands(op, imm, 0, JumpStyle::Immediate)
}

pub(crate) fn parse_ir_operands(cur: &mut TokCursor, op: Bytecode) -> Ret<Vec<u8>> {
    match op {
        JMPS | BRS => Ok(vec![parse_u8_operand(cur)?]),
        JMPSL | BRSL | BRSLN | JMPL | BRL => match cur.peek() {
            Some(AsmTok::Integer(_)) => {
                let n = parse_u16_operand(cur)?;
                Ok(n.to_be_bytes().to_vec())
            }
            _ => {
                let a = parse_u8_operand(cur)?;
                cur.skip_commas();
                let b = parse_u8_operand(cur)?;
                Ok(vec![a, b])
            }
        },
        _ => {
            let (imm, jump) = parse_asm_operands(cur, op)?;
            if jump.is_some() {
                return errf!("labels are not valid in ircode tree operands; use ir_bytecode");
            }
            Ok(imm)
        }
    }
}
