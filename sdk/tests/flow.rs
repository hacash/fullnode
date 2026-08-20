//! End-to-end flow tests (doc 14 §12 acceptance #1/#2/#6): decode → inspect →
//! prepare → vault sign → attach → verify, offline, with the golden Type-2
//! legacy vector and the strict-context guard paths.

use sdk::attach::{SignatureProof, attach_signature, prepare_signature, verify_signatures};
use sdk::build::{ActionSpec, TransactionSpec, build_transaction};
use sdk::inspect::{InspectContext, inspect, inspect_report};
use sdk::profile::{CodecProfile, FULLNODE_COMMIT};
use sdk::schema::SCHEMA_SIGNATURE_PROOF;
use sdk::{Policy, evaluate_policy};

fn profile() -> CodecProfile {
    CodecProfile::standard()
}

/// Golden signed Type-2 HAC+SAT transfer (legacy vector, prikey "123456").
const LEGACY_BODY: &str = "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010002000100e63c33a796b3032ce6b856f68fccf06608d9ed18f8010c000a00e63c33a796b3032ce6b856f68fccf06608d9ed180000000000b71b0000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c9b607f2bd9e1031536c13741facb78585755c116aa7d10628ebc2adbb4be96493bc1bb8ac6c3e78dee6717b9c4a27280b698efc91097d5900418a59c9d8e7ac30000";

fn hac_transfer(to: &str, amount: &str) -> ActionSpec {
    ActionSpec::HacTransfer {
        from: None,
        to: to.to_owned(),
        amount: amount.to_owned(),
    }
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
    let review = inspect_report(LEGACY_BODY, None, &profile()).unwrap();
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
            ActionSpec::HeightScope {
                start: 1_000_000,
                end: 0,
            },
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
        },
        &profile,
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

    // Same key, a signature that is not cryptographically valid for this
    // signer/digest: the protocol's own push_sign (the chain's insertion
    // path) verifies each attached signature and rejects it — the SDK
    // surfaces the protocol boundary error and performs no per-type rule
    // judgment of its own.
    let wrong_hash = {
        let tx = sdk::inspect::decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        hex::encode(tx.hash().0) // non-main digest for the main signer
    };
    let bad_proof = vault_sign(&account, &wrong_hash, &request.id, &request.request_binding);
    let error =
        sdk::attach::attach_signature_unbound(&attached.body, &bad_proof, &profile).unwrap_err();
    assert_eq!(
        error.code, "parse_failed",
        "a signature failing the protocol's push_sign verification is rejected there"
    );

    // Signer outside the required set: type 2 tolerates the extra signature
    // (the chain checks only required signers), so the attach succeeds and
    // completeness is reported instead of being gated.
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

    // Type 3 requires the exact deterministic signer set at execute time, but
    // attach never judges it: a non-D signer attaches mechanically and the
    // resulting body simply fails `tx.verify` (the chain's boundary).
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
            ActionSpec::HeightScope {
                start: 1_000_000,
                end: 2_000_000,
            },
            ActionSpec::ChainAllow { chains: vec![0] },
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
        },
        &profile,
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
        },
        &profile,
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
        },
        &profile,
    )
    .unwrap();
    assert_eq!(review.expired_height, Some(false));
    assert_eq!(review.wrong_chain, Some(true));

    // Report mode (no context) carries no derived facts.
    let review = inspect_report(&built.body, None, &profile).unwrap();
    assert_eq!(review.expired_height, None);
    assert_eq!(review.wrong_chain, None);
}

#[test]
fn type1_is_signable_through_the_full_approval_chain() {
    use base::TransactionBuild;
    use field::Encode;
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let mut tx = protocol::tx_std::TransactionType1::new_by(
        field::Address::from(*account.address()),
        field::Amount::from("1:244").unwrap(),
        1_755_223_764,
    );
    tx.push_action(std::sync::Arc::new(protocol::action_std::HacToTrs::new(
        field::Address::from(*account.address()),
        field::Amount::from("1:244").unwrap(),
    )))
    .unwrap();
    let body = hex::encode(tx.encode());

    // Type 1 is a registered user tx type, so the SDK exposes signing; the
    // chain decides whether it accepts the signed body (flag-gated types).
    let review = inspect_report(&body, None, &profile).unwrap();
    assert_eq!(review.tx_type, 1);
    assert_eq!(review.signability, "signable");

    let request =
        prepare_signature(&body, account.readable(), Some(&review), None, None, None, &profile)
            .unwrap();
    let proof = vault_sign(&account, &request.digest, &request.id, &request.request_binding);
    let attached = attach_signature(&body, &proof, &review, &request, &profile).unwrap();
    assert!(attached.complete);
    let verified = verify_signatures(&attached.body).unwrap();
    assert!(verified.ok, "type-1 body must verify: {:?}", verified.errors);
}

#[test]
fn type1_builds_and_decodes_round_trip() {
    let account = sys::Account::create_by("123456").unwrap();
    let profile = profile();
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 1,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![hac_transfer(account.readable(), "1:244")],
    })
    .unwrap();
    assert_eq!(built.tx_type, 1);
    let review = inspect_report(&built.body, None, &profile).unwrap();
    assert_eq!(review.tx_type, 1);
    assert_eq!(review.signability, "signable");
    // decode → encode reproduces the type-1 body exactly.
    let decoded = sdk::inspect::decode_transaction_json(&built.body).unwrap();
    let rebuilt = sdk::inspect::encode_transaction_json(&decoded, None, &profile).unwrap();
    assert_eq!(rebuilt.body, built.body);
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
    let review = inspect_report(&built.body, None, &profile).unwrap();
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
    assert!(!profile.profile_hash.is_empty());
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
            ActionSpec::ReqSignList {
                signers: vec![second.readable().to_owned()],
            },
            hac_transfer(main.readable(), "1:244"),
        ],
    })
    .unwrap();

    // Attach the non-main required signer first: partial signature sets must
    // be accepted and reported as incomplete.
    let request =
        prepare_signature(&built.body, second.readable(), None, None, None, None, &profile)
            .unwrap();
    let proof = vault_sign(&second, &request.digest, &request.id, &request.request_binding);
    let first = sdk::attach::attach_signature_unbound(&built.body, &proof, &profile).unwrap();
    assert!(!first.complete);
    assert!(first.missing_signers.iter().any(|addr| addr == main.readable()));

    // Then the main signer: the set becomes complete and fully verifiable.
    let request = prepare_signature(&first.body, main.readable(), None, None, None, None, &profile)
        .unwrap();
    let proof = vault_sign(&main, &request.digest, &request.id, &request.request_binding);
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
            ActionSpec::ReqSignList {
                signers: vec![second.readable().to_owned()],
            },
            hac_transfer(main.readable(), "1:244"),
        ],
    })
    .unwrap();

    // Type-3 requires the exact deterministic signer set at execute time, but
    // attach must still build the set up incrementally; completeness is
    // reported, not enforced, per attach.
    let request =
        prepare_signature(&built.body, second.readable(), None, None, None, None, &profile)
            .unwrap();
    let proof = vault_sign(&second, &request.digest, &request.id, &request.request_binding);
    let first = sdk::attach::attach_signature_unbound(&built.body, &proof, &profile).unwrap();
    assert!(!first.complete);

    let request = prepare_signature(&first.body, main.readable(), None, None, None, None, &profile)
        .unwrap();
    let proof = vault_sign(&main, &request.digest, &request.id, &request.request_binding);
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
    let decoded = sdk::inspect::decode_transaction_json(&built.body).unwrap();
    let rebuilt = sdk::inspect::encode_transaction_json(&decoded, None, &profile).unwrap();
    assert_eq!(rebuilt.body, built.body);
    assert_eq!(rebuilt.unsigned_body_hash, built.unsigned_body_hash);

    // Tampering with an action's wire (`raw`) must fail instead of silently
    // emitting a different transaction than the one the declared hash refers
    // to. Build a valid sibling transaction with a different amount and swap
    // its action wire in (decodes fine, produces a different body).
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
    let sibling_decoded = sdk::inspect::decode_transaction_json(&sibling.body).unwrap();
    let mut tampered = decoded.clone();
    tampered.actions[0].raw = sibling_decoded.actions[0].raw.clone();
    let error = sdk::inspect::encode_transaction_json(&tampered, None, &profile).unwrap_err();
    assert_eq!(error.code, "transaction_json_mismatch");

    // Supplying the matching review passes and the review is bound to the
    // rebuilt body; a tampered review fails the binding recomputation.
    let review = inspect_report(&built.body, None, &profile).unwrap();
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
    let review = inspect_report(&built.body, Some(account.readable()), &profile).unwrap();

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
    let review = inspect_report(&built.body, Some(account.readable()), &profile).unwrap();
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

    // The reported bypass: edit a request field after prepare while keeping
    // id/request_binding unchanged. The binding recomputation must detect the
    // edit even though the proof still carries the original id/binding.
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
    let review = inspect_report(&built.body, Some(account.readable()), &profile).unwrap();

    // A proof for request A attached under request B (different origin →
    // different binding/id) is rejected.
    let request_a = prepare_signature(&built.body, account.readable(), None, None, None, None, &profile)
        .unwrap();
    let request_b = prepare_signature(&built.body, account.readable(), None, None, Some("other"), None, &profile)
        .unwrap();
    assert_ne!(request_a.id, request_b.id);
    let proof = vault_sign(&account, &request_a.digest, &request_a.id, &request_a.request_binding);
    let error = attach_signature(&built.body, &proof, &review, &request_b, &profile).unwrap_err();
    assert_eq!(error.code, "review_binding_mismatch");

    // An expired request is rejected before any signature work.
    let expired = prepare_signature(&built.body, account.readable(), None, None, None, Some(1), &profile)
        .unwrap();
    let proof = vault_sign(&account, &expired.digest, &expired.id, &expired.request_binding);
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
    let review = inspect_report(&built.body, Some(account.readable()), &profile).unwrap();

    // A policy that denies the built action kind still mints the request: the
    // SDK evaluates the caller's policy and binds the decision as a fact; the
    // caller decides whether a deny stops the flow.
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
            ActionSpec::ChainAllow { chains: vec![0, 1] },
            ActionSpec::ChainAllow { chains: vec![1, 2] },
        ],
    })
    .unwrap();
    let review = inspect_report(&built.body, None, &profile).unwrap();
    assert_eq!(review.chain_ids_allowed, Some(vec![1]));
    assert!(review.protocol_valid);

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
            ActionSpec::ChainAllow { chains: vec![0] },
            ActionSpec::ChainAllow { chains: vec![1] },
        ],
    })
    .unwrap();
    let review = inspect_report(&built.body, None, &profile).unwrap();
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
            ActionSpec::InscPush {
                diamonds: vec!["AAABBB".to_owned()],
                protocol_cost: None,
                engraved_type: Some(0),
                engraved_content: "First HACD inscription!".to_owned(),
            },
            hac_transfer(to, "0.01"),
        ],
    })
    .unwrap();

    let review = inspect_report(&built.body, Some(&main), &profile()).unwrap();
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
    let decoded = sdk::inspect::decode_transaction_json(&attached.body).unwrap();
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
    // Duplicate diamond names decode fine (the wire carries them); ownership
    // and duplication are execute-time chain rules, so the SDK builds and
    // reports the action instead of refusing to construct it.
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: main.to_owned(),
        fee: "0.0001".to_owned(),
        timestamp: Some(1_700_000_000),
        gas_max: Some(0),
        actions: vec![ActionSpec::InscPush {
            diamonds: vec!["AAABBB".to_owned(), "AAABBB".to_owned()],
            protocol_cost: None,
            engraved_type: Some(0),
            engraved_content: "dup".to_owned(),
        }],
    })
    .expect("duplicate diamonds are wire-valid; rejection is a chain execute rule");
    let review = inspect_report(&built.body, None, &profile()).unwrap();
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
            ActionSpec::InscEdit {
                diamond: "AAABBB".to_owned(),
                index: 0,
                protocol_cost: None,
                engraved_type: Some(0),
                engraved_content: "edited".to_owned(),
            },
            ActionSpec::InscMove {
                from_diamond: "AAABBB".to_owned(),
                to_diamond: "TTTUUU".to_owned(),
                index: 0,
                protocol_cost: None,
            },
            ActionSpec::InscDrop {
                diamond: "AAABBB".to_owned(),
                index: 0,
                protocol_cost: None,
            },
        ],
    })
    .unwrap();
    let decoded = sdk::inspect::decode_transaction_json(&built.body).unwrap();
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
    // A 20KB blob action makes the body exceed the consensus size cap. The
    // SDK decodes it anyway and reports the limit as a review fact instead
    // of refusing to inspect (the upper layer decides).
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 2,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![ActionSpec::TxBlob {
            data: "ab".repeat(20 * 1024),
        }],
    })
    .unwrap();
    assert!(built.body.len() / 2 > base::MAX_TX_SIZE);

    let review = inspect_report(&built.body, None, &profile).unwrap();
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
fn host_opcode_builds_and_inspect_reports_topology_facts() {
    let account = sys::Account::create_by("123456").unwrap();
    // CALL_ONLY host opcodes are wire-valid; the SDK builds and inspects them.
    // Scope rejection is a chain execute rule, reported here as a topology
    // fact so the upper layer can decide.
    let built = build_transaction(&TransactionSpec {
        schema: None,
        tx_type: 3,
        main: account.readable().to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![ActionSpec::RawAction {
            kind: "block_height".to_owned(),
            fields: vec![],
        }],
    })
    .expect("host opcode is wire-valid; rejection is a chain execute rule");
    let review = inspect_report(&built.body, None, &profile()).unwrap();
    assert!(
        review
            .topology_violations
            .iter()
            .any(|note| note.contains("not allowed from")),
        "CALL_ONLY at top must be reported as a topology fact, got {:?}",
        review.topology_violations
    );
    assert_eq!(review.signability, "signable");
    assert_eq!(review.actions.len(), 1);
}
