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

pub fn compile_with_rv64im_and_warnings(
    code: &str,
    external_rv64im: &[(&str, Vec<u8>)],
) -> Ret<(FitshCompileOutput, Vec<String>)> {
    let tkr = Tokenizer::new(code.as_bytes());
    let tokens = tkr.parse().map_err(|e| e.to_string())?;
    let mut state = ParseState::new(tokens);
    for (name, bytes) in external_rv64im {
        state.external_rv64im.insert((*name).to_string(), bytes.clone());
    }

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

pub fn compile_with_warnings(code: &str) -> Ret<(FitshCompileOutput, Vec<String>)> {
    compile_with_rv64im_and_warnings(code, &[])
}

pub fn compile_with_rv64im(
    code: &str,
    external_rv64im: &[(&str, Vec<u8>)],
) -> Ret<FitshCompileOutput> {
    let (output, _) = compile_with_rv64im_and_warnings(code, external_rv64im)?;
    Ok(output)
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

    fn echo8_container_bytes() -> Vec<u8> {
        use crate::calc::container::CalcContainer;
        let addi = |rd: u8, imm: i32| -> [u8; 4] {
            (0x13u32 | ((rd as u32) << 7) | (((imm as u32) & 0xFFF) << 20)).to_le_bytes()
        };
        let mut code = vec![0x83, 0x42, 0x05, 0x00, 0x23, 0x30, 0x56, 0x00];
        code.extend_from_slice(&addi(10, 0));
        code.extend_from_slice(&addi(11, 8));
        code.extend_from_slice(&addi(17, 0));
        code.extend_from_slice(&0x0000_0073u32.to_le_bytes());
        CalcContainer::from_code(
            crate::calc::CALC_PROFILE_RV64IM,
            8,
            crate::calc::CALC_CODE_BASE as u32,
            code,
            128,
            128,
        )
        .encode()
        .unwrap()
    }

    fn halt_text() -> Vec<u8> {
        let mut c = Vec::new();
        for w in [0x0000_0513u32, 0x0000_0593, 0x0000_0893, 0x0000_0073] {
            c.extend_from_slice(&w.to_le_bytes());
        }
        c
    }

    /// Minimal ELF64 LE RISC-V with `calc_entry` at `CODE_BASE` and one
    /// RX `PT_LOAD`. `pack_elf` projects program headers, not sections.
    fn synth_elf() -> Vec<u8> {
        let text = halt_text();
        let shstr = b"\0.text\0.symtab\0.strtab\0.shstrtab\0";
        let strtab = b"\0calc_entry\0";
        let mut sym = vec![0u8; 24];
        sym.extend_from_slice(&1u32.to_le_bytes());
        sym.extend_from_slice(&[0x12, 0]);
        sym.extend_from_slice(&1u16.to_le_bytes());
        sym.extend_from_slice(&crate::calc::container::CODE_BASE.to_le_bytes());
        sym.extend_from_slice(&0u64.to_le_bytes());

        let mut body = vec![0u8; 64 + 56];
        let text_off = body.len() as u64;
        body.extend_from_slice(&text);
        let str_off = body.len() as u64;
        body.extend_from_slice(strtab);
        let sym_off = body.len() as u64;
        body.extend_from_slice(&sym);
        let shstr_off = body.len() as u64;
        body.extend_from_slice(shstr);
        let shoff = body.len() as u64;

        body[64..68].copy_from_slice(&1u32.to_le_bytes());
        body[68..72].copy_from_slice(&5u32.to_le_bytes());
        body[72..80].copy_from_slice(&text_off.to_le_bytes());
        body[80..88].copy_from_slice(&crate::calc::container::CODE_BASE.to_le_bytes());
        body[88..96].copy_from_slice(&crate::calc::container::CODE_BASE.to_le_bytes());
        body[96..104].copy_from_slice(&(text.len() as u64).to_le_bytes());
        body[104..112].copy_from_slice(&(text.len() as u64).to_le_bytes());
        body[112..120].copy_from_slice(&1u64.to_le_bytes());

        let mut sh = |name: u32, typ: u32, flags: u64, addr: u64, off: u64, size: u64, link: u32, entsize: u64| {
            let mut h = Vec::new();
            h.extend_from_slice(&name.to_le_bytes());
            h.extend_from_slice(&typ.to_le_bytes());
            h.extend_from_slice(&flags.to_le_bytes());
            h.extend_from_slice(&addr.to_le_bytes());
            h.extend_from_slice(&off.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&link.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
            h.extend_from_slice(&4u64.to_le_bytes());
            h.extend_from_slice(&entsize.to_le_bytes());
            body.extend_from_slice(&h);
        };
        sh(0, 0, 0, 0, 0, 0, 0, 0);
        sh(1, 1, 6, crate::calc::container::CODE_BASE, text_off, text.len() as u64, 0, 0);
        sh(7, 2, 0, 0, sym_off, sym.len() as u64, 3, 24);
        sh(15, 3, 0, 0, str_off, strtab.len() as u64, 0, 0);
        sh(23, 3, 0, 0, shstr_off, shstr.len() as u64, 0, 0);

        body[0..4].copy_from_slice(b"\x7fELF");
        body[4] = 2;
        body[5] = 1;
        body[6] = 1;
        body[16..18].copy_from_slice(&2u16.to_le_bytes());
        body[18..20].copy_from_slice(&243u16.to_le_bytes());
        body[20..24].copy_from_slice(&1u32.to_le_bytes());
        body[24..32].copy_from_slice(&0u64.to_le_bytes());
        body[32..40].copy_from_slice(&64u64.to_le_bytes());
        body[40..48].copy_from_slice(&shoff.to_le_bytes());
        body[52..54].copy_from_slice(&64u16.to_le_bytes());
        body[54..56].copy_from_slice(&56u16.to_le_bytes());
        body[56..58].copy_from_slice(&1u16.to_le_bytes());
        body[58..60].copy_from_slice(&64u16.to_le_bytes());
        body[60..62].copy_from_slice(&5u16.to_le_bytes());
        body[62..64].copy_from_slice(&4u16.to_le_bytes());
        body
    }

    fn user_code(sto: &crate::contract::ContractSto, name: &str) -> Vec<u8> {
        sto.userfuncs
            .iter()
            .find(|f| f.sign.0 == crate::rt::calc_func_sign(name))
            .unwrap_or_else(|| panic!("missing {name}"))
            .code_stuff
            .data
            .to_vec()
    }

    fn has_callselfpure(code: &[u8]) -> bool {
        let mut needle = vec![crate::rt::Bytecode::CALLSELFPURE as u8];
        needle.extend_from_slice(&crate::rt::calc_func_sign("echo8"));
        code.windows(needle.len()).any(|w| w == needle.as_slice())
    }

    fn roundtrip_lang(src: &str) {
        let ir = crate::lang::lang_to_irnode(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        let back = crate::lang::irnode_to_lang(ir).unwrap_or_else(|e| panic!("decompile {src}: {e}"));
        let ir2 = crate::lang::lang_to_irnode(&back).unwrap_or_else(|e| panic!("recompile {back}: {e}"));
        let back2 = crate::lang::irnode_to_lang(ir2).unwrap_or_else(|e| panic!("decompile again: {e}"));
        assert_eq!(back, back2, "src={src}");
        assert!(
            back.contains("self::echo8") || back.contains("call pure self.echo8") || back.contains("call pure self.0x"),
            "decompiled: {back}"
        );
    }

    #[test]
    fn pure_calc_calls_compile_to_callselfpure_and_roundtrip() {
        let blob = format!("0x{}", hex::encode(echo8_container_bytes()));
        let src = format!(
            r#"
pragma fitsh 1.0.0
contract Echo {{
    const IMG = {blob}
    function rv64im echo8(data: bytes) -> bytes = IMG
    function external direct(x: bytes) -> bytes {{ return self::echo8(x) }}
    function external joined(a: bytes, b: bytes) -> bytes {{ return self::echo8(a ++ b) }}
    function external generic(x: bytes) -> bytes {{ return call pure self.echo8(x) }}
}}
"#
        );
        let (contract, _, _, _) = crate::fitshc::compile(&src).expect("pure calc calls");
        let sto = contract.into_sto();
        assert_eq!(sto.calcfuncs.length(), 0);
        let echo = sto
            .userfuncs
            .iter()
            .find(|f| f.sign.0 == crate::rt::calc_func_sign("echo8"))
            .expect("echo8 userfunc");
        assert_eq!(echo.code_stuff.conf.uint(), crate::rt::CodeType::RV64IM as u8);
        let (c, _) = crate::calc::container::CalcContainer::decode(&echo.code_stuff.data)
            .expect("echo8 container");
        assert_eq!(c.profile, crate::calc::CALC_PROFILE_RV64IM);
        assert_eq!(c.layout().stack_bottom, c.guest_end());
        let direct = user_code(&sto, "direct");
        assert!(has_callselfpure(&direct), "missing CALLSELFPURE: {direct:?}");
        assert_eq!(direct, user_code(&sto, "generic"));
        let joined = user_code(&sto, "joined");
        assert!(has_callselfpure(&joined), "missing CALLSELFPURE: {joined:?}");
        assert!(joined.windows(1).any(|w| w == [crate::rt::Bytecode::CAT as u8]));
        roundtrip_lang("return self::echo8(0x01)");
        // User calls never auto-CAT: a `++` argument is one CAT value and must
        // stay explicit, otherwise recompiling `f(a, b)` would pack a tuple
        // instead of the concatenation the kernel expects. The print must keep
        // `++` and stay IR-idempotent.
        {
            let src = "return self::echo8(0x01 ++ 0x02)";
            let ir = crate::lang::lang_to_irnode(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            let back =
                crate::lang::irnode_to_lang(ir).unwrap_or_else(|e| panic!("decompile {src}: {e}"));
            assert!(
                back.contains("++"),
                "a user-call CAT argument must stay explicit: {back}"
            );
            roundtrip_lang(src);
        }
        // Packed argv is the other convention and still flattens: the same
        // tuple value is rebuilt from `f(a, b)` (both spellings compile to
        // one PACKTUPLE argument).
        roundtrip_lang("return self::echo8(tuple(0x01, 0x02))");
        roundtrip_lang("return call pure self.echo8(0x01)");

        let bare = format!(
            r#"
pragma fitsh 1.0.0
contract Echo {{
    function rv64im echo8(data: bytes) -> bytes = {blob}
    function external bare(x: bytes) -> bytes {{ return echo8(x) }}
}}
"#
        );
        let err = match crate::fitshc::compile(&bare) {
            Ok(_) => panic!("bare calc name must be rejected"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("unknown function 'echo8'"), "err={err}");
    }

    #[test]
    fn compile_with_rv64im_injects_container_and_elf() {
        let src = r#"
pragma fitsh 1.0.0
contract Ext {
    function rv64im echo8(data: bytes) -> bytes
    function rv64im haltkern()
    function external probe(x: bytes) -> bytes { return self::echo8(x) }
}
"#;
        let (contract, _, _, _) = crate::fitshc::compile_with_rv64im(
            src,
            &[
                ("echo8", echo8_container_bytes()),
                ("haltkern", synth_elf()),
            ],
        )
        .expect("external calcs");
        let sto = contract.into_sto();
        assert_eq!(sto.calcfuncs.length(), 0);
        let echo = sto
            .userfuncs
            .iter()
            .find(|f| f.sign.0 == crate::rt::calc_func_sign("echo8"))
            .expect("echo8");
        let halt = sto
            .userfuncs
            .iter()
            .find(|f| f.sign.0 == crate::rt::calc_func_sign("haltkern"))
            .expect("haltkern");
        assert_eq!(echo.code_stuff.conf.uint(), crate::rt::CodeType::RV64IM as u8);
        assert_eq!(halt.code_stuff.conf.uint(), crate::rt::CodeType::RV64IM as u8);
        let (c, used) = crate::calc::container::CalcContainer::decode(&halt.code_stuff.data)
            .expect("elf packed");
        assert_eq!(used, halt.code_stuff.data.len());
        assert_eq!(c.entry, crate::calc::CALC_CODE_BASE as u32);
        assert_eq!(c.first_rx_data(), halt_text());
        // The compiler packs the *minimum* memory that fits the image, the
        // guard, the 2 KiB stack and the declared IO, not the chain maximum.
        assert!(c.mem_kib >= crate::calc::container::CALC_MEM_KIB_MIN);
        assert!(c.mem_kib < crate::calc::container::CALC_MEM_KIB_MAX);
        assert!(c.layout().fits());
        assert_eq!((c.input_max, c.output_max), (1024, 1024));
    }

    #[test]
    fn function_rv64im_compiles_container() {
        let blob = format!("0x{}", hex::encode(echo8_container_bytes()));
        let item = format!("function rv64im echo8(data: bytes) -> bytes = {blob}");
        let src = format!("pragma fitsh 1.0.0\ncontract Echo {{\n    {item}\n}}\n");
        let (contract, _, _, _) =
            crate::fitshc::compile(&src).unwrap_or_else(|e| panic!("{item}: {e}"));
        let sto = contract.into_sto();
        let echo = sto
            .userfuncs
            .iter()
            .find(|f| f.sign.0 == crate::rt::calc_func_sign("echo8"))
            .expect("echo8");
        assert_eq!(echo.code_stuff.conf.uint(), crate::rt::CodeType::RV64IM as u8);
        assert_eq!(echo.code_stuff.conf.uint(), 2);
        assert_eq!(echo.code_stuff.data.to_vec(), echo8_container_bytes());
        for rejected in [
            format!("rv64im echo8(data: bytes) -> bytes = {blob}"),
            format!("function calc echo8(data: bytes) -> bytes = {blob}"),
            format!("calc echo8(data: bytes) -> bytes = {blob}"),
        ] {
            let src = format!("pragma fitsh 1.0.0\ncontract Echo {{\n    {rejected}\n}}\n");
            assert!(
                crate::fitshc::compile(&src).is_err(),
                "accepted shorthand: {rejected}"
            );
        }
    }

    #[test]
    fn reserved_fitsh_kernel_body_reports_not_supported() {
        let src = "pragma fitsh 1.0.0\ncontract D {\n    function rv64im kern(data: bytes) -> bytes { return kern(data) }\n}\n";
        let err = match crate::fitshc::compile(src) {
            Ok(_) => panic!("expected reserved-body error"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("not supported yet"), "{err}");
        assert!(err.contains("kern.rs"), "{err}");
    }

    #[test]
    fn calc_declaration_rejects_bad_signatures_and_payloads() {
        let blob = format!("0x{}", hex::encode(echo8_container_bytes()));
        let cases = [
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im echo8(data: bytes) -> bytes = {blob}\n    function rv64im echo8(data: bytes) -> bytes = {blob}\n}}\n"
                ),
                "duplicate rv64im function",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im echo8(data: bytes) -> bytes = {blob}\n    function echo8() {{}}\n}}\n"
                ),
                    "duplicate function",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function echo8() {{}}\n    function rv64im echo8(data: bytes) -> bytes = {blob}\n}}\n"
                ),
                "duplicate rv64im function",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im bad(n: map) -> bytes = {blob}\n}}\n"
                ),
                "parameter type must be bytes",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im bad(n: tuple) -> bytes = {blob}\n}}\n"
                ),
                "parameter type must be bytes",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im ok(a: bytes, b: bytes) -> bytes = {blob}\n}}\n"
                ),
                "params must be empty or one bytes",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im chunks(chunks: list) -> bytes = {blob}\n}}\n"
                ),
                "parameter type must be bytes",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im wide(n: u64, a: bytes) -> bytes = {blob}\n}}\n"
                ),
                "parameter type must be bytes",
            ),
            (
                format!(
                    "pragma fitsh 1.0.0\ncontract D {{\n    function rv64im bad(n: bytes) -> u64 = {blob}\n}}\n"
                ),
                "return type must be bytes",
            ),
            (
                "pragma fitsh 1.0.0\ncontract D {\n    function rv64im bad(n: bytes) -> bytes = 0xff\n}\n"
                    .to_string(),
                "calc container",
            ),
            (
                "pragma fitsh 1.0.0\ncontract D {\n    function rv64im Echo(n: bytes) -> bytes = 0xff\n}\n"
                    .to_string(),
                "cannot start with uppercase",
            ),
            (
                "pragma fitsh 1.0.0\ncontract D {\n    function rv64im missing(n: bytes) -> bytes\n}\n"
                    .to_string(),
                "external_rv64im",
            ),
        ];
        for (src, needle) in cases {
            let err = match crate::fitshc::compile(&src) {
                Ok(_) => panic!("expected error containing {needle}: {src}"),
                Err(err) => err.to_string(),
            };
            assert!(err.contains(needle), "err={err} needle={needle} src={src}");
        }
    }

}
