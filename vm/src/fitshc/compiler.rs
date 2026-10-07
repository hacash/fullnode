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

    /// The motivating case: abstract (and function) bodies may end without a
    /// hand-written `end`; the compiler appends the terminator implicitly.
    #[test]
    fn abstract_body_without_trailing_end_compiles() {
        let src = r#"
pragma fitsh 1.0.0
contract PermitProbe {
    function _check() {
        print(1)
    }
    abstract PermitHACD(to: address, count: u32, names: bytes) {
        self._check()
    }
}
"#;
        crate::fitshc::compile(src).expect("body without trailing end should compile");
    }

    /// Every "fall-off-end" shape the HVM terminal check (`CodeNotWithEnd`)
    /// rejected before must now compile via the auto-appended `end`.
    #[test]
    fn auto_end_covers_all_statement_shapes() {
        // (body, expect_ok) — none of the bodies carry a trailing `end`.
        let cases: Vec<(&str, bool)> = vec![
            ("self._check()", true),
            ("", true),
            ("var z = 1", true),
            ("1 + 2", true),
            ("print(1)", true),
            ("log(1, 2)", true),
            ("assert(x > 0)", true),
            ("abort", true),
            ("throw 1", true),
            ("return x", true),
            ("bind y = x + 1", true),
            ("if x > 0 { return 1 }", true),
            ("while x > 0 { x = x - 1 }", true),
            ("{ return x }", true),
            ("bytecode { NOP }", true),
            // Pre-existing compile_if limitation, unrelated to auto-end: when
            // the else branch terminates unconditionally, its dead JMPSL jump
            // (skipping the then branch) targets one past the code end and the
            // jump verifier rejects it.
            ("if x > 0 { return 1 } else { return 2 }", false),
        ];
        for (i, (body, expect_ok)) in cases.iter().enumerate() {
            let src = format!(
                "pragma fitsh 1.0.0\ncontract AutoEndProbe{i} {{\n    function f(x: u32) -> u32 {{\n        {body}\n    }}\n}}\n"
            );
            match crate::fitshc::compile(&src) {
                Ok(_) => assert!(*expect_ok, "case {i} ({body}) should not have compiled"),
                Err(e) => assert!(!*expect_ok, "case {i} ({body}) failed to compile: {e}"),
            }
        }
    }

}
