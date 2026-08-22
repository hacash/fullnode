//! `vm.decode_call`: structured view of a `contract_main_call` action
//! (Unified SDK 2.0, doc 14 §6.3). The wire carries `marks` (reserved, must be
//! zero), `codeconf` (2-bit code type + 6 reserved bits) and the bytecode
//! (`codes`). The code type ABI is frozen in the VM (`vm::rt::CodeType` /
//! `CodeConf`, crate-private here, so the constants are re-declared).

use base::BinaryCodecs;
use field::{BytesW2, Decode, Fixed3, Uint1, Uint2};

use crate::error::{SdkError, SdkErrorCode};
use crate::json::SdkJsonTo;
use crate::schema::SCHEMA_VM_CALL;

/// `codeconf` low bits: code type (0 = bytecode, 1 = IR node); the remaining
/// bits are reserved and must be zero (`CodeConf::RESERVED_MASK`).
pub const CODECONF_TYPE_MASK: u8 = 0b0000_0011;
pub const CODECONF_RESERVED_MASK: u8 = 0b1111_1100;

fn code_type_name(raw: u8) -> (&'static str, u8) {
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
    let (type_name, type_id) = code_type_name(codeconf_raw);
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
/// (bytecode → annotated assembly; ir_node → fitsh source or structural tree)
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
            kv("truncated", if self.truncated { "true".to_owned() } else { "false".to_owned() }),
            kv("limit", qnum(self.limit)),
            kv("offset", qnum(self.offset)),
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
    let (type_name, type_id) = crate::audit::code_type_name(code_type);
    if type_id == 2 {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("unsupported code_type {code_type}; expected 0 (bytecode) or 1 (ir_node)"),
        ));
    }
    let limit = limit.unwrap_or(VM_CODE_DEFAULT_LIMIT);
    let offset = offset.unwrap_or(0);

    // External source map: parse at the boundary so the caller's JSON object
    // rides the wire unchanged (the decompiler consumes `vm::lang::SourceMap`).
    let smap = match sourcemap_json {
        Some(raw) => Some(vm::lang::SourceMap::from_json(raw).map_err(|e| {
            SdkError::new(SdkErrorCode::ParseFailed, format!("sourcemap invalid: {e}"))
        })?),
        None => None,
    };

    let (format_name, full_text) = match (type_id, format) {
        (0, None | Some("assembly")) => (
            "assembly".to_owned(),
            vm::lang::disassemble_bytecode(&codes, true)
                .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, e.to_string()))?,
        ),
        (0, Some(other)) => {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("format {other} is not valid for bytecode; expected assembly"),
            ));
        }
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
        (1, Some(other)) => {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("format {other} is not valid for ir_node; expected fitsh or tree"),
            ));
        }
        _ => unreachable!("code type validated above"),
    };

    let total = full_text.len() as u64;
    let start = offset.min(total) as usize;
    let end = (offset.saturating_add(limit)).min(total) as usize;
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
        offset,
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
            kv("marks_valid", if self.marks_valid { "true".to_owned() } else { "false".to_owned() }),
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
    use crate::spec_codec::WireValue;

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn maincall_spec(codes: Vec<u8>) -> TransactionSpec {
        TransactionSpec {
            schema: None,
            tx_type: 3,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![ActionSpec::new(
                "contract_main_call",
                vec![
                    ("marks".to_owned(), WireValue::Hex(vec![0, 0, 0])),
                    ("codeconf".to_owned(), WireValue::Num(0)),
                    ("codes".to_owned(), WireValue::Hex(codes)),
                ],
            )],
        }
    }

    #[test]
    fn decodes_a_built_maincall() {
        let codes = vec![0x01u8, 0x02, 0x03];
        let built = build_transaction(&maincall_spec(codes.clone())).unwrap();
        let decoded = crate::inspect::decode_transaction_json(&built.body, &crate::audit::DescribeOptions::default()).unwrap();
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
        let decoded = crate::inspect::decode_transaction_json(&built.body, &crate::audit::DescribeOptions::default()).unwrap();
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
            actions: vec![ActionSpec::new(
                "contract_main_call",
                vec![
                    ("marks".to_owned(), WireValue::Hex(marks)),
                    ("codeconf".to_owned(), WireValue::Num(1)),
                    ("codes".to_owned(), WireValue::Hex(codes)),
                ],
            )],
        }
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
            actions: vec![ActionSpec::new(
                "transfer_hac_to",
                vec![
                    ("to".to_owned(), WireValue::Str(MAIN.to_owned())),
                    ("hacash".to_owned(), WireValue::Str("12:244".to_owned())),
                ],
            )],
        })
        .unwrap();
        let decoded = crate::inspect::decode_transaction_json(&built.body, &crate::audit::DescribeOptions::default()).unwrap();
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
    assert!(error.message.contains("unsupported code_type"));
}

#[test]
fn vm_code_ir_fitsh_roundtrip_and_paging() {
    // Serialized IR of `let a = 1\nreturn a + 2` (precomputed; the codec-only
    // SDK build has no fitsh compiler — the decompiler is the offline view).
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
    assert_eq!(out.code_type_name, "ir_node");
    assert_eq!(out.format, "fitsh");
    assert!(out.text.len() <= 16, "limit applied, got {}", out.text.len());
    assert!(out.truncated, "short limit must truncate");
    // Full decode still recovers the original source shape.
    let full = code(&hex::encode(&ircode), "1", None, None, None, None).unwrap();
    assert_eq!(full.format, "fitsh");
    assert!(full.text.contains("a"), "fitsh text: {}", full.text);
}

#[test]
fn vm_code_ir_tree_format() {
    let ircode = hex::decode("7f017c0025eeb08026").unwrap();
    let out = code(&hex::encode(&ircode), "1", Some("tree"), None, None, None).unwrap();
    assert_eq!(out.format, "tree");
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
