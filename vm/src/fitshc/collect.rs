//! Pure pre-scan of `function rv64im` declarations for build tooling.
//!
//! [`collect_calc_requests`] walks the token stream of a `.fitsh` source and
//! reports every rv64im calc declaration that does not carry an inline
//! `= 0x...` payload, so a build tool (the `fitshc` CLI) can resolve each
//! kernel to container bytes (compile a `<name>.rs`, load a `.hcf1`, pack an
//! `.elf`) before calling [`crate::fitshc::compile_with_rv64im`]. The scan is
//! intentionally cheap and total on well-formed declarations; the full parse
//! in [`parse_rv64im_function`] stays the authority for signatures and
//! duplicates. Like the rest of `fitshc`, this module touches no file system
//! and spawns no process, so the library stays usable from wasm/SDK builds.

use crate::Token::*;
use crate::lang::Tokenizer;
use crate::rt::{KwTy, TokenSpan};
use sys::{Ret, Rerr, errf};

/// `{line}:{col}: ` prefix from the span sidecar (empty when unknown).
fn span_pre(spans: &[TokenSpan], idx: usize) -> String {
    if idx < spans.len() && spans[idx].known() {
        format!("{}: ", spans[idx])
    } else {
        String::new()
    }
}

/// One `function rv64im <name>` declaration without an inline payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcRequest {
    pub name: String,
    /// `kernel(data: bytes)` (caller supplies call data) vs `kernel()`
    /// (input is empty). The wrapper generator uses this to pick the Rust
    /// signature users must write: `(input: &[u8], out: &mut [u8]) -> usize`
    /// vs `(out: &mut [u8]) -> usize`.
    pub has_input: bool,
    /// The declared parameter name (`Some("data")` for the call-data form).
    pub arg_name: Option<String>,
    /// Tokens inside the braces of a native fitsh kernel body
    /// (`function rv64im kern(data: bytes) -> bytes { ... }`); `None` for
    /// the rust-kernel / container-file resolution form.
    pub body: Option<Vec<crate::rt::Token>>,
    /// P2-D03 T1: parallel span sidecar for `body` (same length); empty when
    /// the body is `None`.
    pub body_spans: Vec<crate::rt::TokenSpan>,
}

/// P2-D03 T5 pre-scan check: every `name: type` pair inside an rv64im
/// parameter list must declare `bytes`. Diagnoses the declared type at the
/// source level instead of letting the backend report the type confusion.
fn validate_rv64im_param_types(
    name: &str,
    params: &[crate::rt::Token],
    spans: &[TokenSpan],
) -> Rerr {
    let mut i = 0usize;
    while i < params.len() {
        let at = span_pre(spans, i);
        let Some(Identifier(pname)) = params.get(i) else {
            return errf!(
                "{}rv64im function '{}': expected parameter name, got {}",
                at,
                name,
                crate::lang::token_desc(params.get(i).unwrap_or(&Identifier(String::new())))
            );
        };
        i += 1;
        match params.get(i) {
            Some(Keyword(KwTy::Colon)) | Some(Partition(':')) => i += 1,
            other => {
                return errf!(
                    "{}rv64im function '{}': expected ':' after parameter '{}', got {}",
                    span_pre(spans, i),
                    name,
                    pname,
                    other.map(crate::lang::token_desc)
                        .unwrap_or_else(|| "end of input".to_string())
                );
            }
        }
        match params.get(i) {
            Some(Identifier(t)) if t == "bytes" => {}
            Some(Keyword(KwTy::Bytes)) => {}
            other => {
                return errf!(
                    "{}rv64im function '{}': parameter '{}' type must be bytes, got {}; \
                     rv64im kernels take at most one bytes parameter — the call data",
                    span_pre(spans, i),
                    name,
                    pname,
                    other.map(crate::lang::token_desc)
                        .unwrap_or_else(|| "end of input".to_string())
                );
            }
        }
        i += 1;
        if matches!(params.get(i), Some(Partition(','))) {
            i += 1;
        }
    }
    Ok(())
}

/// Collect the rv64im calc requests of a contract source. Declarations with
/// an inline `= 0x...` / `= CONST` payload are already self-contained and are
/// skipped; a fitsh kernel body `{ ... }` is captured (balanced-brace) as a
/// native-body request the build tool lowers with the fitsh→RV64IM backend.
pub fn collect_calc_requests(code: &str) -> Ret<Vec<CalcRequest>> {
    let (tokens, spans) = Tokenizer::new(code.as_bytes())
        .parse_with_spans()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < tokens.len() {
        if !matches!(&tokens[i], Keyword(KwTy::Function)) {
            i += 1;
            continue;
        }
        // `parse_top_level` dispatches on exactly this pair; anything else is
        // an ordinary function and left to the full parser.
        let is_rv64im = matches!(&tokens[i + 1], Identifier(word) if word == "rv64im");
        if !is_rv64im {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        let Some(Identifier(name)) = tokens.get(j).cloned() else {
            i += 1;
            continue;
        };
        j += 1;
        // Optional parameter list: `()` or `(data: bytes[, ...])`. Skip to the
        // matching `)` (depth-aware even though the grammar has no nesting).
        let mut has_input = false;
        let mut arg_name: Option<String> = None;
        if matches!(tokens.get(j), Some(Partition('('))) {
            let mut depth = 1usize;
            let start = j + 1;
            j += 1;
            while j < tokens.len() && depth > 0 {
                if let Partition('(') = &tokens[j] {
                    depth += 1;
                } else if let Partition(')') = &tokens[j] {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if j >= tokens.len() {
                return errf!(
                    "{}rv64im function '{}' args list not closed",
                    span_pre(&spans, tokens.len().saturating_sub(1)),
                    name
                );
            }
            has_input = j > start;
            if has_input {
                if let Some(Identifier(arg)) = tokens.get(start) {
                    arg_name = Some(arg.clone());
                }
                // P2-D03 T5: reject non-bytes parameter types here, before the
                // build tool pre-lowers the body. Without this the CLI path
                // bound the parameter as bytes regardless of the declared type
                // and the lowering rejected it with the misleading
                // `expected an int value, got bytes` (doc 66 §5 E05). The full
                // parser (parse_rv64im_param_type) remains the authority and
                // rejects the same declarations.
                validate_rv64im_param_types(&name, &tokens[start..j], &spans[start..j])?;
            }
            j += 1; // consume ')'
        }
        // Optional `-> bytes` (parenthesized form allowed, as in the parser).
        if matches!(tokens.get(j), Some(Keyword(KwTy::Arrow))) {
            j += 1;
            let wrapped = matches!(tokens.get(j), Some(Partition('(')));
            if wrapped {
                j += 1;
            }
            if j < tokens.len() {
                j += 1; // the type token itself
            }
            if wrapped && matches!(tokens.get(j), Some(Partition(')'))) {
                j += 1;
            }
        }
        match tokens.get(j) {
            Some(Keyword(KwTy::Assign)) => {
                // Inline payload: exactly one token (`0x...` bytes literal or a
                // const name). Not a request; keep scanning after it.
                j += 2;
            }
            Some(Partition('{')) => {
                // Native fitsh kernel body: capture the balanced-brace token
                // range for the fitsh→RV64IM lowering.
                let mut depth = 1usize;
                let start = j + 1;
                j += 1;
                while j < tokens.len() && depth > 0 {
                    match &tokens[j] {
                        Partition('{') => depth += 1,
                        Partition('}') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth != 0 {
                    return errf!(
                        "{}rv64im function '{}' body braces not closed",
                        span_pre(&spans, tokens.len().saturating_sub(1)),
                        name
                    );
                }
                let body = tokens[start..j].to_vec();
                j += 1; // consume '}'
                out.push(CalcRequest { name, has_input, arg_name, body: Some(body), body_spans: spans[start..j].to_vec() });
            }
            _ => {
                out.push(CalcRequest { name, has_input, arg_name, body: None, body_spans: Vec::new() });
                // Nothing else belongs to the declaration; continue from j.
            }
        }
        i = j;
    }
    Ok(out)
}

#[cfg(test)]
mod collect_tests {
    use super::*;

    fn names(src: &str) -> Vec<(String, bool)> {
        collect_calc_requests(src)
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.has_input))
            .collect()
    }

    fn bodies(src: &str) -> Vec<(String, bool, bool)> {
        collect_calc_requests(src)
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.body.is_some(), r.arg_name.is_some()))
            .collect()
    }

    #[test]
    fn collects_plain_and_zero_arg_requests() {
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    function rv64im hash(data: bytes) -> bytes
    function rv64im tick()
    function external run(x: bytes) -> bytes { return self::hash(x) }
}
"#;
        assert_eq!(
            names(src),
            vec![("hash".to_string(), true), ("tick".to_string(), false)]
        );
    }

    #[test]
    fn inline_payload_and_const_are_not_requests() {
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    const IMG = 0x484346310100000000000004000010000000000100000000
    function rv64im small(data: bytes) -> bytes = 0x4843463101000000000000040000100000000001000000
    function rv64im viaconst() = IMG
    function rv64im big(data: bytes) -> bytes
}
"#;
        assert_eq!(names(src), vec![("big".to_string(), true)]);
    }

    #[test]
    fn ordinary_functions_are_ignored() {
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    function rv64im_not(data: bytes) -> bytes { return data }
    function rv64 real(data: bytes) -> bytes { return real(data) }
}
"#;
        assert_eq!(names(src), Vec::<(String, bool)>::new());
    }

    #[test]
    fn body_form_is_collected_as_a_native_request() {
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    function rv64im kern(data: bytes) -> bytes {
        var n = size(data)
        return n as bytes
    }
    function external run(x: bytes) -> bytes { return self::kern(x) }
}
"#;
        let reqs = collect_calc_requests(src).unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].name, "kern");
        assert!(reqs[0].has_input);
        assert_eq!(reqs[0].arg_name.as_deref(), Some("data"));
        let body = reqs[0].body.as_ref().unwrap();
        // balanced-brace capture: the body's own braces stay inside
        assert!(
            !body.iter().any(|t| matches!(t, crate::rt::Token::Partition('}'))),
            "closing contract brace leaked into the body"
        );
    }

    #[test]
    fn body_brace_mismatch_is_an_error() {
        let src = "function rv64im kern() -> bytes { var x = 1";
        let err = collect_calc_requests(src).unwrap_err().to_string();
        assert!(err.contains("braces not closed"), "{err}");
    }

    #[test]
    fn parenthesized_return_type_is_skipped() {
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    function rv64im hash(data: bytes) -> (bytes)
}
"#;
        assert_eq!(names(src), vec![("hash".to_string(), true)]);
    }

    #[test]
    fn matches_the_full_parser_on_declared_and_missing_payloads() {
        // The scanner and `compile_with_rv64im` must agree on which
        // declarations need external bytes.
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    function rv64im a(data: bytes) -> bytes
    function rv64im b()
}
"#;
        let reqs = collect_calc_requests(src).unwrap();
        let missing: Vec<String> = crate::fitshc::compile_with_rv64im(src, &[])
            .err()
            .map(|e| e.to_string())
            .into_iter()
            .collect();
        assert_eq!(reqs.len(), 2);
        assert!(!missing.is_empty(), "full parser must reject missing bytes");
    }
}
