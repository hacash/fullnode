use super::compile_body::{CompiledCode, compile_body};
use super::parse_deploy::parse_deploy;
use super::parse_func::{
    parse_func_body_tokens, parse_func_sig, parse_function, parse_optional_code_modifier,
};
use super::state::{FITSH_CURRENT_VERSION, FitshVersion, ParseState};
use crate::contract::Abst;
use crate::rt::Token::*;
use crate::rt::{AbstCall, KwTy, calc_func_sign};
use crate::value::ValueTy;
use sys::*;
use sys::{Ret, errf};

pub fn parse_top_level(state: &mut ParseState) -> Ret<()> {
    parse_required_pragma(state)?;
    state.skip_soft_separators();

    if !matches!(state.current(), Some(Keyword(KwTy::Contract))) {
        return errf!("expected contract declaration after pragma");
    }
    state.advance();
    let Some(Identifier(name)) = state.current() else {
        return errf!("expected contract name after 'contract'");
    };
    state.contract_name = name.clone();
    state.advance();
    state.eat_partition('{')?;

    loop {
        state.skip_soft_separators();
        if let Some(Partition('}')) = state.current() {
            state.advance();
            break;
        }
        parse_contract_body_item(state)?;
    }

    Ok(())
}

fn parse_required_pragma(state: &mut ParseState) -> Rerr {
    if !matches!(state.current(), Some(Keyword(KwTy::Pragma))) {
        return errf!(
            "expected 'pragma fitsh {}' at file start",
            FITSH_CURRENT_VERSION
        );
    }
    state.advance();

    match state.current() {
        Some(Identifier(id)) if id == "fitsh" => state.advance(),
        _ => return errf!("expected 'fitsh' after pragma"),
    }

    let version = parse_semver(state)?;
    check_pragma_version(state, version)?;
    state.version = Some(version);
    Ok(())
}

fn parse_u16_component(state: &mut ParseState, label: &str) -> Ret<u16> {
    let Some(Integer(n)) = state.current() else {
        return errf!("expected {} version number", label);
    };
    let n = u16::try_from(*n).map_err(|_| format!("{} version number overflow", label))?;
    state.advance();
    Ok(n)
}

fn parse_version_dot(state: &mut ParseState) -> Rerr {
    match state.current() {
        Some(Keyword(KwTy::Dot)) => {
            state.advance();
            Ok(())
        }
        _ => errf!("expected '.' in fitsh version"),
    }
}

fn parse_semver(state: &mut ParseState) -> Ret<FitshVersion> {
    let major = parse_u16_component(state, "major")?;
    parse_version_dot(state)?;
    let minor = parse_u16_component(state, "minor")?;
    parse_version_dot(state)?;
    let patch = parse_u16_component(state, "patch")?;
    Ok(FitshVersion::new(major, minor, patch))
}

fn check_pragma_version(state: &mut ParseState, version: FitshVersion) -> Rerr {
    let current = FITSH_CURRENT_VERSION;
    if version.major != current.major {
        return errf!(
            "unsupported fitsh major version {}; compiler supports {}",
            version,
            current
        );
    }
    if version.minor > current.minor {
        return errf!(
            "fitsh source version {} requires newer compatible features; compiler supports {}",
            version,
            current
        );
    }
    if version.patch != current.patch {
        state.warnings.push(format!(
            "fitsh patch version {} differs from compiler {}; only equivalent optimization/formatting changes are expected",
            version,
            current
        ));
    }
    Ok(())
}

fn parse_optional_const_type(state: &mut ParseState) -> Ret<Option<ValueTy>> {
    if !matches!(state.current(), Some(Keyword(KwTy::Colon))) {
        return Ok(None);
    }
    state.advance();
    let token = state
        .current()
        .cloned()
        .ok_or_else(|| "expected const type after ':'".to_string())?;
    let Some(ty) = crate::lang::parse_const_value_ty(&token) else {
        return errf!("const type invalid");
    };
    state.advance();
    Ok(Some(ty))
}

fn parse_contract_body_item(state: &mut ParseState) -> Ret<()> {
    state.skip_soft_separators();
    match state.current() {
        Some(Keyword(KwTy::Const)) => {
            // Parse top-level const: const NAME = VALUE
            state.advance(); // consume 'const'
            let name = if let Some(Identifier(n)) = state.current() {
                n.clone()
            } else {
                return errf!("expected const name after 'const'");
            };
            state.advance();
            let explicit_ty = parse_optional_const_type(state)?;

            // Expect '='
            if let Some(Keyword(KwTy::Assign)) = state.current() {
                state.advance();
            } else {
                return errf!("expected '=' after const name");
            }

            // Parse the const value
            let token = state
                .current()
                .cloned()
                .ok_or_else(|| "expected const value but got EOF".to_string())?;
            state.advance();
            let literal = crate::lang::parse_const_literal(token, explicit_ty)?;

            if state.consts.iter().any(|(n, _)| n == &name) {
                return errf!("duplicate const '{}'", name);
            }
            state.consts.push((name, literal.node));
        }
        Some(Keyword(KwTy::Deploy)) => {
            if state.deploy.is_some() {
                return errf!("duplicate deploy block");
            }
            let info = parse_deploy(state)?;
            state.deploy = Some(info);
        }
        Some(Keyword(KwTy::Library)) => {
            state.advance();
            let libs = parse_addr_list(state)?;
            for (name, addr) in libs {
                if state.libs.len() >= u8::MAX as usize {
                    return errf!("too many contract libraries: max {}", u8::MAX);
                }
                if !state.library_addrs.insert(addr) {
                    return errf!("duplicate library address");
                }
                // libidx is 0-based order
                state.libs.push((name, addr));
                state.contract = state.contract.clone().lib(addr);
            }
        }
        Some(Keyword(KwTy::Inherit)) => {
            state.advance();
            let inherit_list = parse_addr_list(state)?;
            for (_name, addr) in inherit_list {
                if state.inherit_addrs.len() >= u8::MAX as usize {
                    return errf!("too many inherit contracts: max {}", u8::MAX);
                }
                if !state.inherit_addrs.insert(addr) {
                    return errf!("duplicate inherit address");
                }
                state.contract = state.contract.clone().inh(addr);
            }
        }
        Some(Keyword(KwTy::Abstract)) => {
            state.advance(); // consume abstract

            let is_ircode = parse_optional_code_modifier(state, "abstract")?;

            let (name, args, ret_ty) = parse_func_sig(state)?;
            // return type must be integer error code if declared
            if let Some(rty) = ret_ty {
                let ok = matches!(
                    rty,
                    ValueTy::U8 | ValueTy::U16 | ValueTy::U32 | ValueTy::U64 | ValueTy::U128
                );
                if !ok {
                    return errf!(
                        "abstract '{}' return type must be integer (error code)",
                        name
                    );
                }
            }
            // parse body for abstract code
            let body_tokens = parse_func_body_tokens(state)?;
            let aid = AbstCall::from_name(&name).map_err(|e| e.to_string())?;
            // validate param types
            let expect = aid.param_types();
            if expect.len() != args.len() {
                return errf!(
                    "abstract '{}' params length mismatch: expected {}, got {}",
                    name,
                    expect.len(),
                    args.len()
                );
            }
            for (i, (_, ty)) in args.iter().enumerate() {
                if *ty != expect[i] {
                    return errf!(
                        "abstract '{}' param {} type mismatch: expected {:?}, got {:?}",
                        name,
                        i,
                        expect[i],
                        ty
                    );
                }
            }

            // compile abstract body using shared compile function
            let (_irnodes, compiled, source_map) = compile_body(
                body_tokens,
                args.clone(),
                &state.libs,
                &state.consts,
                is_ircode,
            )?;

            let abst = match compiled {
                CompiledCode::IrCode(ircodes) => Abst::new(aid).ircode(ircodes)?,
                CompiledCode::Bytecode(bts) => Abst::new(aid).bytecode(bts)?,
            };
            if !state.abst_signs.insert(aid.uint()) {
                return errf!("duplicate abstract '{}'", name);
            }
            state.contract = state.contract.clone().syst(abst);
            state
                .source_maps
                .push((format!("abstract::{}", name), source_map));
        }
        Some(Keyword(KwTy::Function)) => {
            // consume 'function' inside parse_function
            let (func, smap, name) = parse_function(state, true)?;
            let sign = calc_func_sign(&name);
            if !state.userfunc_signs.insert(sign) {
                return errf!("duplicate function '{}' signature", name);
            }
            state.contract = state.contract.clone().func(func);
            state.source_maps.push((name, smap));
        }
        Some(token) => return errf!("unexpected token in contract body: {:?}", token),
        None => return errf!("unexpected EOF while parsing contract body"),
    }
    Ok(())
}

fn parse_addr_list(state: &mut ParseState) -> Ret<Vec<(String, field::Address)>> {
    state.eat_partition('[')?;
    let mut list = vec![];
    loop {
        state.skip_soft_separators();
        if let Some(Partition(']')) = state.current() {
            state.advance();
            break;
        }
        // Name : Address
        let name = if let Some(Identifier(n)) = state.current() {
            n.clone()
        } else {
            return errf!("expected lib/inherit name");
        };
        state.advance();

        if let Some(Keyword(KwTy::Colon)) = state.current() {
            state.advance();
        } else {
            state.eat_partition(':')?;
        }

        let addr = if let Some(Identifier(a)) = state.current() {
            let adr = field::Address::from_readable(a).map_err(|e| e.to_string())?;
            state.advance();
            adr
        } else if let Some(Address(a)) = state.current() {
            let adr = a.clone();
            state.advance();
            adr
        } else {
            return errf!("expected address but got {:?}", state.current());
        };
        addr.must_contract()?;

        list.push((name, addr));

        state.skip_soft_separators();
    }
    Ok(list)
}
