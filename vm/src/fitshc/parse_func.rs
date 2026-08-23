use super::compile_body::{CompiledCode, compile_body};
use super::state::ParseState;
use crate::Token::*;
use crate::contract::Func;
use crate::rt::SourceMap;
use crate::rt::{KwTy, Token};
use crate::value::ValueTy;
use sys::*;
use sys::{Ret, errf};

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
    let (irnodes, compiled, source_map) =
        compile_body(body_tokens, args, &state.libs, &state.consts, is_ircode)?;

    func = match compiled {
        CompiledCode::IrCode(_) => func.irnode(irnodes)?,
        CompiledCode::Bytecode(bts) => func.bytecode(bts)?,
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
}
