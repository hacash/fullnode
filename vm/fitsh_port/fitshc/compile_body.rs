use crate::IRNode;
use crate::ir::{IRNodeArray, convert_ir_to_runtime_bytecode, drop_irblock_wrap};
use crate::lang::Syntax;
use crate::rt::{KwTy, SourceMap, Token, verify_bytecodes};
use crate::value::ValueTy;
use dyn_clone::clone_box;
use field::Address;
use sys::Ret;

/// Compiled code result - either IR codes or bytecodes
pub enum CompiledCode {
    IrCode(Vec<u8>),
    Bytecode(Vec<u8>),
}


/// Compile function/abstract body tokens to IR or bytecode
pub fn compile_body(
    body_tokens: Vec<Token>,
    args: Vec<(String, ValueTy)>,
    libs: &[(String, Address)],
    consts: &[(String, Box<dyn IRNode>)],
    is_ircode: bool,
) -> Ret<(IRNodeArray, CompiledCode, SourceMap)> {
    let has_manual_param_block = body_tokens
        .iter()
        .any(|tk| matches!(tk, Token::Keyword(KwTy::Param)));
    let mut syntax = Syntax::new(body_tokens);

    if !libs.is_empty() {
        if libs.len() > u8::MAX as usize {
            return Err(format!("too many contract libs: max {}", u8::MAX).into());
        }
        let mut lib_entries = Vec::with_capacity(libs.len());
        for (idx, (name, addr)) in libs.iter().enumerate() {
            lib_entries.push((name.clone(), idx as u8, Some(addr.clone())));
        }
        syntax = syntax.with_libs(lib_entries);
    }

    if !consts.is_empty() {
        let mut const_nodes = Vec::with_capacity(consts.len());
        for (name, node) in consts {
            const_nodes.push((name.clone(), clone_box(node.as_ref())));
        }
        syntax = syntax.with_consts(const_nodes);
    }

    syntax = syntax.with_params(args, has_manual_param_block);

    let (irnodes, source_map) = syntax.parse()?;

    let compiled = if is_ircode {
        let ircodes = drop_irblock_wrap(irnodes.serialize())?;
        let codes = convert_ir_to_runtime_bytecode(&ircodes).map_err(|e| e.to_string())?;
        verify_bytecodes(&codes).map_err(|e| e.to_string())?;
        CompiledCode::IrCode(ircodes)
    } else {
        let bts = irnodes.codegen().map_err(|e| e.to_string())?;
        verify_bytecodes(&bts).map_err(|e| e.to_string())?;
        CompiledCode::Bytecode(bts)
    };

    Ok((irnodes, compiled, source_map))
}
