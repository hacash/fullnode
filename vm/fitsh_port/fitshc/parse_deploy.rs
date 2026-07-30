use super::state::ParseState;
use crate::Token::*;
use crate::rt::*;
use field::{Amount, BytesW2, Uint4};
use std::collections::HashSet;
use sys::*;
use sys::{Ret, errf};

#[derive(Default, Debug, Clone)]
pub struct DeployInfo {
    pub protocol_cost: Option<Amount>,
    pub nonce: Option<Uint4>,
    pub construct_argv: Option<BytesW2>,
    pub matches: bool,
}

pub fn parse_deploy(state: &mut ParseState) -> Ret<DeployInfo> {
    state.advance(); // consume deploy
    state.eat_partition('{')?;

    let mut info = DeployInfo::default();
    info.matches = true;
    let mut seen = HashSet::new();

    loop {
        if let Some(Partition('}')) = state.current() {
            state.advance();
            break;
        }
        if matches!(state.current(), Some(Partition(','))) {
            return errf!("unexpected deploy separator");
        }

        let key = if let Some(Identifier(n)) = state.current() {
            let n = n.clone();
            state.advance();
            n
        } else {
            return errf!("expected deploy key");
        };
        if !seen.insert(key.clone()) {
            return errf!("duplicate deploy field '{}'", key);
        }

        if let Some(Keyword(KwTy::Colon)) = state.current() {
            state.advance();
        } else {
            return errf!("expected ':' after deploy field '{}'", key);
        }

        if key == "protocol_cost" {
            info.protocol_cost = Some(parse_amount_ctor(state)?);
        } else if key == "nonce" {
            info.nonce = Some(Uint4::from(parse_nonce(state)?));
        } else if key == "construct_argv" {
            if let Some(Bytes(v)) = state.current() {
                info.construct_argv = Some(BytesW2::from(v.clone()).map_err(|e| e.to_string())?);
                state.advance();
            } else {
                return errf!("expected bytes literal at construct_argv");
            }
        } else {
            return errf!("unknown deploy field '{}'", key);
        }

        if matches!(state.current(), Some(Partition(','))) {
            state.advance();
            if matches!(state.current(), Some(Partition(','))) {
                return errf!("duplicate deploy separator");
            }
        }
    }

    Ok(info)
}

fn parse_amount_ctor(state: &mut ParseState) -> Ret<Amount> {
    match state.current() {
        Some(Identifier(id)) if id == "amount" => state.advance(),
        _ => return errf!("expected amount(\"...\") at protocol_cost"),
    }
    state.eat_partition('(')?;
    let Some(Bytes(raw)) = state.current() else {
        return errf!("expected amount string at protocol_cost");
    };
    let text = String::from_utf8(raw.clone())
        .map_err(|_| "amount string must be ascii bytes".to_string())?;
    state.advance();
    state.eat_partition(')')?;
    Amount::from(text.as_str()).map_err(|e| e.to_string())
}

fn parse_nonce(state: &mut ParseState) -> Ret<u32> {
    match state.current() {
        Some(Integer(v)) => {
            let n = u32::try_from(*v).map_err(|_| format!("nonce overflow: {}", v))?;
            state.advance();
            Ok(n)
        }
        Some(IntegerWithSuffix(v, KwTy::U32)) => {
            let n = u32::try_from(*v).map_err(|_| format!("nonce overflow: {}", v))?;
            state.advance();
            Ok(n)
        }
        Some(IntegerWithSuffix(_, _)) => errf!("nonce type must be u32"),
        _ => errf!("expected nonce integer"),
    }
}

