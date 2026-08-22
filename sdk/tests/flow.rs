//! End-to-end flow tests (doc 14 §12 acceptance #1/#2/#6): decode → inspect →
//! prepare → vault sign → attach → verify, offline, with golden Type-2 vector.

use sdk::attach::{SignatureProof, attach_signature, prepare_signature, verify_signatures};
use sdk::build::{ActionSpec, TransactionSpec, build_transaction};
use sdk::inspect::{InspectContext, inspect, inspect_report};
use sdk::profile::{CodecProfile, FULLNODE_COMMIT};
use sdk::schema::SCHEMA_SIGNATURE_PROOF;
use sdk::{Policy, WireValue, evaluate_policy};

fn profile() -> CodecProfile {
    CodecProfile::standard()
}

/// Golden signed Type-2 HAC+SAT transfer (legacy vector, prikey "123456").
const LEGACY_BODY: &str = "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010002000100e63c33a796b3032ce6b856f68fccf06608d9ed18f8010c000a00e63c33a796b3032ce6b856f68fccf06608d9ed180000000000b71b0000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c9b607f2bd9e1031536c13741facb78585755c116aa7d10628ebc2adbb4be96493bc1bb8ac6c3e78dee6717b9c4a27280b698efc91097d5900418a59c9d8e7ac30000";

fn wv_str(s: &str) -> WireValue {
    WireValue::Str(s.to_owned())
}
fn wv_num(n: u64) -> WireValue {
    WireValue::Num(n)
}
fn wv_hex(bytes: impl AsRef<[u8]>) -> WireValue {
    WireValue::Hex(bytes.as_ref().to_vec())
}
fn wv_dia(name: &str) -> WireValue {
    WireValue::Hex(name.as_bytes().to_vec())
}
fn chain_ids(ids: &[u32]) -> WireValue {
    WireValue::List(ids.iter().map(|id| WireValue::Num(u64::from(*id))).collect())
}
fn action(kind: &str, fields: Vec<(&str, WireValue)>) -> ActionSpec {
    ActionSpec::new(
        kind,
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn hac_transfer(to: &str, amount: &str) -> ActionSpec {
    action(
        "transfer_hac_to",
        vec![("to", wv_str(to)), ("hacash", wv_str(amount))],
    )
}
fn height_scope(start: u64, end: u64) -> ActionSpec {
    action(
        "height_scope",
        vec![("start", wv_num(start)), ("end", wv_num(end))],
    )
}
fn chain_allow(ids: &[u32]) -> ActionSpec {
    action("chain_allow", vec![("chains", chain_ids(ids))])
}
fn req_sign_list(signers: &[&str]) -> ActionSpec {
    action(
        "req_sign_list",
        vec![(
            "signers",
            WireValue::List(signers.iter().map(|s| wv_str(s)).collect()),
        )],
    )
}
fn insc_push(diamonds: &[&str], content: &str) -> ActionSpec {
    action(
        "hacd_insc_push",
        vec![
            (
                "diamonds",
                WireValue::List(diamonds.iter().map(|d| wv_dia(d)).collect()),
            ),
            ("protocol_cost", wv_str("0")),
            ("engraved_type", wv_num(0)),
            ("engraved_content", wv_hex(content.as_bytes())),
        ],
    )
}

fn vault_sign(
    account: &sys::Account,
    digest_hex: &str,
    request_id: &str,
    request_binding: &str,
) -> SignatureProof {
    let digest = hex::decode(digest_hex).unwrap();
    let signature: [u8; 64] = account.do_sign(&digest.try_into().unwrap());
    SignatureProof {
        schema: SCHEMA_SIGNATURE_PROOF.to_owned(),
        request_id: request_id.to_owned(),
        request_binding: request_binding.to_owned(),
        public_key: hex::encode(account.public_key().serialize_compressed()),
        signature: hex::encode(signature),
        algorithm: "secp256k1-rfc6979-sha256".to_owned(),
    }
}

#[test]
fn legacy_golden_vector_inspects_and_signature_report_matches() {
    let review = inspect_report(LEGACY_BODY, None, &profile(), &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.tx_type, 2);
    assert_eq!(review.signability, "signable");
    assert_eq!(review.auditability, "full");
    assert!(!review.requires_user_confirmation);
    assert_eq!(review.actions.len(), 2);
    assert_eq!(review.actions[0].name.as_deref(), Some("transfer_hac_to"));
    assert_eq!(review.actions[1].name.as_deref(), Some("transfer_sat_to"));
    assert_eq!(
        review.actions[0].transfer.as_ref().unwrap().payload,
        sdk::audit::PayloadDesc::Hac {
            amount: "12:248".to_owned()
        }
    );
    // The legacy vector carries one valid main signature.
    assert_eq!(review.required_signers.len(), 1);
    assert_eq!(review.missing_signers.len(), 0);
    assert!(!review.review_binding.is_empty());
    // Report mode has no block height, so the height-dependent purity-floor
    // comparison is absent (the raw `fee_purity` fact stays).
    assert_eq!(review.fee_purity_ok, None);
    assert!(review.fee_purity.is_some());

    let report = sdk::attach::signature_report(LEGACY_BODY).unwrap();
    assert_eq!(report.valid.len(), 1);
    assert_eq!(report.required, report.present);
}

#[test]
fn full_offline_sign_flow_type2() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let spec = TransactionSpec {
        schema: Some(sdk::schema::SCHEMA_TRANSACTION_SPEC.to_owned()),
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![
            hac_transfer(account.readable(), "12:244"),
            height_scope(1_000_000, 0),
        ],
    };
    let built = build_transaction(&spec).unwrap();
    assert_eq!(built.tx_type, 2);

    // Strict inspect passes inside the guard range.
    let review = inspect(
        &built.body,
        Some(account.readable()),
        &InspectContext {
            current_height: 1_100_000,
            expected_chain_id: 0,
            consensus_flags: None,
        },
        &profile,
        &sdk::DescribeOptions::default(),
    )
    .unwrap();
    assert_eq!(review.valid_height_range.as_ref().unwrap().start, 1_000_000);
    assert_eq!(review.signability, "signable");
    // Strict mode knows the claimed height, so the purity-floor comparison
    // is present (the scheduled floor at 1_100_000 is the initial floor).
    assert!(review.fee_purity_ok.is_some());

    // prepare → vault sign → attach → verify (offline, no node). The full
    // approval chain review → request → proof is handed to attach.
    let request = prepare_signature(
        &built.body,
        account.readable(),
        Some(&review),
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    assert_eq!(request.purpose, "transaction");
    assert_eq!(request.digest.len(), 64);
    let proof = vault_sign(
        &account,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let attached = attach_signature(&built.body, &proof, &review, &request, &profile).unwrap();
    assert!(attached.complete);
    assert!(attached.missing_signers.is_empty());

    let verified = verify_signatures(&attached.body).unwrap();
    assert!(
        verified.ok,
        "attached body must verify: {:?}",
        verified.errors
    );

    // Same key + same signature is idempotent; the approval chain still holds
    // for the attached body (the unsigned body hash is unchanged).
    let again = attach_signature(&attached.body, &proof, &review, &request, &profile).unwrap();
    assert_eq!(again.body, attached.body);
}

#[test]
fn attach_is_mechanical_and_never_judges_chain_signer_rules() {
    let account = sys::Account::create_by("123456").unwrap();
    let other = sys::Account::create_by("654321").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();

    let request = prepare_signature(
        &built.body,
        account.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &account,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    // The low-level unbound path attaches without an approval chain; the full
    // path is exercised by the flow test above.
    let attached = sdk::attach::attach_signature_unbound(&built.body, &proof, &profile).unwrap();
    assert!(attached.complete);

    // Same key, cryptographically invalid signature: insert is mechanical;
    // completeness and tx.verify report that it does not check.
    let wrong_hash = {
        let tx = sdk::inspect::decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        hex::encode(tx.hash().0) // non-main digest for the main signer
    };
    let bad_proof = vault_sign(&account, &wrong_hash, &request.id, &request.request_binding);
    let attached_bad =
        sdk::attach::attach_signature_unbound(&attached.body, &bad_proof, &profile).unwrap();
    assert!(
        !attached_bad.complete || !attached_bad.invalid_signers.is_empty(),
        "bad signature attaches and is reported, got complete={} invalid={:?}",
        attached_bad.complete,
        attached_bad.invalid_signers
    );
    let verified_bad = verify_signatures(&attached_bad.body).unwrap();
    assert!(
        !verified_bad.ok,
        "tx.verify reports the digest mismatch: {:?}",
        verified_bad.errors
    );

    // Signer outside the required set: type 2 tolerates the extra signature,
    // so attach succeeds and completeness is reported, not gated.
    let other_request = prepare_signature(
        &built.body,
        other.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let other_proof = vault_sign(
        &other,
        &other_request.digest,
        &other_request.id,
        &other_request.request_binding,
    );
    let attached_extra =
        sdk::attach::attach_signature_unbound(&built.body, &other_proof, &profile).unwrap();
    assert!(
        attached_extra
            .missing_signers
            .iter()
            .any(|addr| addr == account.readable()),
        "type-2 attach of a non-required signer must succeed and report the missing main signer"
    );

    // Type 3 requires the exact signer set at execute time, but attach never
    // judges it: a non-D signer attaches and the body fails `tx.verify`.
    let type3 = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 3,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: Some(10),
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let other_request = prepare_signature(
        &type3.body,
        other.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let other_proof = vault_sign(
        &other,
        &other_request.digest,
        &other_request.id,
        &other_request.request_binding,
    );
    let attached_non_d =
        sdk::attach::attach_signature_unbound(&type3.body, &other_proof, &profile).unwrap();
    let verified = verify_signatures(&attached_non_d.body).unwrap();
    assert!(
        !verified.ok,
        "a type-3 body with a non-D signer must not verify (chain rule, not an SDK refusal)"
    );
}

#[test]
fn strict_inspect_reports_guard_facts_instead_of_denying() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![
            height_scope(1_000_000, 2_000_000),
            chain_allow(&[0]),
        ],
    })
    .unwrap();

    // Inside the guard range: the strict review reports the derived facts.
    let review = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 1_500_000,
            expected_chain_id: 0,
            consensus_flags: None,
        },
        &profile,
        &sdk::DescribeOptions::default(),
    )
    .unwrap();
    assert_eq!(review.valid_height_range.as_ref().unwrap().start, 1_000_000);
    assert_eq!(review.expired_height, Some(false));
    assert_eq!(review.wrong_chain, Some(false));

    // Outside the height range: the review still returns, reporting the
    // expired fact — the upper layer decides whether to proceed.
    let review = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 999_999,
            expected_chain_id: 0,
            consensus_flags: None,
        },
        &profile,
        &sdk::DescribeOptions::default(),
    )
    .unwrap();
    assert_eq!(review.expired_height, Some(true));
    assert_eq!(review.wrong_chain, Some(false));

    // Wrong chain: reported as a fact, never a denial.
    let review = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 1_500_000,
            expected_chain_id: 1,
            consensus_flags: None,
        },
        &profile,
        &sdk::DescribeOptions::default(),
    )
    .unwrap();
    assert_eq!(review.expired_height, Some(false));
    assert_eq!(review.wrong_chain, Some(true));

    // Report mode (no context) carries no derived facts.
    let review = inspect_report(&built.body, None, &profile, &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.expired_height, None);
    assert_eq!(review.wrong_chain, None);
}

#[test]
fn type1_is_outside_the_sdk_capability_profile() {
    use field::Encode;

    let account = sys::Account::create_by("123456").unwrap();
    let tx = protocol::tx_std::TransactionType1::new_by(
        field::Address::from(*account.address()),
        field::Amount::from("1:244").unwrap(),
        1_755_223_764,
    );
    let error = inspect_report(&hex::encode(tx.encode()), None, &profile(), &sdk::DescribeOptions::default()).unwrap_err();
    assert_eq!(error.code, "parse_failed");
    assert!(
        error
            .message
            .contains("transaction type 1 not registered")
    );

    let error = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 1,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap_err();
    assert_eq!(error.code, "parse_failed");
    assert!(
        error
            .message
            .contains("transaction type 1 not registered")
    );
}

#[test]
fn inspect_consensus_flags_none_does_not_judge_activation() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let without = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 1,
            expected_chain_id: 0,
            consensus_flags: None,
        },
        &profile,
        &sdk::DescribeOptions::default(),
    )
    .unwrap();
    assert!(
        without
            .inspect_context
            .as_ref()
            .unwrap()
            .consensus_flags
            .is_none()
    );
    assert!(
        without
            .schedule_violations
            .iter()
            .all(|f| !f.contains("not activated")),
        "{:?}",
        without.schedule_violations
    );
    let with_zero = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 1,
            expected_chain_id: 0,
            consensus_flags: Some(0),
        },
        &profile,
        &sdk::DescribeOptions::default(),
    )
    .unwrap();
    assert_eq!(
        with_zero.inspect_context.as_ref().unwrap().consensus_flags,
        Some(0)
    );
    // Ordinary transfers need no flags; Some(0) is judged and still empty.
    assert!(
        with_zero
            .schedule_violations
            .iter()
            .all(|f| !f.contains("not activated")),
        "{:?}",
        with_zero.schedule_violations
    );
}

#[test]
fn policy_evaluate_over_review() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let review = inspect_report(&built.body, None, &profile, &sdk::DescribeOptions::default()).unwrap();
    let decision = evaluate_policy(&review, &Policy::default()).unwrap();
    assert_eq!(decision.decision, "allow");
    assert_eq!(decision.review_binding, review.review_binding);
}

#[test]
fn codec_profile_is_pinned() {
    let profile = profile();
    assert_eq!(profile.fullnode_commit, FULLNODE_COMMIT);
    assert_eq!(profile.schema, sdk::schema::SCHEMA_CODEC_PROFILE);
    assert!(profile.limits.max_tx_size >= 16 * 1024);
    assert_eq!(
        profile.params_version,
        hacash_params::MAINNET_PARAMS.version
    );
    assert!(!profile.registry_hash.is_empty());
    assert!(!profile.profile_hash.is_empty());
    assert_eq!(profile.registered_tx_types, vec![2, 3]);
    // Pinned: registry must include the VM actions.
    for kind in [40u16, 41, 44, 46] {
        assert!(profile.registered_kinds.contains(&kind));
    }
}

#[test]
fn type2_multi_signer_attaches_incrementally() {
    let main = sys::Account::create_by("123456").unwrap();
    let second = sys::Account::create_by("second-key-9").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: main.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![
            req_sign_list(&[second.readable()]),
            hac_transfer(main.readable(), "1:244"),
        ],
    })
    .unwrap();

    // Attach the non-main required signer first: partial signature sets must
    // be accepted and reported as incomplete.
    let request = prepare_signature(
        &built.body,
        second.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &second,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let first = sdk::attach::attach_signature_unbound(&built.body, &proof, &profile).unwrap();
    assert!(!first.complete);
    assert!(
        first
            .missing_signers
            .iter()
            .any(|addr| addr == main.readable())
    );

    // Then the main signer: the set becomes complete and fully verifiable.
    let request = prepare_signature(
        &first.body,
        main.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &main,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let attached = sdk::attach::attach_signature_unbound(&first.body, &proof, &profile).unwrap();
    assert!(attached.complete);
    assert!(attached.missing_signers.is_empty());

    let verified = verify_signatures(&attached.body).unwrap();
    assert!(verified.ok, "{:?}", verified.errors);
}

#[test]
fn type3_multi_signer_attaches_incrementally() {
    let main = sys::Account::create_by("123456").unwrap();
    let second = sys::Account::create_by("second-key-9").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 3,
        main: main.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: Some(10),
        actions: vec![
            req_sign_list(&[second.readable()]),
            hac_transfer(main.readable(), "1:244"),
        ],
    })
    .unwrap();

    // Type-3 still lets attach build the signer set incrementally; completeness
    // is reported, not enforced.
    let request = prepare_signature(
        &built.body,
        second.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &second,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let first = sdk::attach::attach_signature_unbound(&built.body, &proof, &profile).unwrap();
    assert!(!first.complete);

    let request = prepare_signature(
        &first.body,
        main.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &main,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let attached = sdk::attach::attach_signature_unbound(&first.body, &proof, &profile).unwrap();
    assert!(attached.complete);

    // The complete Type-3 set must still satisfy the exact-match rule.
    let verified = verify_signatures(&attached.body).unwrap();
    assert!(verified.ok, "{:?}", verified.errors);
}

#[test]
fn tx_encode_round_trips_and_rejects_tampered_input() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "12:244")],
    })
    .unwrap();

    // Untampered round trip reproduces the exact body and hash.
    let decoded = sdk::inspect::decode_transaction_json(&built.body, &sdk::DescribeOptions::default()).unwrap();
    let rebuilt = sdk::inspect::encode_transaction_json(&decoded, None, &profile).unwrap();
    assert_eq!(rebuilt.body, built.body);
    assert_eq!(rebuilt.unsigned_body_hash, built.unsigned_body_hash);

    // Tampering with an action's wire (`raw`) must fail: swapping in a
    // sibling action's wire (decodes fine) yields `transaction_json_mismatch`.
    let sibling = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "12:000")],
    })
    .unwrap();
    let sibling_decoded = sdk::inspect::decode_transaction_json(&sibling.body, &sdk::DescribeOptions::default()).unwrap();
    let mut tampered = decoded.clone();
    tampered.actions[0].raw = sibling_decoded.actions[0].raw.clone();
    let error = sdk::inspect::encode_transaction_json(&tampered, None, &profile).unwrap_err();
    assert_eq!(error.code, "transaction_json_mismatch");

    // Supplying the matching review passes and the review is bound to the
    // rebuilt body; a tampered review fails the binding recomputation.
    let review = inspect_report(&built.body, None, &profile, &sdk::DescribeOptions::default()).unwrap();
    let rebuilt = sdk::inspect::encode_transaction_json(&decoded, Some(&review), &profile).unwrap();
    assert_eq!(rebuilt.body, built.body);
    let mut tampered_review = review.clone();
    tampered_review.fee = "999:244".to_owned();
    let error = sdk::inspect::encode_transaction_json(&decoded, Some(&tampered_review), &profile)
        .unwrap_err();
    assert_eq!(error.code, "review_binding_mismatch");
}

#[test]
fn prepare_and_attach_reject_tampered_review() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let review = inspect_report(&built.body, Some(account.readable()), &profile, &sdk::DescribeOptions::default()).unwrap();

    // Editing a displayed field after inspect must break the binding chain at
    // prepare (the request is never minted for a tampered approval).
    let mut tampered = review.clone();
    tampered.fee = "999:244".to_owned();
    let error = prepare_signature(
        &built.body,
        account.readable(),
        Some(&tampered),
        None,
        None,
        None,
        &profile,
    )
    .unwrap_err();
    assert_eq!(error.code, "review_binding_mismatch");

    // A proof prepared against the genuine review must not attach under a
    // tampered review either.
    let request = prepare_signature(
        &built.body,
        account.readable(),
        Some(&review),
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &account,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let error = attach_signature(&built.body, &proof, &tampered, &request, &profile).unwrap_err();
    assert_eq!(error.code, "review_binding_mismatch");
}

#[test]
fn attach_rejects_tampered_request_fields() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let review = inspect_report(&built.body, Some(account.readable()), &profile, &sdk::DescribeOptions::default()).unwrap();
    let request = prepare_signature(
        &built.body,
        account.readable(),
        Some(&review),
        None,
        None,
        None,
        &profile,
    )
    .unwrap();

    // Editing a request field after prepare while keeping id/binding unchanged
    // must be detected by the binding recomputation.
    let mut tampered = request.clone();
    tampered.expires_at = Some(1_000_000_000_000); // far future, original binding kept
    let proof = vault_sign(
        &account,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let error = attach_signature(&built.body, &proof, &review, &tampered, &profile).unwrap_err();
    assert_eq!(error.code, "invalid_signing_request");

    // Swapping the digest (signer's sign hash) is detected the same way, and
    // even a consistent-but-different digest cannot ride an old binding.
    let mut tampered = request.clone();
    tampered.digest = "00".repeat(32);
    let error = attach_signature(&built.body, &proof, &review, &tampered, &profile).unwrap_err();
    assert_eq!(error.code, "invalid_signing_request");
}

#[test]
fn attach_enforces_request_binding_and_expiry() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let review = inspect_report(&built.body, Some(account.readable()), &profile, &sdk::DescribeOptions::default()).unwrap();

    // A proof for request A attached under request B (different origin →
    // different binding/id) is rejected.
    let request_a = prepare_signature(
        &built.body,
        account.readable(),
        None,
        None,
        None,
        None,
        &profile,
    )
    .unwrap();
    let request_b = prepare_signature(
        &built.body,
        account.readable(),
        None,
        None,
        Some("other"),
        None,
        &profile,
    )
    .unwrap();
    assert_ne!(request_a.id, request_b.id);
    let proof = vault_sign(
        &account,
        &request_a.digest,
        &request_a.id,
        &request_a.request_binding,
    );
    let error = attach_signature(&built.body, &proof, &review, &request_b, &profile).unwrap_err();
    assert_eq!(error.code, "review_binding_mismatch");

    // An expired request is rejected before any signature work.
    let expired = prepare_signature(
        &built.body,
        account.readable(),
        None,
        None,
        None,
        Some(1),
        &profile,
    )
    .unwrap();
    let proof = vault_sign(
        &account,
        &expired.digest,
        &expired.id,
        &expired.request_binding,
    );
    let error = attach_signature(&built.body, &proof, &review, &expired, &profile).unwrap_err();
    assert_eq!(error.code, "request_expired");
}

#[test]
fn prepare_binds_policy_decision_and_attach_never_refuses_for_deny() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    let review = inspect_report(&built.body, Some(account.readable()), &profile, &sdk::DescribeOptions::default()).unwrap();

    // A denying policy still mints the request: the SDK binds the decision as
    // a fact; the caller decides whether a deny stops the flow.
    let denying = Policy {
        deny_kinds: Some(vec![protocol::action_std::HacToTrs::KIND]),
        ..Default::default()
    };
    let request = prepare_signature(
        &built.body,
        account.readable(),
        Some(&review),
        Some(&denying),
        None,
        None,
        &profile,
    )
    .unwrap();
    let decision = request.policy_decision.as_ref().unwrap();
    assert_eq!(decision.decision, "deny");
    assert_eq!(decision.review_binding, review.review_binding);
    assert_eq!(
        decision.policy_binding,
        sdk::policy::policy_binding_of(decision)
    );

    // Attach under the deny decision succeeds mechanically (no refusal): the
    // resulting body is the caller's responsibility.
    let proof = vault_sign(
        &account,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let attached = attach_signature(&built.body, &proof, &review, &request, &profile).unwrap();
    assert!(attached.complete);

    // An allowing policy is evaluated by the SDK and its decision is bound
    // into the request; a forged binding string can no longer ride along.
    let request = prepare_signature(
        &built.body,
        account.readable(),
        Some(&review),
        Some(&Policy::default()),
        None,
        None,
        &profile,
    )
    .unwrap();
    let decision = request.policy_decision.unwrap();
    assert_eq!(decision.decision, "allow");
    assert_eq!(decision.review_binding, review.review_binding);
    assert_eq!(
        decision.policy_binding,
        sdk::policy::policy_binding_of(&decision)
    );
}

#[test]
fn multiple_chain_allow_reviews_intersect() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    // Two ChainAllow actions execute independently, so the effective chain
    // set is the intersection [1] — the review must not display [0,1,2].
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![
            chain_allow(&[0, 1]),
            chain_allow(&[1, 2]),
        ],
    })
    .unwrap();
    let review = inspect_report(&built.body, None, &profile, &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.chain_ids_allowed, Some(vec![1]));
    assert!(review.guard_violations.is_empty());
    // Two guards and no non-guard is a topology finding; protocol_valid is
    // the conjunction of every fact bucket, not guard-only.
    assert!(
        review
            .topology_violations
            .iter()
            .any(|f| f.contains("all GUARD")),
        "{:?}",
        review.topology_violations
    );
    assert!(!review.protocol_valid);

    // A conflicting pair (no common chain) is a protocol fact: the review
    // reports it instead of claiming a valid chain set.
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![
            chain_allow(&[0]),
            chain_allow(&[1]),
        ],
    })
    .unwrap();
    let review = inspect_report(&built.body, None, &profile, &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.chain_ids_allowed, Some(vec![]));
    assert!(!review.protocol_valid);
}

#[test]
fn inscription_actions_registered_in_codec_profile() {
    let profile = profile();
    for kind in [32u16, 33, 34, 35, 36] {
        assert!(
            profile.registered_kinds.contains(&kind),
            "inscription kind {kind} must be registered"
        );
    }
}

#[test]
fn type2_inscription_push_signs_and_verifies() {
    let account = sys::Account::create_by("123456").unwrap();
    let main = account.readable();
    let to = "1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr";
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: main.to_owned(),
        fee: "0.0001".to_owned(),
        timestamp: Some(1_700_000_000),
        gas_max: Some(0),
        actions: vec![
            insc_push(&["AAABBB"], "First HACD inscription!"),
            hac_transfer(to, "0.01"),
        ],
    })
    .unwrap();

    let review = inspect_report(&built.body, Some(&main), &profile(), &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.signability, "signable");
    assert_eq!(review.auditability, "full");
    assert_eq!(review.actions[0].kind, 32);
    assert_eq!(review.actions[0].name.as_deref(), Some("hacd_insc_push"));
    assert!(review.actions[0].transfer.is_none());

    let request = prepare_signature(
        &built.body,
        &main,
        Some(&review),
        None,
        None,
        None,
        &profile(),
    )
    .unwrap();
    let proof = vault_sign(
        &account,
        &request.digest,
        &request.id,
        &request.request_binding,
    );
    let attached = attach_signature(&built.body, &proof, &review, &request, &profile()).unwrap();
    assert!(attached.complete);
    assert!(attached.missing_signers.is_empty());
    let verified = verify_signatures(&attached.body).unwrap();
    assert!(
        verified.ok,
        "inscription tx must verify: {:?}",
        verified.errors
    );

    // decode round-trip preserves the inscription action and re-encodes
    // identically under the same review binding.
    let decoded = sdk::inspect::decode_transaction_json(&attached.body, &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(decoded.actions[0].kind, 32);
    assert_eq!(decoded.actions[0].name.as_deref(), Some("hacd_insc_push"));
    let encoded =
        sdk::inspect::encode_transaction_json(&decoded, Some(&review), &profile()).unwrap();
    assert_eq!(encoded.body, attached.body);
}

#[test]
fn inscription_push_duplicates_build_and_are_chain_execute_rules() {
    let account = sys::Account::create_by("123456").unwrap();
    let main = account.readable();
    // Duplicate diamond names decode fine; ownership/duplication are
    // execute-time chain rules, so the SDK builds rather than refuses.
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: main.to_owned(),
        fee: "0.0001".to_owned(),
        timestamp: Some(1_700_000_000),
        gas_max: Some(0),
        actions: vec![insc_push(&["AAABBB", "AAABBB"], "dup")],
    })
    .expect("duplicate diamonds are wire-valid; rejection is a chain execute rule");
    let review = inspect_report(&built.body, None, &profile(), &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.actions[0].kind, 32);
    assert_eq!(review.actions[0].name.as_deref(), Some("hacd_insc_push"));
    assert!(review.protocol_valid);
    assert!(
        review.topology_violations.is_empty(),
        "inscription at top is protocol-valid topology, got {:?}",
        review.topology_violations
    );
    assert!(
        review.actions[0]
            .audit_notes
            .iter()
            .all(|n| !n.contains("nested_actions") && !n.contains("children collection")),
        "ActScope::AST must not be treated as missing control-flow, got {:?}",
        review.actions[0].audit_notes
    );
}

#[test]
fn inscription_edit_move_drop_build_and_decode() {
    let account = sys::Account::create_by("123456").unwrap();
    let main = account.readable();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: main.to_owned(),
        fee: "0.0001".to_owned(),
        timestamp: Some(1_700_000_000),
        gas_max: Some(0),
        actions: vec![
            action(
                "hacd_insc_edit",
                vec![
                    ("diamond", wv_dia("AAABBB")),
                    ("index", wv_num(0)),
                    ("protocol_cost", wv_str("0")),
                    ("engraved_type", wv_num(0)),
                    ("engraved_content", wv_hex(b"edited")),
                ],
            ),
            action(
                "hacd_insc_move",
                vec![
                    ("from_diamond", wv_dia("AAABBB")),
                    ("to_diamond", wv_dia("TTTUUU")),
                    ("index", wv_num(0)),
                    ("protocol_cost", wv_str("0")),
                ],
            ),
            action(
                "hacd_insc_drop",
                vec![
                    ("diamond", wv_dia("AAABBB")),
                    ("index", wv_num(0)),
                    ("protocol_cost", wv_str("0")),
                ],
            ),
        ],
    })
    .unwrap();
    let decoded = sdk::inspect::decode_transaction_json(&built.body, &sdk::DescribeOptions::default()).unwrap();
    let names: Vec<&str> = decoded
        .actions
        .iter()
        .filter_map(|action| action.name.as_deref())
        .collect();
    assert_eq!(
        names,
        ["hacd_insc_edit", "hacd_insc_move", "hacd_insc_drop"]
    );
}

#[test]
fn oversized_body_decodes_and_reports_limits_facts() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    // A 20KB blob action exceeds the consensus size cap; the SDK still decodes
    // and reports the limit as a review fact, never refusing to inspect.
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![action(
            "tx_blob",
            vec![("data", wv_hex(vec![0xab; 20 * 1024]))],
        )],
    })
    .unwrap();
    assert!(built.body.len() / 2 > hacash_params::MAX_TX_SIZE);

    let review = inspect_report(&built.body, None, &profile, &sdk::DescribeOptions::default()).unwrap();
    assert!(
        review
            .limits_violations
            .iter()
            .any(|note| note.contains("consensus maximum")),
        "oversized body must be reported as a limits fact, got {:?}",
        review.limits_violations
    );
    assert_eq!(review.actions.len(), 1);
}

#[test]
fn host_opcode_is_outside_the_sdk_capability_profile() {
    let account = sys::Account::create_by("123456").unwrap();
    let error = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 3,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![ActionSpec::new("block_height", vec![])],
    })
    .unwrap_err();
    assert_eq!(error.code, "parse_failed");
}

#[test]
fn empty_actions_build_and_inspect_reports_topology() {
    let account = sys::Account::create_by("123456").unwrap();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![],
    })
    .expect("empty action list is wire-legal");
    let review = inspect_report(&built.body, None, &profile(), &sdk::DescribeOptions::default()).unwrap();
    assert!(
        review
            .topology_violations
            .iter()
            .any(|f| f.contains("action length")),
        "{:?}",
        review.topology_violations
    );
    assert!(!review.protocol_valid);
}

#[test]
fn over_tx_actions_max_builds_and_inspect_reports_topology() {
    let account = sys::Account::create_by("123456").unwrap();
    let to = account.readable().to_owned();
    let actions = (0..=hacash_params::TX_ACTIONS_MAX)
        .map(|_| hac_transfer(&to, "1:244"))
        .collect();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: to.clone(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions,
    })
    .expect("action count above consensus max is wire-legal");
    let review = inspect_report(&built.body, None, &profile(), &sdk::DescribeOptions::default()).unwrap();
    assert_eq!(review.actions.len(), hacash_params::TX_ACTIONS_MAX + 1);
    assert!(
        review
            .topology_violations
            .iter()
            .any(|f| f.contains("action length")),
        "{:?}",
        review.topology_violations
    );
    assert!(!review.protocol_valid);
}
