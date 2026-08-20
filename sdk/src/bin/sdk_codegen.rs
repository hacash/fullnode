//! SDK code generator — renders the JS artifacts from the Rust single sources.
//!
//! ```text
//! cargo run -p sdk --bin sdk_codegen
//! ```
//!
//! Writes:
//! - `sdk/js/generated/{op_tables.mjs, operations.mjs, actionspec.mjs,
//!   actionspec.d.ts}` from `profile::OPERATIONS`/`profile::OP_DEFS` /
//!   `error::ERROR_CODES` / `actionspec::ACTION_SPECS`;
//! - `sdk/tests/golden.json` from `sdk/tests/golden_seed.json` (each payload is
//!   decoded with the production Rust decoder, adding the `decoded` field).
//!
//! `pack.sh` runs this after `codec-schema-gen`; the
//! `codegen::tests::generated_artifacts_match` test and the golden-vector tests
//! verify the checked-in copies stay in sync.

use std::fs;

fn main() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let gen_dir = format!("{manifest}/js/generated");
    fs::create_dir_all(&gen_dir).expect("create js/generated");

    let artifacts = [
        ("op_tables.mjs", sdk::codegen::render_op_tables()),
        ("operations.mjs", sdk::codegen::render_operations_mjs()),
        ("actionspec.mjs", sdk::codegen::render_actionspec_mjs()),
        ("actionspec.d.ts", sdk::codegen::render_actionspec_dts()),
        ("bjson_codec.mjs", sdk::codegen::render_bjson_codec_mjs()),
    ];
    for (name, content) in artifacts {
        let path = format!("{gen_dir}/{name}");
        fs::write(&path, &content).unwrap_or_else(|e| panic!("write {path}: {e}"));
        println!("sdk_codegen: wrote {path}");
    }

    render_golden_json(manifest);
}

/// Decodes every seed payload with the production Rust decoder and writes
/// `sdk/tests/golden.json` (seed + `decoded` field). The seed JSON is parsed
/// with the SDK's own `field::json_*` helpers (the SDK has no serde).
fn render_golden_json(manifest: &str) {
    let seed_path = format!("{manifest}/tests/golden_seed.json");
    let seed = fs::read_to_string(&seed_path).unwrap_or_else(|e| panic!("read {seed_path}: {e}"));

    let mut vectors: Vec<(String, String, String, String)> = Vec::new();
    for (_, value) in field::json_split_object(&seed).expect("seed object") {
        if value.starts_with('[') {
            for (i, vector) in field::json_split_array(value)
                .expect("vectors")
                .iter()
                .enumerate()
            {
                let mut name = String::new();
                let mut friendly = String::new();
                let mut wire = String::new();
                let mut payload = String::new();
                for (key, v) in field::json_split_object(vector).expect("vector object") {
                    match key {
                        "name" => {
                            name = field::json_expect_quoted_decoded(v)
                                .expect("name")
                                .to_owned()
                        }
                        "friendly" => friendly = v.to_owned(),
                        "wire" => wire = v.to_owned(),
                        "payload" => {
                            payload = field::json_expect_quoted_decoded(v)
                                .unwrap_or_else(|e| panic!("vector {i} payload: {e}"))
                                .to_owned()
                        }
                        _ => {}
                    }
                }
                vectors.push((name, friendly, wire, payload));
            }
        }
    }

    let mut out = String::from("{\n  \"vectors\": [\n");
    for (i, (name, friendly, wire, payload)) in vectors.iter().enumerate() {
        let bytes = hex::decode(payload).unwrap_or_else(|e| panic!("vector {i} payload hex: {e}"));
        let decoded = sdk::decode_transaction_spec_binary(&bytes)
            .unwrap_or_else(|e| panic!("vector {i} decode: {e}"));
        let actions: Vec<String> = decoded.actions.iter().map(|a| a.to_json_string()).collect();
        let decoded_json = format!("{{\"actions\":[{}]}}", actions.join(","));
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{name}\",\n"));
        out.push_str(&format!("      \"friendly\": {friendly},\n"));
        out.push_str(&format!("      \"wire\": {wire},\n"));
        out.push_str(&format!("      \"decoded\": {decoded_json},\n"));
        out.push_str(&format!("      \"payload\": \"{payload}\"\n"));
        let comma = if i + 1 == vectors.len() { "" } else { "," };
        out.push_str(&format!("    }}{comma}\n"));
    }
    out.push_str("  ]\n}\n");

    let path = format!("{manifest}/tests/golden.json");
    fs::write(&path, &out).unwrap_or_else(|e| panic!("write {path}: {e}"));
    println!("sdk_codegen: wrote {path}");
}
