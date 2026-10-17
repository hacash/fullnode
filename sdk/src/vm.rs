//! `vm.decode_call`: structured view of a `contract_main_call` action
//! (Hacash SDK, doc 14 §6.3). The wire carries `marks` (reserved, must be
//! zero), `codeconf` (2-bit code type + 6 reserved bits) and the bytecode
//! (`codes`). Code-type masks are `vm::action::CODECONF_*` (authority:
//! `CodeType::TYPE_MASK` / `CodeConf::RESERVED_MASK`).

use base::BinaryCodecs;
use field::{BytesW2, Decode, Fixed3, Uint1, Uint2};

use crate::error::{SdkError, SdkErrorCode};
use crate::json::SdkJsonTo;
use crate::schema::SCHEMA_VM_CALL;

pub use vm::action::{CODECONF_RESERVED_MASK, CODECONF_TYPE_MASK};

fn code_type_name(raw: u8) -> Result<(&'static str, u8), SdkError> {
    crate::audit::code_type_name(raw)
}

/// `vm.decode_call` response (`hacash.sdk/vm-call@1`).
#[derive(Debug, Clone, PartialEq)]
pub struct VmCall {
    pub schema: String,
    pub kind: u16,
    pub name: String,
    pub scope: String,
    /// `marks` wire hex (3 bytes); execution requires it to be zero.
    pub marks: String,
    pub marks_valid: bool,
    pub codeconf: u8,
    pub code_type: u8,
    pub code_type_name: String,
    pub codes_len: usize,
    /// sha3-256 of the bytecode (identity without echoing a large payload).
    pub codes_hash: String,
    /// First up-to-64 bytes of the bytecode, hex (display preview).
    pub codes_preview: String,
}

/// `vm.decode_call`: decode a raw `contract_main_call` action wire (the
/// `actions[].raw` of a `tx.decode` output) into its structured fields.
pub fn decode_call(action_hex: &str) -> Result<VmCall, SdkError> {
    let wire = hex::decode(action_hex.trim_start_matches("0x").trim_start_matches("0X"))
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "action hex invalid"))?;
    let codecs = crate::codec::standard_codecs().map_err(SdkError::from)?;
    let action = codecs.decode_action_exact(&wire).map_err(|error| {
        SdkError::with_detail(
            SdkErrorCode::ParseFailed,
            "action wire does not decode",
            format!("{{\"error\":{}}}", crate::json::q(&error.to_string())),
        )
    })?;
    if action.kind() != vm::action::ContractMainCall::KIND {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!(
                "action kind {} is not contract_main_call ({})",
                action.kind(),
                vm::action::ContractMainCall::KIND
            ),
        ));
    }

    // The wire layout is `kind: Uint2, marks: Fixed3, codeconf: Uint1, codes: BytesW2`.
    // Decode returns offsets relative to each input slice, so accumulate.
    let mut offset = 0usize;
    let (kind, n) = Uint2::decode(&wire).map_err(|e| parse_wire(e, "kind"))?;
    offset += n;
    let (marks, n) = Fixed3::decode(&wire[offset..]).map_err(|e| parse_wire(e, "marks"))?;
    offset += n;
    let (codeconf, n) = Uint1::decode(&wire[offset..]).map_err(|e| parse_wire(e, "codeconf"))?;
    offset += n;
    let (codes, n) = BytesW2::decode(&wire[offset..]).map_err(|e| parse_wire(e, "codes"))?;
    offset += n;
    if offset != wire.len() {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "contract_main_call wire has trailing bytes",
        ));
    }

    let codeconf_raw = codeconf.uint();
    let (type_name, type_id) = code_type_name(codeconf_raw)?;
    let codes_bytes = codes.as_vec();
    let codes_hash = hex::encode(sys::calculate_hash(codes_bytes));
    let preview_len = codes_bytes.len().min(64);
    let schema = crate::selection::action_schema(kind.uint()).map(|s| s.name);
    let scope = crate::audit::scope_name(action.scope());
    Ok(VmCall {
        schema: SCHEMA_VM_CALL.to_owned(),
        kind: kind.uint(),
        name: schema.unwrap_or("contract_main_call").to_owned(),
        scope: scope.to_owned(),
        marks: hex::encode(marks.as_bytes()),
        marks_valid: marks.is_zero(),
        codeconf: codeconf_raw,
        code_type: type_id,
        code_type_name: type_name.to_owned(),
        codes_len: codes_bytes.len(),
        codes_hash,
        codes_preview: hex::encode(&codes_bytes[..preview_len]),
    })
}

fn parse_wire(error: sys::Error, field: &str) -> SdkError {
    SdkError::new(
        SdkErrorCode::ParseFailed,
        format!("contract_main_call {field} wire decode failed: {error}"),
    )
}

// ================================ vm.code ================================

/// `vm.code`: decompile a code body into human-readable text. Two code types
/// (bytecode → annotated assembly; irnode → fitsh source or structural tree)
/// and independent output controls (`format`, `limit`/`offset` paging,
/// optional external `sourcemap` for maximum readability). Offline and
/// codec-only — no VM execution, no node.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeOutput {
    pub schema: String,
    pub code_type: u8,
    pub code_type_name: String,
    pub codes_len: usize,
    pub codes_hash: String,
    pub format: String,
    /// Line count of the returned text slice (the UI can use it to decide
    /// inline vs. new-page rendering).
    pub lines: usize,
    pub text: String,
    pub truncated: bool,
    pub limit: u64,
    pub offset: u64,
    pub total_bytes: u64,
    pub next_offset: Option<u64>,
    pub sourcemap_status: String,
    pub warnings: Vec<String>,
    pub sourcemap_labels: Vec<String>,
}

impl CodeOutput {
    pub(crate) fn to_json_string(&self) -> String {
        use crate::json::{kv, obj, q, qnum};
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("code_type", qnum(self.code_type)),
            kv("code_type_name", q(&self.code_type_name)),
            kv("codes_len", qnum(self.codes_len as u64)),
            kv("codes_hash", q(&self.codes_hash)),
            kv("format", q(&self.format)),
            kv("lines", qnum(self.lines as u64)),
            kv("text", q(&self.text)),
            kv(
                "truncated",
                if self.truncated {
                    "true".to_owned()
                } else {
                    "false".to_owned()
                },
            ),
            kv("limit", qnum(self.limit)),
            kv("offset", qnum(self.offset)),
            kv("total_bytes", qnum(self.total_bytes)),
            kv("next_offset", self.next_offset.map(qnum).unwrap_or_else(|| "null".to_owned())),
            kv("sourcemap_status", q(&self.sourcemap_status)),
            kv("warnings", crate::json::arr(self.warnings.iter().map(|s| crate::json::q(s)).collect())),
            kv("sourcemap_labels", crate::json::arr(self.sourcemap_labels.iter().map(|s| crate::json::q(s)).collect())),
        ])
    }
}

const VM_CODE_DEFAULT_LIMIT: u64 = 8000;

fn hex_codes(raw: &str) -> Result<Vec<u8>, SdkError> {
    hex::decode(raw.trim_start_matches("0x").trim_start_matches("0X"))
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "codes hex invalid"))
}

/// `vm.code` entry (offline decompilation; `format` and paging controls are
/// independent of the code type defaults).
pub fn code(
    codes_hex: &str,
    code_type_str: &str,
    format: Option<&str>,
    sourcemap_json: Option<&str>,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<CodeOutput, SdkError> {
    let codes = hex_codes(codes_hex)?;
    let code_type: u8 = code_type_str
        .trim()
        .parse()
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "code_type must be 0 or 1"))?;
    let (type_name, type_id) = crate::audit::code_type_name(code_type)?;
    let limit = limit.unwrap_or(VM_CODE_DEFAULT_LIMIT);
    let offset = offset.unwrap_or(0);

    // External source map: parse at the boundary so the caller's JSON object
    // rides the wire unchanged (the decompiler consumes `vm::lang::SourceMap`).
    let review_mode = format == Some("fitsh_review");
    let mut warnings = Vec::new();
    let mut sourcemap_labels = Vec::new();
    let mut sourcemap_status = "absent".to_owned();
    let smap = match sourcemap_json {
        Some(raw) if review_mode => match vm::lang::SourceMap::from_json(raw) {
            Ok(map) => {
                let (_, mut labels, mut notes) = map.review_labels();
                sourcemap_labels.append(&mut labels);
                sourcemap_status = if notes.is_empty() { "accepted" } else { "partial" }.to_owned();
                warnings.append(&mut notes);
                warnings.push("dApp labels are annotations; numeric slots and selectors remain authoritative".to_owned());
                None
            }
            Err(error) => {
                sourcemap_status = "ignored".to_owned();
                warnings.push(format!("sourcemap ignored: {error}"));
                None
            }
        },
        Some(raw) => {
            // Named (non-review) channel with an external map: the map is
            // UNTRUSTED dapp input, so decompile from the display-sanitized
            // view (validated function/slot labels, library alias names with
            // addresses dropped) and surface the accepted labels the same way
            // review mode does.
            let map = vm::lang::SourceMap::from_json(raw).map_err(|e| {
                SdkError::new(SdkErrorCode::ParseFailed, format!("sourcemap invalid: {e}"))
            })?;
            let (display, mut labels, mut notes) = map.display_labels();
            sourcemap_labels.append(&mut labels);
            sourcemap_status = if notes.is_empty() { "accepted" } else { "partial" }.to_owned();
            warnings.append(&mut notes);
            Some(display)
        }
        None => None,
    };

    let (format_name, full_text) = match (type_id, format) {
        (0, None | Some("assembly")) => (
            "assembly".to_owned(),
            vm::lang::asm::disassemble_bytecode_asm(&codes)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (0, Some("raw")) => (
            "raw".to_owned(),
            vm::lang::asm::disassemble_bytecode_raw(&codes)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (0, Some(other)) => {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("format {other} is not valid for bytecode; expected assembly or raw"),
            ));
        }
        (1, Some("fitsh_review")) => (
            "fitsh_review".to_owned(),
            vm::lang::format_ircode_to_review(&codes)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (1, None | Some("fitsh")) => (
            "fitsh".to_owned(),
            vm::lang::format_ircode_to_lang(&codes, smap.as_ref())
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (1, Some("tree")) => (
            "tree".to_owned(),
            vm::lang::ir_tree_text(&codes)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (1, Some("raw")) => (
            "raw".to_owned(),
            vm::lang::asm::disassemble_ircode_raw(&codes)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (1, Some("assembly")) => (
            "assembly".to_owned(),
            vm::lang::asm::disassemble_ircode_asm(&codes)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (1, Some(other)) => {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                format!(
                    "format {other} is not valid for irnode; expected fitsh, fitsh_review, tree, raw, or assembly"
                ),
            ));
        }
        _ => unreachable!("code type validated above"),
    };

    let total = full_text.len() as u64;
    // UTF-8-safe paging: snap the window to char boundaries and never return
    // a partial character. A window that shrank to nothing while characters
    // remain still yields one whole character — except an explicit `limit: 0`,
    // which stays an empty page. `start` may advance past the requested end
    // (mid-char offset), so clamp before slicing.
    let requested_start = offset.min(total) as usize;
    let mut start = requested_start;
    while start < full_text.len() && !full_text.is_char_boundary(start) {
        start += 1;
    }
    let requested_end = offset.saturating_add(limit).min(total) as usize;
    let mut end = requested_end.max(start);
    while end > start && !full_text.is_char_boundary(end) {
        end -= 1;
    }
    if end <= start && start < full_text.len() && limit > 0 {
        end = start + full_text[start..].chars().next().unwrap().len_utf8();
    }
    let text = &full_text[start..end];
    let lines = text.lines().count();
    Ok(CodeOutput {
        schema: crate::schema::SCHEMA_VM_CODE.to_owned(),
        code_type: type_id,
        code_type_name: type_name.to_owned(),
        codes_len: codes.len(),
        codes_hash: hex::encode(sys::calculate_hash(&codes)),
        format: format_name,
        lines,
        text: text.to_owned(),
        truncated: end < total as usize,
        limit,
        offset: start as u64,
        total_bytes: total,
        next_offset: (end < total as usize).then_some(end as u64),
        sourcemap_status,
        warnings,
        sourcemap_labels,
    })
}

impl SdkJsonTo for VmCall {
    fn to_json_string(&self) -> String {
        use crate::json::{kv, obj, q, qnum};
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("kind", qnum(self.kind)),
            kv("name", q(&self.name)),
            kv("scope", q(&self.scope)),
            kv("marks", q(&self.marks)),
            kv(
                "marks_valid",
                if self.marks_valid {
                    "true".to_owned()
                } else {
                    "false".to_owned()
                },
            ),
            kv("codeconf", qnum(self.codeconf)),
            kv("code_type", qnum(self.code_type)),
            kv("code_type_name", q(&self.code_type_name)),
            kv("codes_len", qnum(self.codes_len as u64)),
            kv("codes_hash", q(&self.codes_hash)),
            kv("codes_preview", q(&self.codes_preview)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{ActionSpec, TransactionSpec, build_transaction};
    use field::Encode;

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn bytes_json(bytes: &[u8]) -> String {
        format!("0x{}", hex::encode(bytes))
    }

    fn maincall_spec(codes: Vec<u8>) -> TransactionSpec {
        TransactionSpec {
            schema: None,
            tx_type: 3,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            addrlist: None,
            actions: vec![
                ActionSpec::new(format!(
                    r#"{{"kind":44,"marks":"0x000000","codeconf":0,"codes":"{}"}}"#,
                    bytes_json(&codes)
                ))
                .expect("test action spec"),
            ],
        }
    }

    #[test]
    fn decodes_a_built_maincall() {
        let codes = vec![0x01u8, 0x02, 0x03];
        let built = build_transaction(&maincall_spec(codes.clone())).unwrap();
        let decoded = crate::inspect::decode_transaction_json(
            &built.body,
            &crate::audit::DescribeOptions::default(),
        )
        .unwrap();
        let call = decode_call(&decoded.actions[0].raw).unwrap();
        assert_eq!(call.kind, vm::action::ContractMainCall::KIND);
        assert_eq!(call.name, "contract_main_call");
        assert!(call.marks_valid);
        assert_eq!(call.marks, "000000");
        assert_eq!(call.code_type_name, "bytecode");
        assert_eq!(call.code_type, 0);
        assert_eq!(call.codes_len, 3);
        assert_eq!(call.codes_preview, hex::encode(&codes));
        assert_eq!(call.codes_hash, hex::encode(sys::calculate_hash(&codes)));
    }

    #[test]
    fn flags_nonzero_marks_as_invalid() {
        let built =
            build_transaction(&maincall_spec_with_marks(vec![1, 2, 3], vec![0x01])).unwrap();
        let decoded = crate::inspect::decode_transaction_json(
            &built.body,
            &crate::audit::DescribeOptions::default(),
        )
        .unwrap();
        let call = decode_call(&decoded.actions[0].raw).unwrap();
        assert!(!call.marks_valid);
    }

    fn maincall_spec_with_marks(marks: Vec<u8>, codes: Vec<u8>) -> TransactionSpec {
        TransactionSpec {
            schema: None,
            tx_type: 3,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            addrlist: None,
            actions: vec![
                ActionSpec::new(format!(
                    r#"{{"kind":44,"marks":"{}","codeconf":1,"codes":"{}"}}"#,
                    bytes_json(&marks),
                    bytes_json(&codes)
                ))
                .expect("test action spec"),
            ],
        }
    }

    #[test]
    fn decode_call_matches_vm_contract_main_call_wire() {
        let codes = vec![0xdeu8, 0xad, 0xbe, 0xef, 0x01, 0x02];
        let mut call = vm::action::ContractMainCall::new();
        call.marks = Fixed3::from([0xaa, 0xbb, 0xcc]);
        call.codeconf = Uint1::from(1);
        call.codes = BytesW2::from(codes.clone()).unwrap();
        let decoded = decode_call(&hex::encode(call.encode())).unwrap();
        assert_eq!(decoded.kind, vm::action::ContractMainCall::KIND);
        assert_eq!(decoded.marks, "aabbcc");
        assert!(!decoded.marks_valid);
        assert_eq!(decoded.codeconf, 1);
        assert_eq!(decoded.code_type, 1);
        assert_eq!(decoded.code_type_name, "irnode");
        assert_eq!(decoded.codes_len, codes.len());
        assert_eq!(decoded.codes_preview, hex::encode(&codes));
        assert_eq!(decoded.codes_hash, hex::encode(sys::calculate_hash(&codes)));
        // Masks are `vm::action` re-exports of `CodeType::TYPE_MASK` /
        // `CodeConf::RESERVED_MASK` (batch 2 cross-assert kept as a numeric lock).
        assert_eq!(CODECONF_TYPE_MASK, vm::action::CODECONF_TYPE_MASK);
        assert_eq!(CODECONF_RESERVED_MASK, vm::action::CODECONF_RESERVED_MASK);
        assert_eq!(CODECONF_TYPE_MASK, 0b0000_0011);
        assert_eq!(CODECONF_RESERVED_MASK, 0b1111_1100);
    }

    #[test]
    fn rejects_a_non_maincall_action() {
        let built = build_transaction(&TransactionSpec {
            schema: None,
            tx_type: 2,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            addrlist: None,
            actions: vec![
                ActionSpec::new(format!(r#"{{"kind":1,"to":"{MAIN}","hacash":"12:244"}}"#))
                    .expect("test action spec"),
            ],
        })
        .unwrap();
        let decoded = crate::inspect::decode_transaction_json(
            &built.body,
            &crate::audit::DescribeOptions::default(),
        )
        .unwrap();
        let error = decode_call(&decoded.actions[0].raw).unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(error.message.contains("not contract_main_call"));
    }
}

// ================================ vm.code tests ================================

#[test]
fn vm_code_disassembles_bytecode() {
    // A tiny valid bytecode stream: push 1, push 1, add, return (END).
    // (Opcodes P1 = 0x25, ADD = 0xb0, END = 0xef; all zero-param.)
    let codes = vec![0x25u8, 0x25, 0xb0, 0xef];
    let out = code(&hex::encode(&codes), "0", None, None, None, None).unwrap();
    assert_eq!(out.code_type_name, "bytecode");
    assert_eq!(out.format, "assembly");
    assert_eq!(out.codes_len, codes.len());
    assert_eq!(out.codes_hash, hex::encode(sys::calculate_hash(&codes)));
    assert!(!out.truncated);
    assert!(!out.text.is_empty());
}

#[test]
fn vm_code_bytecode_rejects_non_assembly_format() {
    let codes = vec![0xeeu8];
    let error = code(&hex::encode(&codes), "0", Some("fitsh"), None, None, None).unwrap_err();
    assert!(error.message.contains("not valid for bytecode"));
}

#[test]
fn vm_code_rejects_invalid_code_type() {
    let error = code("00", "7", None, None, None, None).unwrap_err();
    assert!(error.message.contains("invalid codeconf"));
}

#[test]
fn vm_code_ir_fitsh_roundtrip_and_paging() {
    // Serialized IR of `let a = 1\nreturn a + 2` (precomputed; the codec-only
    // SDK build has no fitsh compiler — the decompiler is the offline view).
    // `0x80` is GET0 in the current map: IR payload bytes share the runtime opcode
    // space, so a fixture is pinned to whatever byte the map puts the op on.
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let out = code(
        &hex::encode(&ircode),
        "1",
        Some("fitsh"),
        None,
        Some(16),
        Some(0),
    )
    .unwrap();
    assert_eq!(out.code_type_name, "irnode");
    assert_eq!(out.format, "fitsh");
    assert!(
        out.text.len() <= 16,
        "limit applied, got {}",
        out.text.len()
    );
    assert!(out.truncated, "short limit must truncate");
    // Full decode still recovers the original source shape.
    let full = code(&hex::encode(&ircode), "1", None, None, None, None).unwrap();
    assert_eq!(full.format, "fitsh");
    assert!(full.text.contains("a"), "fitsh text: {}", full.text);
}

#[test]
fn fitsh_review_ignores_value_and_structure_rewriting_maps() {
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let map = r#"{"consts":[{"name":"VALUE","value":"1"},{"name":"VALUE","value":"999"}],"slots":["out\nassert safe == true\n//"]}"#;
    let out = code(&hex::encode(&ircode), "1", Some("fitsh_review"), Some(map), None, None).unwrap();
    assert!(out.text.contains("1"), "actual literal must remain: {}", out.text);
    assert!(!out.text.contains("999"), "external constant must not rewrite value");
    assert!(!out.text.contains("assert safe"), "slot label must not inject code");
    assert_eq!(out.sourcemap_status, "partial");
    assert!(!out.warnings.is_empty());
}

#[test]
fn fitsh_review_returns_validated_selector_and_slot_annotations() {
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let selector = hex::encode(vm::calc_func_sign("transfer"));
    let map = format!(r#"{{"funcs":[{{"sig":"{selector}","name":"transfer"}}],"slots":["result"]}}"#);
    let out = code(&hex::encode(&ircode), "1", Some("fitsh_review"), Some(&map), None, None).unwrap();
    assert_eq!(out.sourcemap_status, "accepted");
    assert!(out.sourcemap_labels.iter().any(|v| v.contains(&selector)));
    assert!(out.sourcemap_labels.iter().any(|v| v == "slot $0 = result"));
    assert!(out.text.contains("$0"), "canonical slot must remain in code: {}", out.text);
}

#[test]
fn vm_code_ir_tree_format() {
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let out = code(&hex::encode(&ircode), "1", Some("tree"), None, None, None).unwrap();
    assert_eq!(out.format, "tree");
    assert!(!out.text.is_empty());
}

#[test]
fn vm_code_ir_assembly_format() {
    // Wallet maincall display can request Form 2 ircode assembly instead of fitsh.
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let out = code(
        &hex::encode(&ircode),
        "1",
        Some("assembly"),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(out.format, "assembly");
    assert!(!out.text.is_empty());
}


#[test]
fn vm_code_accepts_external_sourcemap() {
    let smap_json = r#"{"libs":[{"idx":1,"name":"lib.a","address":null}],"funcs":[],"slots":[],"lets":[],"vars":[],"params":[],"param_prelude_count":null,"consts":[]}"#;
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let out = code(
        &hex::encode(&ircode),
        "1",
        None,
        Some(smap_json),
        None,
        None,
    )
    .unwrap();
    assert_eq!(out.format, "fitsh");
    assert!(!out.text.is_empty());
}

/// Real maincall-shaped block (ext call + assert), built through the fitsh
/// parser so the named-display path is exercised end to end.
#[cfg(test)]
fn maincall_ircode_hex() -> String {
    let program = "var out = callext Pool::0x25226e33(0u64, 1000000u64, 0x00e63c33a796b3032ce6b856f68fccf06608d9ed18, 0x)\nassert out >= 1000u64\nend\n";
    let tokens = vm::lang::Tokenizer::new(program.as_bytes()).parse().unwrap();
    let (irnodes, _) = vm::lang::Syntax::new(tokens)
        .with_libs(vec![("Pool".to_string(), 1u8, None)])
        .parse()
        .unwrap();
    // drop_irblock_wrap strips the outer IRBLOCK wrapper from the serialized block.
    let codes = vm::ir::drop_irblock_wrap(vm::IRNode::serialize(&irnodes)).unwrap();
    hex::encode(codes)
}

#[test]
fn fitsh_named_mode_applies_validated_lib_aliases() {
    let codes = maincall_ircode_hex();
    let selector = hex::encode(vm::calc_func_sign("swap"));
    let map = format!(
        r#"{{"libs":[{{"idx":1,"name":"AMM","address":"1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9"}}],"funcs":[{{"sig":"{selector}","name":"swap"}}],"slots":["out"],"lets":[],"vars":[],"params":[],"param_prelude_count":null,"consts":[]}}"#
    );
    let out = code(&codes, "1", Some("fitsh"), Some(&map), None, None).unwrap();
    assert_eq!(out.format, "fitsh");
    assert!(out.text.contains("AMM."), "lib alias must replace ext(1): {}", out.text);
    assert!(!out.text.contains("ext(1)"), "raw ext index must be gone: {}", out.text);
    assert!(!out.text.contains("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9"), "untrusted map address must be dropped from text: {}", out.text);
    assert!(out.text.contains("swap"), "validated selector name must appear: {}", out.text);
    assert!(out.text.contains("out"), "validated slot name must appear: {}", out.text);
    // Source-level sugar: name-only declaration + bare alias call.
    assert!(out.text.contains("var out = AMM.swap("), "named short call form: {}", out.text);
    assert!(!out.text.contains("$0"), "slot id must not shadow the name: {}", out.text);
    assert!(!out.text.contains("call edit"), "edit-effect ext calls use the bare alias form: {}", out.text);
    // Compilability: the named text must reparse to the very same IR through
    // the real fitsh frontend — with NO external lib bindings, because the
    // emitted `lib AMM = 1` prelude carries the binding itself.
    let tokens = vm::lang::Tokenizer::new(out.text.as_bytes()).parse().unwrap();
    let (reir, _) = vm::lang::Syntax::new(tokens).parse().unwrap();
    assert_eq!(
        vm::ir::drop_irblock_wrap(vm::IRNode::serialize(&reir)).unwrap(),
        hex::decode(&codes).unwrap(),
        "named text must recompile to the same IR: {}",
        out.text
    );
    assert_eq!(out.sourcemap_status, "accepted");
    assert!(out.sourcemap_labels.iter().any(|v| v == "lib AMM = 1"));
    assert!(out.sourcemap_labels.iter().any(|v| v.contains(&selector)));
}

#[test]
fn fitsh_named_mode_drops_hostile_labels() {
    let codes = maincall_ircode_hex();
    let selector = hex::encode(vm::calc_func_sign("swap"));
    // Wrong-signature function label, code-injecting slot label, non-identifier
    // lib alias: none may reach the displayed text.
    let map = format!(
        r#"{{"libs":[{{"idx":1,"name":"bad alias!","address":null}}],"funcs":[{{"sig":"{selector}","name":"swap"}},{{"sig":"deadbeef","name":"lie"}}],"slots":["out\nassert false //"],"lets":[],"vars":[],"params":[],"param_prelude_count":null,"consts":[]}}"#
    );
    let out = code(&codes, "1", Some("fitsh"), Some(&map), None, None).unwrap();
    assert_eq!(out.sourcemap_status, "partial");
    assert!(!out.warnings.is_empty());
    assert!(!out.text.contains("bad alias"), "invalid lib alias must fall back to ext(): {}", out.text);
    assert!(out.text.contains("ext(1)"), "fallback keeps the numeric index: {}", out.text);
    assert!(!out.text.contains("lie"), "wrong-signature label must not appear: {}", out.text);
    assert!(!out.text.contains("assert false"), "slot label must not inject code: {}", out.text);
    // The one honest entry still applies.
    assert!(out.text.contains("swap"), "valid labels keep working alongside dropped ones: {}", out.text);
}

/// Multi-param maincall (`param { a, b }` compiles to the ALLOC+UNPACK
/// prelude), built through the fitsh parser; returns (codes hex, the
/// program's own sourcemap JSON).
#[cfg(test)]
fn maincall_param_ircode() -> (String, String) {
    let program = "param { a, b }\nrequire a > b, 1\nend\n";
    let tokens = vm::lang::Tokenizer::new(program.as_bytes()).parse().unwrap();
    let (irnodes, smap) = vm::lang::Syntax::new(tokens).parse().unwrap();
    let codes = vm::ir::drop_irblock_wrap(vm::IRNode::serialize(&irnodes)).unwrap();
    (hex::encode(codes), vm::lang::SourceMap::to_json(&smap).unwrap())
}

#[test]
fn fitsh_named_mode_binds_multi_param_maincall() {
    let (codes, map) = maincall_param_ircode();
    let out = code(&codes, "1", Some("fitsh"), Some(&map), None, None).unwrap();
    assert_eq!(out.sourcemap_status, "accepted", "{:?}", out.warnings);
    assert!(
        out.text.contains("param { a, b }"),
        "sanitized map must carry params so the binding line prints: {}",
        out.text
    );
    assert!(out.text.contains("require a > b"), "{}", out.text);
    let tokens = vm::lang::Tokenizer::new(out.text.as_bytes()).parse().unwrap();
    let (reir, _) = vm::lang::Syntax::new(tokens).parse().unwrap();
    assert_eq!(
        vm::ir::drop_irblock_wrap(vm::IRNode::serialize(&reir)).unwrap(),
        hex::decode(&codes).unwrap(),
        "named text must recompile to the same IR: {}",
        out.text
    );
}

#[test]
fn fitsh_named_mode_degrades_unbound_param_names_to_slots() {
    let (codes, _) = maincall_param_ircode();
    // Valid slot names but no params: nothing declares `a`/`b`, so the body
    // must degrade to `$0`/`$1` with the ALLOC+UNPACK prelude kept intact —
    // not reference names that no printed line ever binds.
    let map = r#"{"slots":["a","b"]}"#;
    let out = code(&codes, "1", Some("fitsh"), Some(map), None, None).unwrap();
    assert!(out.text.contains("local_alloc(2)"), "{}", out.text);
    assert!(out.text.contains("unpack(roll_0(), 0)"), "{}", out.text);
    assert!(out.text.contains("require $0 > $1"), "{}", out.text);
    assert!(!out.text.contains("require a > b"), "{}", out.text);
    let tokens = vm::lang::Tokenizer::new(out.text.as_bytes()).parse().unwrap();
    let (reir, _) = vm::lang::Syntax::new(tokens).parse().unwrap();
    assert_eq!(
        vm::ir::drop_irblock_wrap(vm::IRNode::serialize(&reir)).unwrap(),
        hex::decode(&codes).unwrap(),
        "degraded text must recompile to the same IR: {}",
        out.text
    );
}

#[test]
fn vm_code_zero_limit_returns_empty_page() {
    // Serialized IR of `let a = 1\nreturn a + 2`.
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let out = code(
        &hex::encode(&ircode),
        "1",
        Some("fitsh"),
        None,
        Some(0),
        Some(0),
    )
    .unwrap();
    assert!(
        out.text.is_empty(),
        "explicit limit 0 must page empty, got {:?}",
        out.text
    );
    assert!(out.truncated, "data remains past the empty page");
    assert_eq!(out.next_offset, Some(0));
}
