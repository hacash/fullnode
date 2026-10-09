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

#[cfg(test)]
mod compile_body_tests {
    use super::*;
    use crate::lang::Tokenizer;

    #[test]
    fn parse_const_bytes_keeps_first_byte() {
        let parsed = crate::lang::parse_const_literal(Token::Bytes(vec![0x57, 0x54, 0x59]), None)
            .unwrap()
            .node;
        let expected = crate::ir::push_bytes(&vec![0x57, 0x54, 0x59]).unwrap();
        assert_eq!(parsed.serialize(), expected.serialize());
    }

    #[test]
    fn rejects_contract_lib_count_overflow() {
        let body_tokens = Tokenizer::new(b"return 1").parse().unwrap();
        let addr = Address::from_readable("emqjNS9PscqdBpMtnC3Jfuc4mvZUPYTPS").unwrap();
        let libs: Vec<_> = (0..=u8::MAX as usize)
            .map(|idx| (format!("L{}", idx), addr.clone()))
            .collect();
        let err = match compile_body(body_tokens, vec![], &libs, &[], true) {
            Ok(_) => panic!("compile_body should fail for overflowing lib count"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("too many contract libs: max 255"));
    }

    #[test]
    fn manual_param_block_with_empty_signature_compiles() {
        let body_tokens = Tokenizer::new(b"param { a b }\nreturn a + b")
            .parse()
            .unwrap();
        let _ = compile_body(body_tokens, vec![], &[], &[], true).unwrap();
    }

    fn compile_bytes(src: &[u8]) -> Vec<u8> {
        let tokens = Tokenizer::new(src).parse().unwrap();
        match compile_body(tokens, vec![], &[], &[], false).unwrap() {
            (_, CompiledCode::Bytecode(bts), _) => bts,
            _ => unreachable!(),
        }
    }

    /// A body relying on the auto-appended `end` and the same body with the
    /// terminator hand-written must compile to identical bytecode — the
    /// appended token is absorbed whenever a terminal statement is present.
    #[test]
    fn auto_end_byte_identical_with_hand_written_terminal() {
        let pairs: Vec<(&[u8], &[u8])> = vec![
            (b"return 1", b"return 1 end"),
            (b"self._check()", b"self._check() end"),
            (b"end", b"end end"),
            (b"end", b"end end end"),
            (b"abort", b"abort end"),
            (b"throw 1", b"throw 1 end"),
            (b"require 1001, true", b"require 1001, true end"),
            (b"var z = 1", b"var z = 1 end"),
        ];
        for (auto_body, manual_body) in pairs {
            assert_eq!(
                compile_bytes(auto_body),
                compile_bytes(manual_body),
                "auto-appended end vs hand-written: {:?} vs {:?}",
                String::from_utf8_lossy(auto_body),
                String::from_utf8_lossy(manual_body),
            );
        }
    }

    /// Decompile → recompile must be a stable fixed point for bodies whose
    /// trailing `end` was auto-appended: the decompiler prints it as an
    /// explicit `end` statement, which reparses to the identical IR node.
    #[test]
    fn decompile_recompile_roundtrip_converges_with_auto_end() {
        use crate::lang::format_ircode_to_lang;
        let args = vec![("x".to_string(), ValueTy::U32)];
        let bodies: Vec<&[u8]> = vec![
            b"self._check()",
            b"self._check() end",
            b"return x",
            b"return x end",
            b"var z = x + 1",
            b"while x > 0 { x = x - 1 }",
            b"if x > 0 { return 1 }",
            b"",
            b"end",
        ];
        for body in bodies {
            let label = String::from_utf8_lossy(body).to_string();
            let tokens = Tokenizer::new(body).parse().unwrap();
            let (irnodes, _, mut smap) =
                compile_body(tokens, args.clone(), &[], &[], true)
                    .unwrap_or_else(|e| panic!("body {:?} failed to compile: {}", label, e));
            let mut ircodes = drop_irblock_wrap(irnodes.serialize())
                .unwrap_or_else(|e| panic!("body {:?} failed to serialize: {}", label, e));
            let mut converged = false;
            for _ in 0..3 {
                let text = format_ircode_to_lang(&ircodes, Some(&smap))
                    .unwrap_or_else(|e| panic!("body {:?} failed to decompile: {}", label, e));
                // Decompiled text is self-contained (it carries a `param { .. }`
                // prelude line), so recompilation must not re-inject args.
                let tokens = Tokenizer::new(text.as_bytes()).parse().unwrap();
                let (irnodes, _, next_smap) = compile_body(tokens, vec![], &[], &[], true)
                    .unwrap_or_else(|e| panic!("body {:?} recompile failed: {}", label, e));
                let next = drop_irblock_wrap(irnodes.serialize())
                    .unwrap_or_else(|e| panic!("body {:?} failed to serialize: {}", label, e));
                if next == ircodes {
                    converged = true;
                    break;
                }
                ircodes = next;
                smap = next_smap;
            }
            assert!(
                converged,
                "decompile/recompile did not converge for body {:?}",
                label
            );
        }
    }
}

/// Compile function/abstract body tokens to IR or bytecode
pub fn compile_body(
    body_tokens: Vec<Token>,
    args: Vec<(String, ValueTy)>,
    libs: &[(String, Address)],
    consts: &[(String, Box<dyn IRNode>)],
    is_ircode: bool,
) -> Ret<(IRNodeArray, CompiledCode, SourceMap)> {
    // Function/abstract bodies may omit the trailing terminator HVM requires
    // (rt::ensure_terminal_instruction): append a virtual `end` statement. The
    // parser's redundant-terminal-end skipping absorbs it whenever the body
    // already ends with `return`/`abort`/`throw`/`end`/`codecall` (or an
    // if/else whose branches all terminate), so hand-written terminators are
    // never duplicated and emitted code stays byte-identical.
    let mut body_tokens = body_tokens;
    body_tokens.push(Token::Keyword(KwTy::End));
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
