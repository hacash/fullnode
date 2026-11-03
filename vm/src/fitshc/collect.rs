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
use crate::rt::KwTy;
use sys::{Ret, errf};

/// One `function rv64im <name>` declaration without an inline payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcRequest {
    pub name: String,
    /// `kernel(data: bytes)` (caller supplies call data) vs `kernel()`
    /// (input is empty). The wrapper generator uses this to pick the Rust
    /// signature users must write: `(input: &[u8], out: &mut [u8]) -> usize`
    /// vs `(out: &mut [u8]) -> usize`.
    pub has_input: bool,
}

/// Collect the rv64im calc requests of a contract source. Declarations with
/// an inline `= 0x...` / `= CONST` payload are already self-contained and are
/// skipped; the reserved fitsh kernel body form `{ ... }` is rejected here
/// with a "not supported yet" error (same wording as the full parser).
pub fn collect_calc_requests(code: &str) -> Ret<Vec<CalcRequest>> {
    let tokens = Tokenizer::new(code.as_bytes())
        .parse()
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
                return errf!("rv64im function '{}' args list not closed", name);
            }
            has_input = j > start;
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
                return errf!(
                    "rv64im function '{}' fitsh kernel body is reserved and not supported yet; \
                     assign container bytes ('= 0x..'), supply a '{name}.rs' rust kernel, \
                     or pass '--calc {name}=<kernel.hcf1|kernel.elf>'",
                    name
                );
            }
            _ => {
                out.push(CalcRequest { name, has_input });
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
    fn reserved_body_form_is_rejected() {
        let src = r#"
pragma fitsh 1.0.0
contract Demo {
    function rv64im kern(data: bytes) -> bytes { return kern(data) }
}
"#;
        let err = collect_calc_requests(src).unwrap_err().to_string();
        assert!(err.contains("not supported yet"), "{err}");
        assert!(err.contains("kern.rs"), "{err}");
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
