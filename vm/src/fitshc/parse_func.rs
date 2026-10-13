use super::compile_body::{compile_body_with_macros, CompiledCode};
use super::state::ParseState;
use crate::contract::Func;
use crate::rt::SourceMap;
use crate::rt::{KwTy, Token};
use crate::value::ValueTy;
use crate::Token::*;
use sys::*;
use sys::{errf, Ret};

fn parse_type(state: &mut ParseState) -> Ret<Option<ValueTy>> {
    if state.idx >= state.max {
        return Ok(None);
    }
    let tk = &state.tokens[state.idx];
    let ty = if let Keyword(k) = tk {
        match k {
            KwTy::U8 => Some(ValueTy::U8),
            KwTy::U16 => Some(ValueTy::U16),
            KwTy::U32 => Some(ValueTy::U32),
            KwTy::U64 => Some(ValueTy::U64),
            KwTy::U128 => Some(ValueTy::U128),
            KwTy::Address => Some(ValueTy::Address),
            KwTy::Bytes => Some(ValueTy::Bytes),
            KwTy::Bool => Some(ValueTy::Bool),
            KwTy::Tuple => Some(ValueTy::Tuple),
            KwTy::List | KwTy::Map => Some(ValueTy::Compo),
            _ => None,
        }
    } else if let Identifier(tn) = tk {
        match tn.as_str() {
            "u8" => Some(ValueTy::U8),
            "u16" => Some(ValueTy::U16),
            "u32" => Some(ValueTy::U32),
            "u64" => Some(ValueTy::U64),
            "u128" => Some(ValueTy::U128),
            "address" => Some(ValueTy::Address),
            "bytes" => Some(ValueTy::Bytes),
            "bool" => Some(ValueTy::Bool),
            "tuple" => Some(ValueTy::Tuple),
            "list" | "map" => Some(ValueTy::Compo),
            _ => None,
        }
    } else {
        None
    };

    if ty.is_some() {
        state.advance();
    }
    Ok(ty)
}

fn reserved_modifier_name(kw: KwTy) -> Option<&'static str> {
    match kw {
        KwTy::Virtual => Some("virtual"),
        KwTy::Inner => Some("inner"),
        KwTy::View => Some("view"),
        KwTy::Pure => Some("pure"),
        KwTy::Struct => Some("struct"),
        _ => None,
    }
}

fn reject_reserved_modifier(kw: KwTy, context: &str) -> Rerr {
    if let Some(name) = reserved_modifier_name(kw) {
        return errf!("reserved {} modifier '{}' is not supported", context, name);
    }
    Ok(())
}

pub(super) fn parse_optional_code_modifier(state: &mut ParseState, context: &str) -> Ret<bool> {
    let is_ircode = match state.current() {
        Some(Keyword(KwTy::IrCode)) => {
            state.advance();
            true
        }
        Some(Keyword(KwTy::ByteCode)) => {
            state.advance();
            false
        }
        Some(Keyword(kw)) => {
            reject_reserved_modifier(*kw, context)?;
            false
        }
        _ => false,
    };
    match state.current() {
        Some(Keyword(KwTy::IrCode)) | Some(Keyword(KwTy::ByteCode)) => {
            errf!("{} code modifier must appear at most once", context)
        }
        Some(Keyword(kw)) => {
            reject_reserved_modifier(*kw, context)?;
            Ok(is_ircode)
        }
        _ => Ok(is_ircode),
    }
}

fn asm_token_text(token: &Token) -> Ret<String> {
    Ok(match token {
        Token::Identifier(s) => s.clone(),
        Token::Integer(n) => n.to_string(),
        Token::Bytes(bytes) => format!("0x{}", hex::encode(bytes)),
        Token::Keyword(k) => {
            let name = format!("{:?}", k);
            let mut out = String::new();
            for (i, c) in name.chars().enumerate() {
                if c.is_ascii_uppercase() && i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            }
            out
        }
        Token::Operator(op) => op.symbol().to_string(),
        Token::Partition(c) => c.to_string(),
        other => return errf!("invalid token in raw bytecode function: {:?}", other),
    })
}

fn raw_bytecode_function(body: &[Token]) -> Ret<Vec<u8>> {
    if body.is_empty() {
        return errf!("raw bytecode function body cannot be empty");
    }

    // Function bodies have already passed through the FitSH tokenizer. Rebuild
    // a token-preserving assembly stream; the assembler then selects raw bytes
    // or readable mnemonics using its existing format detection.
    let mut asm = String::new();
    let mut previous: Option<&Token> = None;
    for token in body {
        let text = asm_token_text(token)?;
        let attach = matches!(
            token,
            Token::Partition(',' | ':' | ')' | ']') | Token::Keyword(KwTy::Dot)
        ) || matches!(
            previous,
            Some(Token::Keyword(KwTy::Dot) | Token::Partition('(' | '['))
        );
        if !asm.is_empty() && !attach {
            asm.push(' ');
        }
        asm.push_str(&text);
        previous = Some(token);
    }

    let codes = crate::rt::assemble_bytecode(&asm)?;
    crate::rt::verify_bytecodes(&codes).map_err(|e| e.to_string())?;
    Ok(codes)
}

pub fn parse_function(state: &mut ParseState, consume_kw: bool) -> Ret<(Func, SourceMap, String)> {
    // function external/ircode Name(...) -> Ret { ... }
    if consume_kw {
        state.advance(); // function
    }

    let is_external = if matches!(state.current(), Some(Keyword(KwTy::External))) {
        state.advance();
        true
    } else {
        false
    };
    let is_raw_bytecode = matches!(state.current(), Some(Keyword(KwTy::ByteCode)));
    let is_ircode = parse_optional_code_modifier(state, "function")?;
    if matches!(state.current(), Some(Keyword(KwTy::External))) {
        return errf!("function modifier 'external' must appear before code modifier");
    }

    let (name, args, ret_ty) = parse_func_sig(state)?;
    let body_tokens = parse_func_body_tokens(state)?;

    // Setup Func
    let mut func = Func::new(&name)?;
    if is_external {
        func = func.external();
    }

    let arg_types: Vec<ValueTy> = args.iter().map(|(_, t)| *t).collect();
    func = func.types(ret_ty, arg_types);

    // Compile body using shared compile function
    // A function-level `bytecode` modifier makes the body a complete assembly
    // stream, so no FitSH parameter prelude or local-slot allocation is added.
    let source_map = if is_raw_bytecode {
        let codes = raw_bytecode_function(&body_tokens)?;
        func = func.bytecode(codes)?;
        SourceMap::default()
    } else {
        let (irnodes, compiled, source_map) =
            compile_body_with_macros(
                body_tokens,
                args,
                &state.libs,
                &state.consts,
                is_ircode,
                &state.macros,
            )?;

        func = match compiled {
            CompiledCode::IrCode(_) => func.irnode(irnodes)?,
            CompiledCode::Bytecode(bts) => func.bytecode(bts)?,
        };
        source_map
    };

    Ok((func, source_map, name))
}

pub fn parse_func_sig(
    state: &mut ParseState,
) -> Ret<(String, Vec<(String, ValueTy)>, Option<ValueTy>)> {
    // Name(args?) -> Ret
    let name = if let Some(Identifier(n)) = state.current() {
        let n = n.clone();
        state.advance();
        n
    } else {
        return errf!("expected function name but got {:?}", state.current());
    };

    let mut args = Vec::new();
    if let Some(Partition('(')) = state.current() {
        state.advance();
        loop {
            while let Some(Partition(',')) = state.current() {
                state.advance();
            }
            if let Some(Partition(')')) = state.current() {
                state.advance();
                break;
            }
            if state.idx >= state.max {
                return errf!("args list not closed (missing ')')");
            }

            // arg: type
            let arg_name = if let Some(Identifier(n)) = state.current() {
                let n = n.clone();
                state.advance();
                n
            } else {
                return errf!("expected arg name");
            };

            // :
            if let Some(Keyword(KwTy::Colon)) = state.current() {
                state.advance();
            } else if let Some(Partition(':')) = state.current() {
                state.advance();
            } else {
                return errf!("expected ':' after arg name");
            }

            // type
            let rtype = parse_type(state)?;
            let aty = match rtype {
                Some(t) => t,
                None => return errf!("unknown type"),
            };
            aty.check_func_argv_type()?;
            args.push((arg_name, aty));

            // comma
            if let Some(Partition(',')) = state.current() {
                state.advance();
            }
        }
    }

    // -> Ret
    let mut ret_ty = None;
    if let Some(Keyword(KwTy::Arrow)) = state.current() {
        state.advance();

        // Optional parenthesized return type, but if '(' appears then ')' is required.
        let mut wrapped = false;
        if let Some(Partition('(')) = state.current() {
            state.advance();
            wrapped = true;
        }

        if state.idx >= state.max {
            return errf!("expected return type");
        }
        let rtype = parse_type(state)?;

        ret_ty = match rtype {
            Some(t) => Some(t),
            None => return errf!("unknown return type"),
        };

        if wrapped {
            if let Some(Partition(')')) = state.current() {
                state.advance();
            } else {
                return errf!("expected ')' after return type");
            }
        }
    }

    Ok((name, args, ret_ty))
}

pub fn parse_func_body_tokens(state: &mut ParseState) -> Ret<Vec<Token>> {
    state.eat_partition('{')?;
    let mut inner = Vec::new();
    let mut depth = 1;
    while state.idx < state.max {
        let t = &state.tokens[state.idx];
        if let Partition('{') = t {
            depth += 1;
        }
        if let Partition('}') = t {
            depth -= 1;
        }
        if depth == 0 {
            state.advance(); // consume closing }
            return Ok(inner);
        }
        inner.push(t.clone());
        state.advance();
    }
    errf!("bracket mismatch")
}

#[cfg(test)]
mod parse_func_tests {
    use super::*;
    use crate::lang::Tokenizer;

    fn parse_sig(src: &str) -> Ret<(Func, SourceMap, String)> {
        let tokens = Tokenizer::new(src.as_bytes()).parse()?;
        let mut state = ParseState::new(tokens);
        parse_function(&mut state, true)
    }

    #[test]
    fn parse_function_rejects_unknown_argument_type() {
        let err = match parse_sig("function probe(a: u256) { return a }") {
            Ok(_) => panic!("expected unknown type error"),
            Err(err) => err,
        };
        assert!(err.contains("unknown type"), "{}", err);
    }

    #[test]
    fn parse_function_rejects_unknown_return_type() {
        let err = match parse_sig("function probe() -> uint { return 1 }") {
            Ok(_) => panic!("expected unknown type error"),
            Err(err) => err,
        };
        assert!(err.contains("unknown return type"), "{}", err);
    }

    fn assemble_body(src: &str) -> Ret<Vec<u8>> {
        let tokens = Tokenizer::new(src.as_bytes()).parse()?;
        raw_bytecode_function(&tokens)
    }

    fn compiled_bytecode(src: &str) -> Ret<Vec<u8>> {
        let (func, _, _) = parse_sig(src)?;
        let sto = crate::contract::Contract::new().func(func).into_sto();
        Ok(sto.userfuncs[0].code_stuff.data.to_vec())
    }

    #[test]
    fn raw_bytecode_function_accepts_machine_bytes_and_mnemonics() {
        let machine = assemble_body("0x1200e85fde5d").unwrap();
        let raw_mnemonic = assemble_body("CODE_CALL 0 0xe85fde5d").unwrap();
        let readable_mnemonic = assemble_body("code_call 0, 0xe85fde5d").unwrap();
        let expected = vec![0x12, 0x00, 0xe8, 0x5f, 0xde, 0x5d];

        assert_eq!(machine, expected);
        assert_eq!(raw_mnemonic, expected);
        assert_eq!(readable_mnemonic, expected);
    }

    #[test]
    fn bytecode_function_parses_direct_assembly_body() {
        let expected = vec![0x12, 0x00, 0xe8, 0x5f, 0xde, 0x5d];
        assert_eq!(
            compiled_bytecode("function bytecode open() { CODE_CALL 0 0xe85fde5d }").unwrap(),
            expected
        );
        assert_eq!(
            compiled_bytecode("function bytecode open() { 0x1200e85fde5d }").unwrap(),
            expected
        );
    }

    #[test]
    fn ircode_modifier_still_emits_ir_node_code() {
        let (func, _, _) =
            parse_sig("function ircode probe(value: u64) -> u64 { return value }").unwrap();
        let sto = crate::contract::Contract::new().func(func).into_sto();
        let code_stuff = &sto.userfuncs[0].code_stuff;

        assert!(matches!(
            code_stuff.parse_conf().unwrap().code_type(),
            crate::rt::CodeType::IRNode
        ));
        assert!(!code_stuff.data.is_empty());
    }
}
