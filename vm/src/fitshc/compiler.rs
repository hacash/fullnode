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
    fn function_code(src: &str, name: &str) -> Vec<u8> {
        let (contract, _, _, _) = crate::fitshc::compile(src).unwrap();
        let sto = contract.into_sto();
        let func = sto
            .userfuncs
            .iter()
            .find(|f| f.sign.0 == crate::rt::calc_func_sign(name))
            .expect("compiled function");
        func.code_stuff.data.to_vec()
    }

    #[test]
    fn statement_macro_is_hygienic_and_byte_identical() {
        let handwritten = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    function external probe(a: u64, b: u64) -> u64 {
        var c = a + 1
        var balances = c + b
        var out = balances
        var c2 = a + 1
        var balances2 = c2 + b
        var out2 = balances2
        return out + out2
    }
}
"#;
        let expanded = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    define liquidity($a, $b, $out) {
        var c = $a + 1
        var balances = c + $b
        var $out = balances
    }
    function external probe(a: u64, b: u64) -> u64 {
        macro liquidity(a, b, out)
        macro liquidity(a, b, out2)
        return out + out2
    }
}
"#;
        assert_eq!(
            function_code(handwritten, "probe"),
            function_code(expanded, "probe")
        );
    }

    #[test]
    fn statement_macro_private_name_avoids_caller_identifier_collision() {
        let handwritten = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    function external probe(a: u64, b: u64) -> u64 {
        var c__macro_0 = a
        var c = a + 1
        var balances = c + b
        var out = balances
        return c__macro_0 + out
    }
}
"#;
        let expanded = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    define liquidity($a, $b, $out) {
        var c = $a + 1
        var balances = c + $b
        var $out = balances
    }
    function external probe(a: u64, b: u64) -> u64 {
        var c__macro_0 = a
        macro liquidity(a, b, out)
        return c__macro_0 + out
    }
}
"#;
        assert_eq!(
            function_code(handwritten, "probe"),
            function_code(expanded, "probe")
        );
    }

    #[test]
    fn statement_macro_bind_name_avoids_parameter_and_const_collisions() {
        let handwritten = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    const scratch__macro_0 = 99
    function external probe(scratch__macro_0_1: u64) -> u64 {
        var result = 7
        return result
    }
}
"#;
        let expanded = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    const scratch__macro_0 = 99
    define read($out) {
        bind scratch = 7
        bind value = scratch
        var $out = value
    }
    function external probe(scratch__macro_0_1: u64) -> u64 {
        macro read(result)
        return result
    }
}
"#;
        assert_eq!(
            function_code(handwritten, "probe"),
            function_code(expanded, "probe")
        );
    }

    #[test]
    fn statement_macro_rejects_expression_arguments() {
        let src = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    define identity($x) { var y = $x }
    function external probe(a: u64) { macro identity(a + 1) }
}
"#;
        let error = match crate::fitshc::compile(src) {
            Ok(_) => panic!("expected macro argument error"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("arguments must be variable names"),
            "{error}"
        );
    }

    #[test]
    fn statement_macro_recursion_is_bounded() {
        let src = r#"
pragma fitsh 1.0.0
contract MacroProbe {
    define recurse() { macro recurse() }
    function external probe() { macro recurse() }
}
"#;
        let error = match crate::fitshc::compile(src) {
            Ok(_) => panic!("expected recursion limit error"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("expansion limit exceeded"), "{error}");
    }

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
        require 1001, true
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
            ("require 1001, true", true),
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
            ("if x > 0 { return 1 } else { return 2 }", true),
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

    #[test]
    fn both_return_if_compiles_as_bytecode_and_irnode() {
        for modifier in ["", "ircode"] {
            let src = format!(
                "pragma fitsh 1.0.0\ncontract ExitProbe {{\n    function {modifier} f(x: u32) -> u32 {{\n        if x > 0 {{ return 1 }} else {{ return 2 }}\n    }}\n}}\n"
            );
            crate::fitshc::compile(&src).unwrap_or_else(|e| panic!("{modifier}: {e}"));
        }
    }
}
