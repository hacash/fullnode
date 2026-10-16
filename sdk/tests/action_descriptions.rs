//! Full-catalog action description fixtures and report (spec
//! `doc/action-description-spec.md`, version `action-description@1`).
//!
//! Verify mode (plain `cargo test -p sdk`): every fixture input below is
//! decoded through the complete 57-action catalog (protocol + mint-core +
//! vm), the description is recomputed from the real implementation, and it
//! must equal the frozen expectation in `tests/fixtures/action-descriptions.json`
//! byte-for-byte. For the 35 SDK-profile kinds the wire bytes additionally go
//! through `describe_single`, proving Fullnode and SDK emit one identical
//! text; the 22 CALL_ONLY kinds must be rejected there.
//!
//! Update mode (`ACTION_DESCRIPTIONS_UPDATE=1 cargo test -p sdk --test
//! action_descriptions`): regenerates the frozen fixture file and the
//! human-review report `doc/action-description-examples.{json,md}` from the
//! real codecs/descriptions. Inputs (the `json` field of each case) are the
//! fixed test vectors; the generator never invents expected text.
//!
//! Coverage gate: the union of fixture kinds must equal the registry union —
//! every one of the 57 catalog kinds appears at least once, no fixture kind
//! exists outside the catalog, and no duplicate fixture ids.

use base::{ActionCodecBinding, JsonCodecs, WireCodecTable};
use serde::{Deserialize, Serialize};
use sdk::audit::scope_name;
use sdk::{DescribeOptions, describe_single};

use base::ActScope;

const SPEC_VERSION: &str = field::DESC_SPEC_VERSION;
const A: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";
const B: &str = "1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr";

// ================================ catalog ================================

struct FullCatalog {
    table: WireCodecTable,
}

base::test_codec_host!(FullCatalog);

fn full_catalog() -> FullCatalog {
    let mut table = WireCodecTable::new();
    for binding in protocol::ACTION_CODECS {
        table.add_action(*binding).expect("register protocol action");
    }
    for binding in mint_core::ACTION_CODECS {
        table.add_action(*binding).expect("register mint-core action");
    }
    for binding in vm::ACTION_CODECS {
        table.add_action(*binding).expect("register vm action");
    }
    FullCatalog { table }
}

fn all_bindings() -> Vec<ActionCodecBinding> {
    protocol::ACTION_CODECS
        .iter()
        .chain(mint_core::ACTION_CODECS.iter())
        .chain(vm::ACTION_CODECS.iter())
        .copied()
        .collect()
}

/// The wallet SDK profile (mirror of `sdk::selection`: every catalog action
/// whose scope is not `CALL_ONLY`).
fn sdk_profile_kinds() -> Vec<u16> {
    all_bindings()
        .iter()
        .filter(|b| b.scope != ActScope::CALL_ONLY)
        .map(|b| b.schema.kind)
        .collect()
}

// ================================ fixture cases ================================

struct Case {
    fixture_id: String,
    json: String,
}

fn case(fixture_id: &str, json: String) -> Case {
    Case {
        fixture_id: fixture_id.to_owned(),
        json,
    }
}

fn cases() -> Vec<Case> {
    vec![
        // ---- transfers (13 kinds) ----
        case("transfer_hac_to.standard", format!(r#"{{"kind":1,"to":"{A}","hacash":"12:244"}}"#)),
        case("transfer_hac_from.standard", format!(r#"{{"kind":13,"from":"{B}","hacash":"37947332:240"}}"#)),
        case("transfer_hac_from_to.standard", format!(r#"{{"kind":14,"from":"{B}","to":"{A}","hacash":"1:248"}}"#)),
        case("transfer_hac_from_to.pointers_zero", format!(r#"{{"kind":14,"from":2,"to":0,"hacash":"0:0"}}"#)),
        case("transfer_sat_to.standard", format!(r#"{{"kind":10,"to":"{A}","satoshi":500}}"#)),
        case("transfer_sat_from.standard", format!(r#"{{"kind":11,"from":"{B}","satoshi":7}}"#)),
        case("transfer_sat_from_to.standard", format!(r#"{{"kind":12,"from":"{B}","to":"{A}","satoshi":500}}"#)),
        case("transfer_asset_to.standard", format!(r#"{{"kind":17,"to":"{A}","asset":{{"serial":7,"amount":100}}}}"#)),
        case("transfer_asset_from.standard", format!(r#"{{"kind":18,"from":"{B}","asset":{{"serial":1101,"amount":123456789}}}}"#)),
        case("transfer_asset_from_to.standard", format!(r#"{{"kind":19,"from":"{B}","to":"{A}","asset":{{"serial":7,"amount":100}}}}"#)),
        case("transfer_hacd_single_to.standard", format!(r#"{{"kind":5,"to":"{A}","diamond":"WTYUIA"}}"#)),
        case("transfer_hacd_from_to.standard", format!(r#"{{"kind":6,"from":"{B}","to":"{A}","diamonds":"WTYUIA,XVMEKB"}}"#)),
        case("transfer_hacd_to.standard", format!(r#"{{"kind":7,"to":"{A}","diamonds":"WTYUIA,XVMEKB"}}"#)),
        case("transfer_hacd_from.standard", format!(r#"{{"kind":8,"from":"{B}","diamonds":"WTYUIA"}}"#)),
        // ---- payload carriers ----
        case("message.standard", r#"{"kind":1025,"data":"0x68656c6c6f"}"#.to_owned()),
        case("message.quoted_content", r#"{"kind":1025,"data":"0x7361792022686922"}"#.to_owned()),
        case("message.non_printable", r#"{"kind":1025,"data":"0x610062"}"#.to_owned()),
        case("blob.standard", r#"{"kind":1026,"data":"0x01020304"}"#.to_owned()),
        case(
            "blob.truncated_preview",
            format!(r#"{{"kind":1026,"data":"0x{}"}}"#, "5a".repeat(129)),
        ),
        // ---- guards ----
        case("chain_allow.standard", r#"{"kind":1041,"chains":[1,2]}"#.to_owned()),
        case("height_scope.standard", r#"{"kind":1042,"start":1000000,"end":2000000}"#.to_owned()),
        case("height_scope.unlimited", r#"{"kind":1042,"start":1000000,"end":0}"#.to_owned()),
        case(
            "balance_floor.standard",
            format!(r#"{{"kind":1043,"addr":"{A}","hacash":"12:244","satoshi":100,"diamond":5,"assets":[{{"serial":7,"amount":100}}]}}"#),
        ),
        case(
            "balance_floor.partial_pointer",
            r#"{"kind":1043,"addr":1,"hacash":"0:0","satoshi":50,"diamond":0,"assets":[]}"#.to_owned(),
        ),
        case("required_signers.standard", format!(r#"{{"kind":1044,"signers":["{A}","{B}"]}}"#)),
        // ---- AST ----
        case(
            "ast_select.standard",
            format!(r#"{{"kind":25,"exe_min":1,"exe_max":2,"actions":[{{"kind":1,"to":"{A}","hacash":"12:244"}}]}}"#),
        ),
        case(
            "ast_if.standard",
            format!(
                concat!(
                    r#"{{"kind":26,"cond":{{"kind":25,"exe_min":1,"exe_max":1,"actions":[{{"kind":1,"to":"{A}","hacash":"12:244"}}]}},"#,
                    r#""br_if":{{"kind":25,"exe_min":1,"exe_max":1,"actions":[{{"kind":14,"from":"{B}","to":"{A}","hacash":"1:244"}}]}},"#,
                    r#""br_else":{{"kind":25,"exe_min":0,"exe_max":0,"actions":[]}}}}"#
                ),
                A = A,
                B = B,
            ),
        ),
        // ---- TEX ----
        case(
            "tex_cell_execute.standard",
            format!(
                r#"{{"kind":22,"addr":"{A}","cells":[{{"cellid":1,"haczhu":1000000}},{{"cellid":2,"haczhu":2000000}},{{"cellid":3,"satnum":300}},{{"cellid":4,"satnum":400}},{{"cellid":5,"diamonds":"WTYUIA,XVMEKB"}},{{"cellid":6,"dianum":3}},{{"cellid":7,"asset":{{"serial":7,"amount":70}}}},{{"cellid":8,"asset":{{"serial":8,"amount":80}}}},{{"cellid":11,"haczhu":1100000}},{{"cellid":12,"haczhu":1200000}},{{"cellid":13,"haczhu":1300000}},{{"cellid":14,"satoshi":140}},{{"cellid":15,"satoshi":150}},{{"cellid":16,"satoshi":160}},{{"cellid":17,"diamond":17}},{{"cellid":18,"diamond":18}},{{"cellid":19,"diamond":19}},{{"cellid":20,"asset":{{"serial":20,"amount":200}}}},{{"cellid":21,"asset":{{"serial":21,"amount":210}}}},{{"cellid":22,"asset":{{"serial":22,"amount":220}}}},{{"cellid":23,"height":900000}},{{"cellid":24,"height":900001}},{{"cellid":25,"chainid":1}}],"sign":{{"publickey":"0x02{}","signature":"0x{}"}}}}"#,
                "11".repeat(32),
                "ab".repeat(64)
            ),
        ),
        // ---- channels ----
        case(
            "channel_open.standard",
            format!(r#"{{"kind":2,"channel_id":"0xabababababababababababababababab","left_bill":{{"address":"{A}","amount":"1:244"}},"right_bill":{{"address":"{B}","amount":"12:244"}}}}"#),
        ),
        case("channel_close.standard", r#"{"kind":3,"channel_id":"0xabcdabcdabcdabcdabcdabcdabcdabcd"}"#.to_owned()),
        // ---- mints / registration ----
        case(
            "hacd_mint.custom_message",
            format!(r#"{{"kind":4,"d":{{"diamond":"WTYUIA","number":20001,"prev_hash":"0x{}","nonce":"0x{}","address":"{A}","custom_message":"0x{}"}}}}"#, "aa".repeat(32), "bb".repeat(8), "cc".repeat(32)),
        ),
        case(
            "hacd_mint.no_custom_message",
            format!(r#"{{"kind":4,"d":{{"diamond":"WTYUIA","number":100,"prev_hash":"0x{}","nonce":"0x{}","address":"{A}"}}}}"#, "aa".repeat(32), "bb".repeat(8)),
        ),
        case(
            "asset_create.standard",
            format!(r#"{{"kind":16,"protocol_cost":"1:244","metadata":{{"serial":1,"supply":100,"decimal":2,"issuer":"{A}","ticket":"0x7469636b657431","name":"0x6173736574"}}}}"#),
        ),
        case(
            "asset_create.large_supply",
            format!(r#"{{"kind":16,"protocol_cost":"1:244","metadata":{{"serial":2,"supply":9007199254740993,"decimal":0,"issuer":"{A}","ticket":"0x41535432","name":"0x4c61726765"}}}}"#),
        ),
        // ---- inscriptions ----
        case("hacd_insc_push.standard", r#"{"kind":32,"diamonds":"WTYUIA","engraved_content":"0x68656c6c6f","engraved_type":2,"protocol_cost":"0:0"}"#.to_owned()),
        case("hacd_insc_push.fee", r#"{"kind":32,"diamonds":"WTYUIA,XVMEKB","engraved_content":"0x68656c6c6f","engraved_type":2,"protocol_cost":"1:244"}"#.to_owned()),
        case("hacd_insc_push.type_200", r#"{"kind":32,"diamonds":"WTYUIA","engraved_content":"0x68656c6c6f","engraved_type":200,"protocol_cost":"0:0"}"#.to_owned()),
        case("hacd_insc_clean.standard", r#"{"kind":33,"diamonds":"WTYUIA,XVMEKB","protocol_cost":"1:244"}"#.to_owned()),
        case("hacd_insc_edit.standard", r#"{"kind":34,"diamond":"WTYUIA","index":1,"engraved_content":"0x00ff","protocol_cost":"1:244","engraved_type":0}"#.to_owned()),
        case("hacd_insc_move.standard", r#"{"kind":35,"from_diamond":"WTYUIA","to_diamond":"XVMEKB","index":2,"protocol_cost":"1:244"}"#.to_owned()),
        case("hacd_insc_drop.standard", r#"{"kind":36,"diamond":"WTYUIA","index":0,"protocol_cost":"1:244"}"#.to_owned()),
        // ---- contracts / code carriers ----
        case(
            "contract_deploy.standard",
            format!(r#"{{"kind":40,"protocol_cost":"1:244","nonce":7,"construct_argv":"0x0102","marks":"0x00000000","contract":{{"metas":{{"vrsn":0,"revision":0,"mark":"0x000000","mext":"0x00000000"}},"inherit":[],"library":[],"abstcalls":[],"userfuncs":[],"calcfuncs":[],"morextend":0}}}}"#),
        ),
        case(
            "contract_update.standard",
            format!(r#"{{"kind":41,"protocol_cost":"1:244","address":"{A}","marks":"0x0000","edit":{{"new_revision":3,"inherit_add":[],"inherit_replace_at":[],"library_add":[],"library_replace_at":[],"abstcalls":[],"userfuncs":[],"calcfuncs":[]}}}}"#),
        ),
        case(
            "contract_update.replacements",
            format!(r#"{{"kind":41,"protocol_cost":"1:244","address":"{A}","marks":"0x0000","edit":{{"new_revision":4,"inherit_add":[],"inherit_replace_at":[{{"idx":1,"addr":"dLVr1xSixCKt2a6KC6tbjNS6yMSCF68tA"}}],"library_add":[],"library_replace_at":[{{"idx":0,"addr":"dLVr1xSixCKt2a6KC6tbjNS6yMSCF68tA"}}],"abstcalls":[],"userfuncs":[],"calcfuncs":[]}}}}"#),
        ),
        case("contract_main_call.bytecode", r#"{"kind":44,"marks":"0x000000","codeconf":0,"codes":"0x25ef"}"#.to_owned()),
        case("contract_main_call.ir_node", r#"{"kind":44,"marks":"0x000000","codeconf":1,"codes":"0x010203"}"#.to_owned()),
        case("p2sh_script_prove.standard", r#"{"kind":46,"argvkey":"0xaabb","adrlibs":[],"codeconf":0,"lockbox":"0x25ef","merkels":[],"marks":"0x0000"}"#.to_owned()),
        case("p2sh_script_prove.library", r#"{"kind":46,"argvkey":"0xaabb","adrlibs":["dLVr1xSixCKt2a6KC6tbjNS6yMSCF68tA"],"codeconf":0,"lockbox":"0x25ef","merkels":[],"marks":"0x0000"}"#.to_owned()),
        // ---- CALL_ONLY syscalls (22 kinds; Fullnode-only review) ----
        case("block_height.standard", r#"{"kind":1793}"#.to_owned()),
        case("tx_main_addr.standard", r#"{"kind":1794}"#.to_owned()),
        case("block_author_addr.standard", r#"{"kind":1795}"#.to_owned()),
        case("tx_message_num.standard", r#"{"kind":1796}"#.to_owned()),
        case("tx_blob_num.standard", r#"{"kind":1797}"#.to_owned()),
        case("balance_coin.standard", format!(r#"{{"kind":1537,"addr":"{A}"}}"#)),
        case("balance_asset.standard", format!(r#"{{"kind":1538,"addr":"{A}","serial":7}}"#)),
        case("balance_fungible.standard", format!(r#"{{"kind":1539,"addr":"{A}","serial":2}}"#)),
        case("asset_meta.standard", r#"{"kind":1540,"serial":7}"#.to_owned()),
        case(
            "balance_table_check.standard",
            format!(r#"{{"kind":1541,"addr":"{A}","table":[{{"serial":1,"amount":100}},{{"serial":11,"amount":7}}]}}"#),
        ),
        case(
            "balance_fungible_batch.standard",
            format!(r#"{{"kind":1542,"addr":"{A}","serials":[1,2,3,11]}}"#),
        ),
        case("check_signature.standard", format!(r#"{{"kind":1545,"addr":"{A}"}}"#)),
        case("sigset_count.standard", format!(r#"{{"kind":1546,"keys":["{A}","{B}"]}}"#)),
        case(
            "sigset_at_least.standard",
            format!(r#"{{"kind":1547,"keys":["{A}","{B}"],"threshold":1}}"#),
        ),
        case("hacd_insc_num.standard", r#"{"kind":1553,"diamond":"WTYUIA"}"#.to_owned()),
        case("hacd_insc_get.standard", r#"{"kind":1554,"diamond":"WTYUIA","inscidx":2}"#.to_owned()),
        case("hacd_name_list.standard", format!(r#"{{"kind":1555,"addr":"{A}","page":0,"limit":20}}"#)),
        case("hacd_owner_addrs.standard", r#"{"kind":1556,"diamonds":"WTYUIA,XVMEKB"}"#.to_owned()),
        case("tx_message.standard", r#"{"kind":1569,"idx":0}"#.to_owned()),
        case("tx_message_single.standard", r#"{"kind":1570}"#.to_owned()),
        case("tx_blob.standard", r#"{"kind":1571,"idx":0,"start":4,"end":10}"#.to_owned()),
        case("tx_blob_size.standard", r#"{"kind":1572,"idx":1}"#.to_owned()),
    ]
}

// ================================ wallet mode map (MoneyNex appendix A) ================================

fn wallet_mode(name: &str) -> &'static str {
    match name {
        "transfer_hac_to" | "transfer_hac_from" | "transfer_hac_from_to" | "transfer_sat_to"
        | "transfer_sat_from" | "transfer_sat_from_to" | "transfer_hacd_single_to"
        | "transfer_hacd_from_to" | "transfer_hacd_to" | "transfer_hacd_from" => "structured: transfer",
        "transfer_asset_to" | "transfer_asset_from" | "transfer_asset_from_to" => {
            "structured: transfer + asset metadata enrichment"
        }
        "channel_open" => "structured: channel",
        "channel_close" => "decorate",
        "hacd_mint" => "decorate (payload isolated)",
        "asset_create" => "structured: asset metadata",
        "tex_cell_execute" => "structured: TEX cells",
        "ast_select" | "ast_if" => "structured: AST tree",
        "hacd_insc_push" | "hacd_insc_clean" | "hacd_insc_edit" | "hacd_insc_move"
        | "hacd_insc_drop" => "structured: inscription",
        "contract_deploy" | "contract_update" => "structured: contract",
        "contract_main_call" => "verbatim + code enrichment",
        "p2sh_script_prove" => "verbatim + code enrichment",
        "message" => "structured: payload + msglayout enrichment",
        "blob" => "structured: payload",
        "chain_allow" | "balance_floor" | "required_signers" => "structured: guard",
        "height_scope" => "structured: guard + time enrichment",
        _ => "n/a (CALL_ONLY: Fullnode review only)",
    }
}

// ================================ frozen file ================================

#[derive(Serialize, Deserialize, Clone)]
struct FrozenCase {
    fixture_id: String,
    kind: u16,
    name: String,
    scope: String,
    sdk_profile: bool,
    json: String,
    raw: String,
    fullnode_description: String,
    sdk_description: Option<String>,
    wallet_mode: String,
}

#[derive(Serialize, Deserialize)]
struct FrozenFile {
    spec_version: String,
    sdk_version: String,
    catalog_total: usize,
    sdk_profile_total: usize,
    call_only_total: usize,
    cases: Vec<FrozenCase>,
}

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/action-descriptions.json")
}

fn read_frozen() -> FrozenFile {
    let text = std::fs::read_to_string(fixture_path())
        .expect("frozen fixtures missing; run ACTION_DESCRIPTIONS_UPDATE=1 once");
    serde_json::from_str(&text).expect("parse frozen fixtures")
}

fn write_frozen(file: &FrozenFile) {
    let text = serde_json::to_string_pretty(file).expect("serialize frozen fixtures");
    std::fs::write(fixture_path(), text + "\n").expect("write frozen fixtures");
}

// ================================ the test ================================

#[test]
fn full_catalog_descriptions_match_frozen_fixtures() {
    let update = std::env::var("ACTION_DESCRIPTIONS_UPDATE").is_ok();
    let catalog = full_catalog();
    let bindings = all_bindings();
    let profile = sdk_profile_kinds();

    assert_eq!(bindings.len(), 57, "catalog size changed");
    assert_eq!(profile.len(), 35, "SDK profile size changed");
    assert_eq!(
        bindings
            .iter()
            .filter(|b| b.scope == ActScope::CALL_ONLY)
            .count(),
        22,
        "CALL_ONLY size changed"
    );

    let inputs = cases();
    let mut seen_ids = std::collections::HashSet::new();
    let mut covered_kinds = std::collections::HashSet::new();
    let mut generated: Vec<FrozenCase> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for input in &inputs {
        if !seen_ids.insert(input.fixture_id.to_owned()) {
            failures.push(format!("duplicate fixture id {}", input.fixture_id));
        }
        let action = match catalog.decode_action_json(&input.json) {
            Ok(action) => action,
            Err(error) => {
                failures.push(format!(
                    "{}: fixture JSON decode failed: {}",
                    input.fixture_id, error
                ));
                continue;
            }
        };
        let kind = action.kind();
        let binding = bindings
            .iter()
            .find(|b| b.schema.kind == kind)
            .expect("fixture decoded to a catalog kind");
        covered_kinds.insert(kind);
        let in_profile = profile.contains(&kind);
        let description = action.description();
        let raw = hex::encode(action.encode());

        let sdk_description = if in_profile {
            match describe_single(&raw, &DescribeOptions::default()) {
                Ok(desc) => {
                    let text = desc.description.clone();
                    if text.as_deref() != Some(description.as_str()) {
                        failures.push(format!(
                            "{}: SDK description diverged from Fullnode:\n  fullnode: {description}\n  sdk:     {}",
                            input.fixture_id,
                            text.clone().unwrap_or_default()
                        ));
                    }
                    text
                }
                Err(error) => {
                    failures.push(format!(
                        "{}: SDK describe_single failed: {}",
                        input.fixture_id, error
                    ));
                    None
                }
            }
        } else {
            // CALL_ONLY actions must never be describable as ordinary
            // transaction actions through the SDK profile.
            match describe_single(&raw, &DescribeOptions::default()) {
                Ok(_) => {
                    failures.push(format!(
                        "{}: CALL_ONLY kind {} unexpectedly accepted by the SDK profile",
                        input.fixture_id, kind
                    ));
                    None
                }
                Err(_) => None,
            }
        };

        generated.push(FrozenCase {
            fixture_id: input.fixture_id.to_owned(),
            kind,
            name: binding.schema.name.to_owned(),
            scope: scope_name(binding.scope).to_owned(),
            sdk_profile: in_profile,
            json: input.json.clone(),
            raw,
            fullnode_description: description,
            sdk_description,
            wallet_mode: wallet_mode(binding.schema.name).to_owned(),
        });
    }

    // Coverage: every catalog kind appears in at least one fixture, and no
    // fixture kind falls outside the catalog (decode would have failed).
    for binding in &bindings {
        if !covered_kinds.contains(&binding.schema.kind) {
            failures.push(format!(
                "kind {} ({}) has no fixture case",
                binding.schema.kind, binding.schema.name
            ));
        }
    }

    if update {
        assert!(
            failures.is_empty(),
            "fixture generation itself must be clean: {failures:#?}"
        );
        write_frozen(&FrozenFile {
            spec_version: SPEC_VERSION.to_owned(),
            sdk_version: sdk::SDK_VERSION.to_owned(),
            catalog_total: bindings.len(),
            sdk_profile_total: profile.len(),
            call_only_total: bindings.len() - profile.len(),
            cases: generated.clone(),
        });
        write_examples(&generated);
        eprintln!(
            "regenerated {} fixture cases -> {} and doc/action-description-examples.{{json,md}}",
            generated.len(),
            fixture_path().display()
        );
        return;
    }

    let frozen = read_frozen();
    assert_eq!(frozen.spec_version, SPEC_VERSION, "spec version drifted");
    assert_eq!(
        frozen.cases.len(),
        generated.len(),
        "fixture case count drifted (inputs changed without ACTION_DESCRIPTIONS_UPDATE=1)"
    );
    for (have, want) in generated.iter().zip(frozen.cases.iter()) {
        if have.fixture_id != want.fixture_id {
            failures.push(format!(
                "case order drifted: {} vs {}",
                want.fixture_id, have.fixture_id
            ));
            continue;
        }
        for (field, a, b) in [
            ("kind", have.kind.to_string(), want.kind.to_string()),
            ("name", have.name.clone(), want.name.clone()),
            ("scope", have.scope.clone(), want.scope.clone()),
            (
                "sdk_profile",
                have.sdk_profile.to_string(),
                want.sdk_profile.to_string(),
            ),
            ("json", have.json.clone(), want.json.clone()),
            ("raw", have.raw.clone(), want.raw.clone()),
            (
                "fullnode_description",
                have.fullnode_description.clone(),
                want.fullnode_description.clone(),
            ),
            (
                "sdk_description",
                have.sdk_description.clone().unwrap_or_default(),
                want.sdk_description.clone().unwrap_or_default(),
            ),
        ] {
            if a != b {
                failures.push(format!(
                    "{}: {field} drifted\n  frozen:  {b}\n  current: {a}",
                    have.fixture_id
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "action description fixtures drifted (update with ACTION_DESCRIPTIONS_UPDATE=1 only after review):\n{}",
        failures.join("\n")
    );
}

#[test]
fn critical_list_and_type_parameters_change_descriptions() {
    let catalog = full_catalog();
    let describe = |json: &str| catalog.decode_action_json(json).unwrap().description();
    let addr_a = "dLVr1xSixCKt2a6KC6tbjNS6yMSCF68tA";
    let update = |idx: u8, addr: &str| format!(
        r#"{{"kind":41,"protocol_cost":"0:0","address":"{A}","marks":"0x0000","edit":{{"new_revision":4,"inherit_add":[],"inherit_replace_at":[],"library_add":[],"library_replace_at":[{{"idx":{idx},"addr":"{addr}"}}],"abstcalls":[],"userfuncs":[],"calcfuncs":[]}}}}"#
    );
    assert_ne!(describe(&update(0, addr_a)), describe(&update(1, addr_a)));
    let p2sh = |addr: &str| format!(r#"{{"kind":46,"argvkey":"0xaabb","adrlibs":["{addr}"],"codeconf":0,"lockbox":"0x25ef","merkels":[],"marks":"0x0000"}}"#);
    assert_ne!(describe(&p2sh(addr_a)), describe(r#"{"kind":46,"argvkey":"0xaabb","adrlibs":[],"codeconf":0,"lockbox":"0x25ef","merkels":[],"marks":"0x0000"}"#));
    let inscription = |kind: u8| format!(r#"{{"kind":32,"diamonds":"WTYUIA","engraved_content":"0x68656c6c6f","engraved_type":{kind},"protocol_cost":"0:0"}}"#);
    assert_ne!(describe(&inscription(2)), describe(&inscription(200)));
}

// ================================ report generation ================================

fn write_examples(cases: &[FrozenCase]) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../doc");
    let json_path = root.join("action-description-examples.json");
    let md_path = root.join("action-description-examples.md");

    let json = serde_json::json!({
        "spec_version": SPEC_VERSION,
        "sdk_version": sdk::SDK_VERSION,
        "generated_by": "cargo test -p sdk --test action_descriptions (ACTION_DESCRIPTIONS_UPDATE=1)",
        "cases": cases,
    });
    std::fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap() + "\n")
        .expect("write examples json");

    let mut md = String::new();
    md.push_str("# Action description examples — full catalog\n\n");
    md.push_str(&format!(
        "Machine-generated by the real codecs and `description()` implementations. \
         Spec version `{SPEC_VERSION}`, SDK version `{}`. Regenerate with \
         `ACTION_DESCRIPTIONS_UPDATE=1 cargo test -p sdk --test action_descriptions`.\n\n\
         Catalog: 57 actions (44 protocol + 9 mint-core + 4 vm); 35 in the wallet SDK \
         profile, 22 CALL_ONLY (VM-only, reviewed here through the Fullnode codec layer \
         and rejected by `action.describe`).\n\n",
        sdk::SDK_VERSION
    ));
    md.push_str("Every entry below is real output for the listed fixture input; the \
                 checklist items are deliberately unchecked pending human review.\n\n");

    let mut ordered: Vec<&FrozenCase> = cases.iter().collect();
    ordered.sort_by_key(|c| c.kind);
    for case in ordered {
        md.push_str(&format!(
            "## kind {} — `{}`\n\n- fixture: `{}`; scope: `{}`; SDK profile: {}\n- wallet mode: {}\n- input JSON: `{}`\n- raw wire: `0x{}`\n- Fullnode description: `{}`\n",
            case.kind,
            case.name,
            case.fixture_id,
            case.scope,
            case.sdk_profile,
            case.wallet_mode,
            escape_md(&case.json),
            case.raw,
            escape_md(&case.fullnode_description)
        ));
        if case.sdk_profile {
            md.push_str(&format!(
                "- SDK description: `{}` — identical to Fullnode\n",
                escape_md(case.sdk_description.as_deref().unwrap_or(""))
            ));
        } else {
            md.push_str("- SDK description: n/a — CALL_ONLY, not a transaction action\n");
        }
        md.push_str("\nChecklist (human review, not auto-signed):\n\n");
        md.push_str("- [ ] semantics correct\n- [ ] parameters complete\n- [ ] units/direction correct\n- [ ] concise and unambiguous\n- [ ] output stable\n- [ ] reviewer comments handled\n\n");
    }
    std::fs::write(&md_path, md).expect("write examples markdown");
}

fn escape_md(text: &str) -> String {
    text.replace('\\', "\\\\").replace('`', "\\`")
}
