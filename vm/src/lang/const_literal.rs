#[derive(Clone)]
pub struct ConstLiteral {
    pub node: Box<dyn IRNode>,
    pub display: String,
}

pub fn parse_const_value_ty(token: &Token) -> Option<ValueTy> {
    match token {
        Token::Keyword(KwTy::Bool) => Some(ValueTy::Bool),
        Token::Keyword(KwTy::U8) => Some(ValueTy::U8),
        Token::Keyword(KwTy::U16) => Some(ValueTy::U16),
        Token::Keyword(KwTy::U32) => Some(ValueTy::U32),
        Token::Keyword(KwTy::U64) => Some(ValueTy::U64),
        Token::Keyword(KwTy::U128) => Some(ValueTy::U128),
        Token::Keyword(KwTy::Bytes) => Some(ValueTy::Bytes),
        Token::Keyword(KwTy::Address) => Some(ValueTy::Address),
        _ => None,
    }
}

fn const_token_to_node(token: &Token) -> Ret<(Box<dyn IRNode>, ValueTy, String)> {
    match token {
        Token::Integer(n) => Ok((push_num(*n), ValueTy::U128, n.to_string())),
        Token::IntegerWithSuffix(n, kw) => {
            let Some(ty) = parse_const_value_ty(&Token::Keyword(*kw)) else {
                return errf!("const literal type suffix invalid");
            };
            let mut node = push_num(*n);
            Syntax::check_literal_as_cast(node.as_ref(), ty)?;
            if crate::lang::ir_node_effective_ty(node.as_ref()) != Some(ty) {
                node = Syntax::build_cast_node(node, ty);
            }
            Ok((node, ty, format!("{}{}", n, ty.name())))
        }
        Token::Character(b) => Ok((push_num(*b as u128), ValueTy::U8, format!("'{}'", *b as char))),
        Token::Bytes(bytes) => {
            let node = push_bytes(bytes)?;
            let display = match String::from_utf8(bytes.clone()) {
                Ok(text) => format!("\"{}\"", text.escape_default()),
                Err(_) => format!("0x{}", hex::encode(bytes)),
            };
            Ok((node, ValueTy::Bytes, display))
        }
        Token::Address(addr) => Ok((push_addr(*addr), ValueTy::Address, addr.to_readable())),
        Token::Keyword(KwTy::True) => Ok((
            push_inst(Bytecode::PTRUE),
            ValueTy::Bool,
            "true".to_string(),
        )),
        Token::Keyword(KwTy::False) => Ok((
            push_inst(Bytecode::PFALSE),
            ValueTy::Bool,
            "false".to_string(),
        )),
        _ => errf!("const literal must be integer, bool, bytes, char, or address"),
    }
}

pub fn parse_const_literal(token: Token, explicit_ty: Option<ValueTy>) -> Ret<ConstLiteral> {
    let (mut node, inferred_ty, mut display) = const_token_to_node(&token)?;
    if let Some(ty) = explicit_ty {
        match ty {
            ValueTy::Bool
            | ValueTy::U8
            | ValueTy::U16
            | ValueTy::U32
            | ValueTy::U64
            | ValueTy::U128
            | ValueTy::Bytes
            | ValueTy::Address => {}
            _ => return errf!("const type '{}' is not supported", ty.name()),
        }
        Syntax::check_literal_as_cast(node.as_ref(), ty)?;
        if inferred_ty != ty && crate::lang::ir_node_effective_ty(node.as_ref()) != Some(ty) {
            node = Syntax::build_cast_node(node, ty);
        }
        display = format!("{} as {}", display, ty.name());
    }
    Ok(ConstLiteral { node, display })
}
