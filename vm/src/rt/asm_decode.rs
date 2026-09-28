/// Stream decode + form-1 (raw) print/assemble. Codec-safe.

/// Form 2 instruction indent. Two spaces so wallet / popup viewers stay readable
/// in a narrow column.
pub const ASM_INDENT: &str = "  ";

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedInst {
    pub offset: usize,
    pub op: Bytecode,
    pub imm: Vec<u8>,
}

pub fn decode_bytecode(codes: &[u8]) -> Ret<Vec<DecodedInst>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < codes.len() {
        let offset = i;
        let op = Bytecode::try_from_u8(codes[i]).map_err(|e| Error::normal(e.to_string()))?;
        let meta = op.metadata();
        if !meta.valid {
            return errf!("invalid opcode 0x{:02x} at {}", codes[i], i);
        }
        i += 1;
        let imm = match op {
            PBUF => {
                if i >= codes.len() {
                    return errf!("truncated PBUF length at {}", i);
                }
                let n = codes[i] as usize;
                let end = i + 1 + n;
                if end > codes.len() {
                    return errf!("truncated PBUF payload at {}..{}", i + 1, end);
                }
                let slice = codes[i..end].to_vec();
                i = end;
                slice
            }
            PBUFL => {
                if i + 2 > codes.len() {
                    return errf!("truncated PBUFL length at {}", i);
                }
                let n = u16::from_be_bytes([codes[i], codes[i + 1]]) as usize;
                let end = i + 2 + n;
                if end > codes.len() {
                    return errf!("truncated PBUFL payload at {}..{}", i + 2, end);
                }
                let slice = codes[i..end].to_vec();
                i = end;
                slice
            }
            _ => {
                let p = meta.param as usize;
                if i + p > codes.len() {
                    return errf!(
                        "truncated {} params at {}..{}",
                        format!("{:?}", op),
                        i,
                        i + p
                    );
                }
                let slice = codes[i..i + p].to_vec();
                i += p;
                slice
            }
        };
        out.push(DecodedInst { offset, op, imm });
    }
    Ok(out)
}

pub fn disassemble_bytecode_raw(codes: &[u8]) -> Ret<String> {
    let insts = decode_bytecode(codes)?;
    let mut res = String::new();
    for inst in &insts {
        res.push_str(&format!("{:?} ", inst.op));
        match inst.op {
            PBUF => {
                if inst.imm.is_empty() {
                    return errf!("PBUF missing length prefix");
                }
                res.push_str(&format!("{} ", inst.imm[0]));
                if inst.imm.len() > 1 {
                    res.push_str(&format!("0x{} ", hex::encode(&inst.imm[1..])));
                }
            }
            PBUFL => {
                if inst.imm.len() < 2 {
                    return errf!("PBUFL missing length prefix");
                }
                res.push_str(&format!("{} {} ", inst.imm[0], inst.imm[1]));
                if inst.imm.len() > 2 {
                    res.push_str(&format!("0x{} ", hex::encode(&inst.imm[2..])));
                }
            }
            _ => {
                for b in &inst.imm {
                    res.push_str(&format!("{} ", b));
                }
            }
        }
    }
    Ok(res.trim_end().to_string())
}

pub fn assemble_bytecode_raw(text: &str) -> Ret<Vec<u8>> {
    let _ = indent_rows(text)?;
    let toks = lex_asm(text)?;
    let mut out = Vec::new();
    for st in &toks {
        match &st.tok {
            AsmTok::Ident(id) => {
                let Some(op) = Bytecode::parse(id) else {
                    return errf!("bytecode {} not found", id);
                };
                out.push(op as u8);
            }
            AsmTok::Integer(n) => {
                if *n > u8::MAX as u128 {
                    return errf!("raw assembler integer {} exceeds 255", n);
                }
                out.push(*n as u8);
            }
            AsmTok::Hex(bytes) => out.extend_from_slice(bytes),
            other => return errf!("raw assembler unexpected token {:?}", other),
        }
    }
    Ok(out)
}

pub fn jump_dest_of(op: Bytecode, imm_off: usize, imm: &[u8]) -> Option<usize> {
    match op {
        JMPS | BRS => {
            let p = *imm.first()? as i8;
            Some((imm_off as isize + p as isize + 1) as usize)
        }
        JMPSL | BRSL | BRSLN => {
            if imm.len() < 2 {
                return None;
            }
            let p = i16::from_be_bytes([imm[0], imm[1]]);
            Some((imm_off as isize + p as isize + 2) as usize)
        }
        JMPL | BRL => {
            if imm.len() < 2 {
                return None;
            }
            Some(u16::from_be_bytes([imm[0], imm[1]]) as usize)
        }
        _ => None,
    }
}

pub fn looks_like_asm_text(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    if text.contains('{') || text.contains(',') || text.contains(':') {
        return true;
    }
    for line in text.lines() {
        let s = line.trim_start();
        if s.is_empty() {
            continue;
        }
        if s.starts_with('.') {
            return true;
        }
        if line.starts_with(ASM_INDENT) {
            return true;
        }
        if let Some(id) = s.split_whitespace().next() {
            if Bytecode::parse_intro(id).is_some() && Bytecode::parse(id).is_none() {
                return true;
            }
        }
    }
    false
}

pub(crate) fn check_asm_indent(text: &str) -> Ret<()> {
    apply_indent_levels(&indent_rows(text)?, 0)
}

pub(crate) fn check_asm_indent_from_min(text: &str) -> Ret<()> {
    let rows = indent_rows(text)?;
    let base = rows.iter().map(|r| r.1).min().unwrap_or(0);
    apply_indent_levels(&rows, base)
}

pub(crate) fn check_top_level_bytecode_indent(text: &str) -> Ret<()> {
    let rows = indent_rows(text)?;
    apply_indent_levels(&rows, 0)?;
    let instr: Vec<usize> = rows
        .iter()
        .filter(|(_, _, rest)| !(rest.starts_with('.') && rest.contains(':')) && *rest != "}")
        .map(|(_, n, _)| *n)
        .collect();
    match instr.as_slice() {
        [] | [0] => Ok(()),
        indents if indents.iter().all(|&i| i == 2) => Ok(()),
        _ => errf!("assembler: form-2 instructions must use 2-space indent"),
    }
}

fn indent_rows(text: &str) -> Ret<Vec<(usize, usize, &str)>> {
    let mut rows = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let mut i = 0usize;
        let bytes = line.as_bytes();
        while i < bytes.len() {
            match bytes[i] {
                b' ' => i += 1,
                b'\t' => {
                    return errf!(
                        "assembler: tab indent is not allowed, use 2 spaces (line {})",
                        lineno
                    )
                }
                _ => break,
            }
        }
        let rest = line[i..].trim();
        if rest.is_empty() || rest.starts_with("//") || rest.starts_with(';') {
            continue;
        }
        rows.push((lineno, i, rest));
    }
    Ok(rows)
}

fn apply_indent_levels(rows: &[(usize, usize, &str)], base: usize) -> Ret<()> {
    let mut stack = vec![base];
    for &(lineno, indent, _) in rows {
        if indent < base {
            return errf!("assembler: indent under base at line {}", lineno);
        }
        while indent < *stack.last().unwrap() {
            stack.pop();
            if stack.is_empty() {
                return errf!("assembler: indent underflow at line {}", lineno);
            }
        }
        if indent > *stack.last().unwrap() {
            if indent != *stack.last().unwrap() + 2 {
                return errf!(
                    "assembler: indent must be 2 spaces per level (line {})",
                    lineno
                );
            }
            stack.push(indent);
        }
    }
    Ok(())
}
