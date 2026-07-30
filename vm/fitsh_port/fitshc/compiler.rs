use sys::*;

use super::parse_deploy::DeployInfo;
use super::parse_top::parse_top_level;
use super::state::ParseState;
use crate::contract::Contract;
use crate::lang::Tokenizer;
use crate::rt::SourceMap;

pub type FitshCompileOutput = (
    Contract,
    Option<DeployInfo>,
    Vec<(String, SourceMap)>,
    String,
);

pub fn compile_with_warnings(code: &str) -> Ret<(FitshCompileOutput, Vec<String>)> {
    let tkr = Tokenizer::new(code.as_bytes());
    let tokens = tkr.parse().map_err(|e| e.to_string())?;
    let mut state = ParseState::new(tokens);

    parse_top_level(&mut state)?;
    if state.idx != state.max {
        return errf!(
            "unexpected token after contract end: {:?}",
            state.current().cloned()
        );
    }

    let warnings = std::mem::take(&mut state.warnings);
    Ok((
        (
            state.contract,
            state.deploy,
            state.source_maps,
            state.contract_name,
        ),
        warnings,
    ))
}

pub fn compile(code: &str) -> Ret<FitshCompileOutput> {
    let (output, _) = compile_with_warnings(code)?;
    Ok(output)
}
