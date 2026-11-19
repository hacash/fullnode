//! P2-D03 (T8 / T5 / T6 / T9 / T2) error-message regression tests at the
//! library level. Each test pins the post-change text and asserts the absence
//! of the pre-change Debug leak (`Some(..)`, `Identifier(..)`, `Bytes(..)`,
//! enum-variant names).
//!
//! Pre-change behavior is documented per test (doc 66 §5 E19 etc.).

use vm::fitshc::compile_with_rv64im;

fn compile_err(src: &str) -> String {
    match compile_with_rv64im(src, &[]) {
        Ok(_) => panic!("expected compile error for:\n{src}"),
        Err(e) => e.to_string(),
    }
}

const CONTRACT_OK: &str = "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n}\n";

/// T8 / E19: trailing garbage after the contract end used to print
/// `unexpected token after contract end: Some(Identifier("garbage"))`.
#[test]
fn token_after_contract_end_names_the_token() {
    let err = compile_err(&format!("{CONTRACT_OK}garbage\n"));
    assert!(
        err.contains("unexpected token 'garbage' after contract end"),
        "{err}"
    );
    assert!(!err.contains("Some("), "Debug leak: {err}");
    assert!(!err.contains("Identifier("), "Debug leak: {err}");
}

/// T8: an unexpected token inside the contract body used to print
/// `unexpected token in contract body: Bytes([...])`.
#[test]
fn contract_body_token_is_human_readable() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    0x1234\n}\n",
    );
    assert!(
        err.contains("unexpected token '0x1234' in contract body"),
        "{err}"
    );
    assert!(!err.contains("Bytes("), "Debug leak: {err}");
}

/// T8: a missing function-body brace used to print
/// `expected '{' but got Keyword(Return)`.
#[test]
fn missing_brace_names_the_token() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function f() return 1 }\n}\n",
    );
    assert!(err.contains("expected '{' but got 'return'"), "{err}");
    assert!(!err.contains("Keyword("), "Debug leak: {err}");
}

/// T8: an expression starting with an operator used to print
/// `operator MUL cannot start expression` (Debug enum leak).
#[test]
fn operator_start_expression_uses_symbol() {
    let err = match vm::lang::lang_to_irnode("var a = * 5") {
        Ok(_) => panic!("expected operator-start error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("operator '*' cannot start expression"), "{err}");
    assert!(!err.contains("MUL"), "Debug leak: {err}");
}

/// T8: a raw bytecode body containing a character literal used to print
/// `invalid token in raw bytecode function: Character(97)`.
#[test]
fn raw_bytecode_invalid_token_is_human_readable() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function bytecode f() { 'a' }\n}\n",
    );
    assert!(
        err.contains("invalid token 'a' in raw bytecode function"),
        "{err}"
    );
    assert!(!err.contains("Character("), "Debug leak: {err}");
}

/// T8 / E05 companion: a bytes literal where a const name is expected in the
/// `= CONST` position names the token (was `got Some(Bytes(..))`).
#[test]
fn rv64im_payload_position_names_the_token() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im f() -> bytes = 5\n}\n",
    );
    assert!(
        err.contains("requires bytes literal or const, got '5'"),
        "{err}"
    );
    assert!(!err.contains("Some("), "Debug leak: {err}");
    assert!(!err.contains("Integer("), "Debug leak: {err}");
}

/// T8: token_desc mirrors keyword source spellings (spot check via the
/// `unexpected token` channel with symbol-shaped keywords).
#[test]
fn keyword_desc_uses_source_spelling() {
    // `->` in a position where a function name is expected.
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function -> () { }\n}\n",
    );
    assert!(err.contains("expected function name but got '->'"), "{err}");
    assert!(!err.contains("Arrow"), "Debug leak: {err}");
}

/// T9 / E26: the deep-variable nesting reject carries the actionable split
/// hint (appended at the fitshc boundary; the core message, and with it the
/// externally asserted substring `nesting exceeds`, stays contiguous).
///
/// E10 (bank construction leaves the `d <= 6` gate) split this face by
/// lowering mode: with ImcRemap banking active, 9 hot int locals can sit in
/// bank homes while at most 8 stack slots are live, so the 9-variable
/// right-nested sum COMPILES in the production default mode (ImcRemap) and
/// executes correctly (probe: sum of nine `byte(data,i)` with input 0x01…
/// = 9, matching the interpreter bit-for-bit). The fixed 8-value pool still
/// binds Im (no bank there): that mode keeps rejecting with the full hint,
/// and that is what this test pins (through `lower_body_mode_arena` with an
/// explicit `Rv64Mode::Im` — the `compile_with_rv64im` entry always rides
/// the ImcRemap default).
#[test]
fn nesting_reject_carries_split_hint() {
    use vm::ir::rv64::{lower_body_mode_arena, Rv64Mode};

    let mut body = String::from("var v0 = (byte(data, 0) as u64)\n");
    for i in 1..9 {
        body.push_str(&format!(
            "var v{i} = (byte(data, {i}) as u64)\n"
        ));
    }
    body.push_str("var r = (v0 + (v1 + (v2 + (v3 + (v4 + (v5 + (v6 + (v7 + v8))))))))\n");
    body.push_str("return (r as u64) as bytes\n");
    let tokens = vm::lang::Tokenizer::new(body.as_bytes())
        .parse()
        .expect("tokenize");
    let (ir, _, _) = vm::fitshc::compile_body(
        tokens,
        vec![("data".to_string(), vm::ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .expect("front end accepts the body");
    let err = lower_body_mode_arena(&ir, true, Rv64Mode::Im, vm::ir::rv64::ARENA_SIZE_MAX)
        .expect_err("Im must reject the 9-deep nesting");
    let err = vm::fitshc::kernel::annotate_backend_reject(&err.to_string());
    assert!(err.contains("nesting exceeds"), "{err}");
    assert!(err.contains("hint: split the expression"), "{err}");
}

/// T5 / E05: an rv64im kernel parameter declared `x: u32` used to sail
/// through the pre-scan, get bound as bytes by the pre-lower and fail in the
/// backend with the self-contradictory
/// `rv64 lower: expected an int value, got bytes`. Now both guard layers
/// reject at the source level and name the declared type: the collect
/// pre-scan (CLI pre-lower path) and the full parser.
#[test]
fn rv64im_kernel_bad_param_type_rejected_at_prescan() {
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(x: u32) -> bytes { return 0x00 }\n}\n";

    // CLI path: collect_calc_requests guards the pre-lower.
    let err = match vm::fitshc::collect_calc_requests(src) {
        Ok(_) => panic!("collect must reject non-bytes rv64im param type"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("parameter 'x' type must be bytes, got 'u32'"), "{err}");

    // Library path: the full parser rejects with the named type.
    let err = compile_err(src);
    assert!(err.contains("parameter type must be bytes (got 'u32')"), "{err}");
    assert!(
        !err.contains("expected an int value, got bytes"),
        "misleading backend message must be gone: {err}"
    );
    assert!(!err.contains("rv64 lower:"), "{err}");
}

/// T5: unknown parameter types list the supported set (substring
/// `unknown type` is asserted by the in-module parse_func tests).
#[test]
fn unknown_param_type_lists_supported_set() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function probe(a: u256) { return a }\n}\n",
    );
    assert!(err.contains("unknown type"), "{err}");
    assert!(err.contains("got 'u256'"), "{err}");
    assert!(err.contains("supported: u8, u16, u32, u64, u128"), "{err}");
}

/// T5: unknown return types list the supported set.
#[test]
fn unknown_return_type_lists_supported_set() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function probe() -> uint { return 1 }\n}\n",
    );
    assert!(err.contains("unknown return type"), "{err}");
    assert!(err.contains("got 'uint'"), "{err}");
    assert!(err.contains("supported: u8, u16, u32, u64, u128"), "{err}");
}

/// T6 / E13: `$maxn(...)` inside a kernel body used to report the bare
/// `unknown function '$maxn'`, indistinguishable from a typo'd function.
/// Now the `$` prefix is statically diagnosed as a macro misuse.
#[test]
fn dollar_call_is_diagnosed_as_macro_misuse() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        var n = $maxn(data, data)\n        return n as bytes\n    }\n}\n",
    );
    assert!(err.contains("looks like a macro call"), "{err}");
    assert!(err.contains("'$maxn'"), "{err}");
    assert!(err.contains("hint:"), "{err}");
    assert!(
        err.contains("define $name"),
        "message must show the macro definition syntax: {err}"
    );
}

/// T7 / E02: a missing contract-closing brace used to report the bare,
/// misleading `unexpected EOF while parsing contract body`. Now the brace
/// imbalance is diagnosed from the token stream.
#[test]
fn missing_contract_brace_is_diagnosed() {
    // One function-body '}' present, contract '}' missing → 2 '{' vs 1 '}'.
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n",
    );
    assert!(
        err.contains("unexpected EOF while parsing contract body"),
        "{err}"
    );
    assert!(err.contains("never closed"), "{err}");
    assert!(err.contains("missing its closing '}'"), "{err}");
}

/// T7: a missing function-body brace names the function-body cause
/// (parse_func_body_tokens hits EOF with depth > 0).
#[test]
fn missing_function_body_brace_is_diagnosed() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x\n",
    );
    assert!(
        err.contains("bracket mismatch: function body not closed"),
        "{err}"
    );
}

/// T2 / E06/E11: `data[0]` used to collapse to the inactionable
/// `opcode ITEMGET is outside the translatable kernel subset`. The boundary
/// names the source construct and the fix. P3-V2IMPL (A-25 M1): the desc
/// was corrected per doc 73 E6 — `data[i]` on a bytes receiver is a RUNTIME
/// CompoOpNotMatch(83) on the interpreter, so the message now says the
/// indexed value is not a list variable (list variables are the only
/// admissible ITEMGET receivers).
#[test]
fn byte_indexing_reject_names_the_construct() {
    let err = compile_err(
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        var hv = data[0] as u64\n        return hv as bytes\n    }\n}\n",
    );
    assert!(
        err.contains("not a list variable"),
        "{err}"
    );
    assert!(err.contains("hint: read fixed byte positions"), "{err}");
    assert!(err.contains("rv64im kernel 'k'"), "{err}");
    assert!(
        !err.contains("opcode ITEMGET"),
        "IR opcode must not leak: {err}"
    );
}

// ---------------- T1: token position sidecar ----------------

/// T1: tokenizer errors carry `{line}:{col}: ` positions.
#[test]
fn tokenizer_errors_carry_positions() {
    let err = match vm::lang::Tokenizer::new(b"var a = 0xzz").parse() {
        Ok(_) => panic!("expected hex error"),
        Err(e) => e.to_string(),
    };
    assert!(err.starts_with("1:9: hex data format invalid '0xzz'"), "{err}");

    let err = match vm::lang::Tokenizer::new(b"return @").parse() {
        Ok(_) => panic!("expected char error"),
        Err(e) => e.to_string(),
    };
    assert!(err.starts_with("1:8: unsupported char [@]"), "{err}");
}

/// T1: span table is in lockstep with the token stream (same length).
#[test]
fn span_table_matches_token_stream() {
    let src = b"var a = 1u64\nreturn a as bytes // trailing comment\n";
    let (tokens, spans) = vm::lang::Tokenizer::new(src)
        .parse_with_spans()
        .unwrap();
    assert_eq!(tokens.len(), spans.len(), "lockstep violated");
    // First token `var` at 1:1.
    assert_eq!(spans[0], vm::lang::TokenSpan { line: 1, col: 1 });
}

/// T1: contract-level errors carry `{file}:{line}:{col}: ` when compiled
/// through the file-aware entry.
#[test]
fn contract_errors_carry_file_positions() {
    let err = match vm::fitshc::compiler::compile_with_rv64im_at(
        "t.fitsh",
        &format!("{CONTRACT_OK}garbage\n"),
        &[],
    ) {
        Ok(_) => panic!("expected error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("t.fitsh:"), "{err}");
    assert!(err.contains(": unexpected token 'garbage' after contract end"), "{err}");
}

/// T1 / E08: kernel-body `cannot find symbol` carries the position (was
/// bare `cannot find symbol 'dat'` with no location anywhere).
#[test]
fn kernel_body_symbol_error_carries_position() {
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        return dat\n    }\n}\n";
    let err = compile_err(src);
    // `dat` sits on source line 4, col 16 (8 spaces + `return `).
    assert!(err.contains("4:16: cannot find symbol 'dat'"), "{err}");

    // File-aware entry prefixes the file too.
    let err = match vm::fitshc::compiler::compile_with_rv64im_at("t.fitsh", src, &[]) {
        Ok(_) => panic!("expected error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("t.fitsh:4:16: cannot find symbol 'dat'"), "{err}");
}

/// T1: the spanless paths (`parse()`, `compile_body`, `Syntax::new`) still
/// produce byte-identical token streams and messages without positions —
/// zero-behavior proof for existing callers.
#[test]
fn spanless_paths_unchanged() {
    let src = b"var a = 1u64\nreturn a";
    let plain = vm::lang::Tokenizer::new(src).parse().unwrap();
    let (with, _) = vm::lang::Tokenizer::new(src).parse_with_spans().unwrap();
    assert_eq!(plain, with);

    // compile_body (public, spanless) reports the message without position.
    let err = match vm::fitshc::compile_body(
        vm::lang::Tokenizer::new(b"return zz").parse().unwrap(),
        vec![],
        &[],
        &[],
        false,
    ) {
        Ok(_) => panic!("expected error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("cannot find symbol 'zz'"), "{err}");
    assert!(!err.contains("1:"), "spanless path must not invent positions: {err}");
}

/* ================================================================== */
/* P3-GUARDTAIL GT-c: rv64.rs error-site batch pins (doc 66 §1.4       */
/* leftover — the D-03 milestone could not touch rv64.rs).             */
/* ================================================================== */

/// GT-c: the input-parameter reject now carries an actionable hint; the
/// asserted core substring "input parameter cannot be assigned"
/// (rv64_gen_corpus.rs c2_probe_engine_edges P2) stays contiguous. T15
/// flipped the bytes rebind (`data = <bytes>`) to a supported form, so the
/// pinned reject now rides the still-rejected compound (`data += v`).
#[test]
fn gt_c_input_param_reject_carries_hint() {
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        data += 1u64\n        return 0u64 as bytes\n    }\n}\n";
    let err = compile_err(src);
    assert!(err.contains("input parameter cannot be assigned"), "{err}");
    assert!(err.contains("copy it into a local first"), "{err}");
}


/// GT-c: slot-type conflict messages name the slot type in words (no Debug
/// variant leakage) — the plan-side XOP walk rejects a bytes slot consumed
/// as int before the lowering runs.
#[test]
fn gt_c_slot_mix_names_the_type() {
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        var b = data\n        b = b + 1u64\n        return b\n    }\n}\n";
    let err = compile_err(src);
    println!("mixes probe: {err}");
    assert!(
        err.contains("used with conflicting types") || err.contains("mixes bytes with int"),
        "{err}"
    );
    assert!(!err.contains("Bytes("), "Debug leak: {err}");
    assert!(!err.contains("Int("), "Debug leak: {err}");
}
