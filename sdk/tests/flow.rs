//! End-to-end flow tests (doc 14 §12 acceptance #1/#2/#6): decode → inspect →
//! prepare → vault sign → attach → verify, offline, with the golden Type-2
//! legacy vector and the strict-context guard paths.

use sdk::attach::{attach_signature, prepare_signature, verify_signatures, SignatureProof};
use sdk::build::{build_transaction, ActionSpec, TransactionSpec};
use sdk::inspect::{inspect, inspect_report, InspectContext};
use sdk::profile::{CodecProfile, FULLNODE_COMMIT};
use sdk::schema::SCHEMA_SIGNATURE_PROOF;
use sdk::{evaluate_policy, Policy};

fn profile() -> CodecProfile {
    CodecProfile::standard()
}

/// Golden signed Type-2 HAC+SAT transfer (legacy vector, prikey "123456").
const LEGACY_BODY: &str = "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010002000100e63c33a796b3032ce6b856f68fccf06608d9ed18f8010c000a00e63c33a796b3032ce6b856f68fccf06608d9ed180000000000b71b0000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c9b607f2bd9e1031536c13741facb78585755c116aa7d10628ebc2adbb4be96493bc1bb8ac6c3e78dee6717b9c4a27280b698efc91097d5900418a59c9d8e7ac30000";

fn vault_sign(account: &sys::Account, digest_hex: &str, request_id: &str, request_binding: &str) -> SignatureProof {
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
    // The legacy vector's 1:244 fee sits below the current 50_000 purity
    // floor: the SDK reports the honest protocol fact instead of a fake pass.
    assert_eq!(review.fee_purity_ok, Some(false));

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
            ActionSpec::HacTransfer {
                to: account.readable().to_owned(),
                amount: "12:244".to_owned(),
            },
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

    // prepare → vault sign → attach → verify (offline, no node).
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
    let attached = attach_signature(&built.body, &proof, None, &profile).unwrap();
    assert!(attached.complete);
    assert!(attached.missing_signers.is_empty());

    let verified = verify_signatures(&attached.body).unwrap();
    assert!(verified.ok, "attached body must verify: {:?}", verified.errors);

    // Same key + same signature is idempotent.
    let again = attach_signature(&attached.body, &proof, None, &profile).unwrap();
    assert_eq!(again.body, attached.body);
}

#[test]
fn duplicate_signer_and_not_required_signer_are_rejected() {
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
        actions: vec![ActionSpec::HacTransfer {
            to: account.readable().to_owned(),
            amount: "1:244".to_owned(),
        }],
    })
    .unwrap();

    let request = prepare_signature(&built.body, account.readable(), None, None, None, None, &profile).unwrap();
    let proof = vault_sign(&account, &request.digest, &request.id, &request.request_binding);
    let attached = attach_signature(&built.body, &proof, None, &profile).unwrap();

    // Same key, different signature (sign the non-main hash) → DuplicateSigner.
    let wrong_hash = {
        let tx = sdk::inspect::decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        hex::encode(tx.hash().0) // non-main digest for the main signer
    };
    let bad_proof = vault_sign(&account, &wrong_hash, &request.id, &request.request_binding);
    let error = attach_signature(&attached.body, &bad_proof, None, &profile).unwrap_err();
    assert_eq!(error.code, "duplicate_signer");

    // Signer outside the required set → NotRequiredSigner.
    let other_request =
        prepare_signature(&built.body, other.readable(), None, None, None, None, &profile).unwrap();
    let other_proof = vault_sign(
        &other,
        &other_request.digest,
        &other_request.id,
        &other_request.request_binding,
    );
    let error = attach_signature(&built.body, &other_proof, None, &profile).unwrap_err();
    assert_eq!(error.code, "not_required_signer");
}

#[test]
fn strict_inspect_guard_checks() {
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
            ActionSpec::ChainAllow {
                chains: vec![0],
            },
        ],
    })
    .unwrap();

    inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 1_500_000,
            expected_chain_id: 0,
        },
        &profile,
    )
    .unwrap();

    let error = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 999_999,
            expected_chain_id: 0,
        },
        &profile,
    )
    .unwrap_err();
    assert_eq!(error.code, "expired_height");

    let error = inspect(
        &built.body,
        None,
        &InspectContext {
            current_height: 1_500_000,
            expected_chain_id: 1,
        },
        &profile,
    )
    .unwrap_err();
    assert_eq!(error.code, "wrong_chain_id");
}

#[test]
fn type1_is_reportable_but_not_signable() {
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

    let review = inspect_report(&body, None, &profile).unwrap();
    assert_eq!(review.tx_type, 1);
    assert_eq!(review.signability, "unsupported_tx_type");

    let error =
        prepare_signature(&body, account.readable(), None, None, None, None, &profile).unwrap_err();
    assert_eq!(error.code, "unsupported_tx_type");
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
        actions: vec![ActionSpec::HacTransfer {
            to: account.readable().to_owned(),
            amount: "1:244".to_owned(),
        }],
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
