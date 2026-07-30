use sys::*;

use field::Address as FieldAddress;

use super::ir::*;
use super::rt::Token::*;
use super::rt::*;
use super::*;
// use super::rt::TokenType::*;

use super::native::*;

include! {"print_option.rs"}
include! {"decompilation_helper.rs"}
include! {"ir_payload.rs"}
include! {"ir_literal.rs"}

pub mod syntax;
pub use syntax::Syntax;

include! {"const_literal.rs"}
include! {"tokenizer.rs"}
include! {"formater.rs"}
include! {"test.rs"}

fn try_consume_display_lib_prelude(tokens: &[Token], start: usize) -> Option<usize> {
    let mut idx = start;
    if !matches!(tokens.get(idx), Some(Token::Keyword(KwTy::Lib))) {
        return None;
    }
    idx += 1;
    if !matches!(tokens.get(idx), Some(Token::Identifier(_))) {
        return None;
    }
    idx += 1;
    if !matches!(tokens.get(idx), Some(Token::Keyword(KwTy::Assign))) {
        return None;
    }
    idx += 1;
    if !matches!(tokens.get(idx), Some(Token::Integer(_))) {
        return None;
    }
    idx += 1;
    if matches!(
        tokens.get(idx),
        Some(Token::Keyword(KwTy::Colon)) | Some(Token::Partition(':'))
    ) {
        idx += 1;
        if !matches!(
            tokens.get(idx),
            Some(Token::Address(_)) | Some(Token::Identifier(_))
        ) {
            return None;
        }
        idx += 1;
    }
    Some(idx)
}

fn try_consume_display_const_prelude(tokens: &[Token], start: usize) -> Option<usize> {
    let mut idx = start;
    if !matches!(tokens.get(idx), Some(Token::Keyword(KwTy::Const))) {
        return None;
    }
    idx += 1;
    if !matches!(tokens.get(idx), Some(Token::Identifier(_))) {
        return None;
    }
    idx += 1;
    if !matches!(tokens.get(idx), Some(Token::Keyword(KwTy::Assign))) {
        return None;
    }
    idx += 1;
    if !matches!(
        tokens.get(idx),
        Some(Token::Integer(_)) | Some(Token::Bytes(_)) | Some(Token::Address(_))
    ) {
        return None;
    }
    Some(idx + 1)
}

fn skip_display_prelude(tokens: &[Token]) -> usize {
    let mut idx = 0usize;
    loop {
        if let Some(next) = try_consume_display_lib_prelude(tokens, idx) {
            idx = next;
            continue;
        }
        if let Some(next) = try_consume_display_const_prelude(tokens, idx) {
            idx = next;
            continue;
        }
        break;
    }
    idx
}

fn strip_display_root_block(tokens: &mut Vec<Token>) {
    let body_start = skip_display_prelude(tokens);
    if tokens.len() < body_start + 2 {
        return;
    }
    if !matches!(tokens.get(body_start), Some(Token::Partition('{'))) {
        return;
    }
    if !matches!(tokens.last(), Some(Token::Partition('}'))) {
        return;
    }
    let mut depth: isize = 0;
    let mut close_at: Option<usize> = None;
    for (i, tk) in tokens.iter().enumerate().skip(body_start) {
        match tk {
            Token::Partition('{') => depth += 1,
            Token::Partition('}') => {
                depth -= 1;
                if depth == 0 {
                    close_at = Some(i);
                    break;
                }
                if depth < 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    if close_at != Some(tokens.len() - 1) {
        return;
    }
    let mut stripped = Vec::with_capacity(tokens.len() - 2);
    stripped.extend(tokens[..body_start].iter().cloned());
    stripped.extend(tokens[body_start + 1..tokens.len() - 1].iter().cloned());
    *tokens = stripped;
}

pub fn lang_to_irnode_with_sourcemap(langscript: &str) -> Ret<(IRNodeArray, SourceMap)> {
    let tkr = Tokenizer::new(langscript.as_bytes());
    let mut tks = tkr.parse()?;
    // The formatter may emit a file-level `{ ... }` wrapper when `trim_root_block` is disabled.
    // That wrapper can appear after file-level `lib ...` / `const ...` prelude lines, so skip
    // those declarations before deciding whether the outermost braces are presentation-only.
    strip_display_root_block(&mut tks);
    let syx = Syntax::new(tks);
    syx.parse()
}

pub fn lang_to_irnode(langscript: &str) -> Ret<IRNodeArray> {
    let (block, _) = lang_to_irnode_with_sourcemap(langscript)?;
    Ok(block)
}

pub fn lang_to_ircode(langscript: &str) -> Ret<Vec<u8>> {
    let ir = lang_to_irnode(langscript)?;
    drop_irblock_wrap(ir.serialize())
}

pub fn lang_to_ircode_with_sourcemap(langscript: &str) -> Ret<(Vec<u8>, SourceMap)> {
    let (ir, smap) = lang_to_irnode_with_sourcemap(langscript)?;
    Ok((drop_irblock_wrap(ir.serialize())?, smap))
}

pub fn irnode_to_lang_with_sourcemap(block: IRNodeArray, smap: &SourceMap) -> Ret<String> {
    let mut opt = PrintOption::new("  ", 0);
    opt.map = Some(smap);
    Ok(Formater::new(&opt).print(&block))
}

pub fn irnode_to_lang(block: IRNodeArray) -> Ret<String> {
    let mut opt = PrintOption::new("  ", 0);
    opt.recover_literals = true;
    opt.simplify_numeric_as_suffix = true;
    Ok(Formater::new(&opt).print(&block))
}

pub fn format_ircode_to_lang(ircode: &Vec<u8>, map: Option<&SourceMap>) -> VmrtRes<String> {
    let mut seek = 0;
    let block = parse_ir_block(ircode, &mut seek)?;
    let mut opt = PrintOption::new("  ", 0);
    if let Some(map) = map {
        opt.map = Some(map);
    }
    opt.trim_root_block = true;
    opt.trim_head_alloc = true;
    opt.trim_param_unpack = true;
    opt.hide_default_call_argv = true;
    opt.call_short_syntax = true;
    opt.flatten_call_list = true;
    opt.flatten_array_list = true;
    opt.flatten_syscall_cat = true;
    opt.recover_literals = true;
    opt.simplify_numeric_as_suffix = true;
    Ok(Formater::new(&opt).print(&block))
}

pub fn ircode_to_lang_with_sourcemap(ircode: &Vec<u8>, smap: &SourceMap) -> Ret<String> {
    let mut seek = 0;
    let block = parse_ir_block(ircode, &mut seek)?;
    irnode_to_lang_with_sourcemap(block, smap).map_err(|e| e.to_string())
}

pub fn ircode_to_lang(ircode: &Vec<u8>) -> Ret<String> {
    let mut seek = 0;
    let block = parse_ir_block(ircode, &mut seek)?;
    irnode_to_lang(block).map_err(|e| e.to_string())
}

pub fn lang_to_bytecode(langscript: &str) -> Ret<Vec<u8>> {
    let ir = lang_to_irnode(langscript)?;
    let codes = ir.codegen()?;
    Ok(codes)
}
