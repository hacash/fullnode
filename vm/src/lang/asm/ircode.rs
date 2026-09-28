use super::validate_ircode_body;
use crate::ir::*;
use crate::rt::Bytecode::*;
use crate::rt::{
    assemble_bytecode_asm_nested, check_asm_indent, disassemble_bytecode_asm, lex_asm,
    parse_ir_operands, print_ir_operands, AsmTok, Bytecode, TokCursor,
};
use sys::*;

pub fn disassemble_ircode_asm(codes: &[u8]) -> Ret<String> {
    let mut seek = 0usize;
    let block = parse_ir_block(codes, &mut seek).map_err(|e| Error::normal(e.to_string()))?;
    if seek != codes.len() {
        return errf!("ircode trailing bytes at {}", seek);
    }
    let mut out = String::new();
    print_block_wrapper(&block, 0, &mut out)?;
    Ok(out)
}

fn print_block_wrapper(block: &IRNodeArray, indent: usize, out: &mut String) -> Ret<()> {
    out.push_str(&format!("{}ir_block {{\n", pad(indent)));
    for child in &block.subs {
        print_node(&**child, indent + 1, out)?;
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str(&format!("{}}}", pad(indent)));
    Ok(())
}

fn pad(indent: usize) -> String {
    crate::rt::ASM_INDENT.repeat(indent)
}

fn print_node(node: &dyn IRNode, indent: usize, out: &mut String) -> Ret<()> {
    let (inst, imm, kids) = ir_parts(node)?;
    if inst == IRBYTECODE {
        let inner = disassemble_bytecode_asm(&imm)?;
        out.push_str(&format!("{}ir_bytecode {{\n", pad(indent)));
        for line in inner.lines() {
            if line.is_empty() {
                continue;
            }
            out.push_str(&format!("{}{}\n", pad(indent + 1), line.trim_start()));
        }
        out.push_str(&format!("{}}}", pad(indent)));
        return Ok(());
    }
    if matches!(inst, IRBLOCK | IRBLOCKR | IRLIST) {
        out.push_str(&format!("{}{} {{\n", pad(indent), inst.metadata().intro));
        for kid in kids {
            print_node(kid, indent + 1, out)?;
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str(&format!("{}}}", pad(indent)));
        return Ok(());
    }
    let intro = inst.metadata().intro;
    let ops = print_ir_operands(inst, &imm)?;
    out.push_str(&pad(indent));
    out.push_str(intro);
    if !ops.is_empty() {
        out.push(' ');
        out.push_str(&ops);
    }
    for kid in kids {
        out.push_str(" {");
        let mut inner = String::new();
        print_node(kid, 0, &mut inner)?;
        if inner.contains('\n') {
            out.push('\n');
            for line in inner.lines() {
                out.push_str(&pad(indent + 1));
                out.push_str(line.trim_start());
                out.push('\n');
            }
            out.push_str(&pad(indent));
            out.push('}');
        } else {
            out.push(' ');
            out.push_str(&inner);
            out.push_str(" }");
        }
    }
    Ok(())
}

fn ir_parts(node: &dyn IRNode) -> Ret<(Bytecode, Vec<u8>, Vec<&dyn IRNode>)> {
    if let Some(n) = node.as_any().downcast_ref::<IRNodeBytecodes>() {
        return Ok((IRBYTECODE, n.codes.clone(), vec![]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeArray>() {
        let kids: Vec<&dyn IRNode> = n.subs.iter().map(|s| s.as_ref()).collect();
        return Ok((n.inst, vec![], kids));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeLeaf>() {
        return Ok((n.inst, vec![], vec![]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1>() {
        return Ok((n.inst, vec![n.para], vec![]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam2>() {
        return Ok((n.inst, n.para.to_vec(), vec![]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParams>() {
        return Ok((n.inst, n.para.clone(), vec![]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeSingle>() {
        return Ok((n.inst, vec![], vec![n.subx.as_ref()]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
        return Ok((n.inst, vec![], vec![n.subx.as_ref(), n.suby.as_ref()]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
        return Ok((
            n.inst,
            vec![],
            vec![n.subx.as_ref(), n.suby.as_ref(), n.subz.as_ref()],
        ));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeQuad>() {
        return Ok((
            n.inst,
            vec![],
            vec![
                n.subx.as_ref(),
                n.suby.as_ref(),
                n.subz.as_ref(),
                n.subw.as_ref(),
            ],
        ));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeQuint>() {
        return Ok((
            n.inst,
            vec![],
            vec![
                n.suba.as_ref(),
                n.subb.as_ref(),
                n.subc.as_ref(),
                n.subd.as_ref(),
                n.sube.as_ref(),
            ],
        ));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
        return Ok((n.inst, vec![n.para], vec![n.subx.as_ref()]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam2Single>() {
        return Ok((n.inst, n.para.to_vec(), vec![n.subx.as_ref()]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParamsSingle>() {
        return Ok((n.inst, n.para.clone(), vec![n.subx.as_ref()]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Double>() {
        return Ok((n.inst, vec![n.para], vec![n.subx.as_ref(), n.suby.as_ref()]));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Triple>() {
        return Ok((
            n.inst,
            vec![n.para],
            vec![n.subx.as_ref(), n.suby.as_ref(), n.subz.as_ref()],
        ));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Quad>() {
        return Ok((
            n.inst,
            vec![n.para],
            vec![
                n.subx.as_ref(),
                n.suby.as_ref(),
                n.subz.as_ref(),
                n.subw.as_ref(),
            ],
        ));
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return ir_parts(n.node.as_ref());
    }
    errf!("unsupported IR node for assembler print")
}

pub fn assemble_ircode_asm(text: &str) -> Ret<Vec<u8>> {
    check_asm_indent(text)?;
    let toks = lex_asm(text)?;
    let mut cur = TokCursor::new(text, &toks);
    let mut body = Vec::new();
    if is_outer_ir_block(&cur) {
        cur.next();
        cur.expect_lbrace()?;
        while !matches!(cur.peek(), Some(AsmTok::RBrace) | None) {
            body.extend(emit_ir_node(&mut cur, 0, 0)?);
        }
        cur.expect_rbrace()?;
        if !cur.is_empty() {
            return errf!("trailing tokens after ir_block");
        }
    } else {
        while !cur.is_empty() {
            body.extend(emit_ir_node(&mut cur, 0, 0)?);
        }
    }
    validate_ircode_body(&body)?;
    Ok(body)
}

fn is_outer_ir_block(cur: &TokCursor) -> bool {
    matches!(cur.peek(), Some(AsmTok::Ident(s)) if Bytecode::parse_intro(s) == Some(IRBLOCK))
}

fn emit_ir_node(cur: &mut TokCursor, depth: usize, loop_depth: usize) -> Ret<Vec<u8>> {
    if depth > 32 {
        return errf!("IR node over depth");
    }
    let name = match cur.next().map(|t| &t.tok) {
        Some(AsmTok::Ident(s)) => s.clone(),
        other => return errf!("expected instruction, got {:?}", other),
    };
    let Some(inst) = Bytecode::parse_intro(&name) else {
        return errf!("bytecode {} not found", name);
    };
    if matches!(inst, IRBREAK | IRCONTINUE) && loop_depth == 0 {
        return errf!("{:?} is only valid inside a while loop body", inst);
    }
    match inst {
        IRBYTECODE => {
            let (inner, next) = cur.brace_inner_src()?;
            cur.set_pos(next);
            let codes = assemble_bytecode_asm_nested(inner)?;
            let node = IRNodeBytecodes::new(codes).map_err(|e| Error::normal(e.to_string()))?;
            let mut out = vec![IRBYTECODE as u8];
            out.extend_from_slice(&(node.codes.len() as u16).to_be_bytes());
            out.extend_from_slice(&node.codes);
            Ok(out)
        }
        IRLIST | IRBLOCK | IRBLOCKR => {
            cur.expect_lbrace()?;
            let mut kids = Vec::new();
            while !matches!(cur.peek(), Some(AsmTok::RBrace) | None) {
                kids.push(emit_ir_node(cur, depth + 1, loop_depth)?);
            }
            cur.expect_rbrace()?;
            if inst == IRBLOCKR && kids.is_empty() {
                return errf!("empty block expr");
            }
            if kids.len() > u16::MAX as usize {
                return errf!("too many IR children");
            }
            let mut out = vec![inst as u8];
            out.extend_from_slice(&(kids.len() as u16).to_be_bytes());
            for k in kids {
                out.extend(k);
            }
            Ok(out)
        }
        IRIF | IRIFR => {
            let a = emit_braced_child(cur, depth + 1, loop_depth)?;
            let b = emit_braced_child(cur, depth + 1, loop_depth)?;
            let c = emit_braced_child(cur, depth + 1, loop_depth)?;
            let mut out = vec![inst as u8];
            out.extend(a);
            out.extend(b);
            out.extend(c);
            Ok(out)
        }
        IRWHILE => {
            let cond = emit_braced_child(cur, depth + 1, loop_depth)?;
            let body = emit_braced_child(cur, depth + 1, loop_depth + 1)?;
            let mut out = vec![inst as u8];
            out.extend(cond);
            out.extend(body);
            Ok(out)
        }
        _ => {
            let meta = inst.metadata();
            if !meta.valid {
                return errf!("bytecode {} not found", inst as u8);
            }
            let kids = match meta.input {
                0 | 255 => 0usize,
                n @ 1..=5 => n as usize,
                i => return errf!("invalid irnode {:?} of ps={} i={}", inst, meta.param, i),
            };
            let imm = if meta.param == 0 {
                vec![]
            } else {
                parse_ir_operands(cur, inst)?
            };
            if kids == 0 {
                let mut out = vec![inst as u8];
                out.extend(imm);
                Ok(out)
            } else {
                emit_prefix_kids(inst, imm, kids, cur, depth, loop_depth)
            }
        }
    }
}

fn emit_prefix_kids(
    inst: Bytecode,
    imm: Vec<u8>,
    n: usize,
    cur: &mut TokCursor,
    depth: usize,
    loop_depth: usize,
) -> Ret<Vec<u8>> {
    let mut out = vec![inst as u8];
    out.extend(imm);
    for _ in 0..n {
        out.extend(emit_braced_child(cur, depth + 1, loop_depth)?);
    }
    Ok(out)
}

fn emit_braced_child(cur: &mut TokCursor, depth: usize, loop_depth: usize) -> Ret<Vec<u8>> {
    cur.expect_lbrace()?;
    let bytes = emit_ir_node(cur, depth, loop_depth)?;
    cur.expect_rbrace()?;
    Ok(bytes)
}
