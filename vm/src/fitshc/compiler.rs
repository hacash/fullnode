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

#[cfg(test)]
mod compiler_tests {
    #[test]
    fn ternary_external_function_compiles() {
        let src = r#"
pragma fitsh 1.0.0
contract TernaryProbe {
    function external pick(x: bool) -> u8 {
        return x ? 1 : 2
    }
}
"#;
        crate::fitshc::compile(src).expect("ternary contract should compile");
    }

    #[test]
    fn ternary_and_choose_both_compile_in_one_contract() {
        let src = r#"
pragma fitsh 1.0.0
contract TernaryChoose {
    function external pick_q(x: bool) -> u8 {
        return x ? 1 : 2
    }
    function external pick_c(x: bool) -> u8 {
        return choose(x, 1, 2)
    }
}
"#;
        crate::fitshc::compile(src).expect("ternary and choose in one contract");
    }

    #[test]
    fn sha2_prefix_u32_list_as_bytes_compiles() {
        let src = r#"
pragma fitsh 1.0.0
contract Check4Prefix {
    function ircode check4(
        dir: u8, asset: u8, token: u8, chain: u32, slot: u32,
        recipient: address, units: u64, names: bytes
    ) -> bytes {
        return sha2_prefix_u32(["HBX1", dir, asset, token, chain, slot, recipient, units, names]) as bytes
    }
}
"#;
        crate::fitshc::compile(src).expect("sha2_prefix_u32 list should compile");
    }

}
