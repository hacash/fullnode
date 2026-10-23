//! TEX authorization parsing and review facts. TEX bundles are independent
//! signed authorizations; this module never accesses an account or private key.

use base::ActionCodec;
use field::{Decode, Encode, FromJSON, ToJSON};

use crate::{
    error::{SdkError, SdkErrorCode},
    json::SdkJsonTo,
    jsonparse,
};

pub const SCHEMA_TEX_AUTHORIZATION: &str = "hacash.tex/authorization@1";

#[derive(Debug, Clone, PartialEq)]
pub struct Authorization {
    pub schema: String,
    pub address: String,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellReview {
    pub index: usize,
    pub cellid: u8,
    pub cell: String,
    pub category: &'static str,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub authorization: Authorization,
    pub cells: Vec<CellReview>,
    pub chain_id_condition: Vec<GuardCondition>,
    pub height_conditions: Vec<GuardCondition>,
    pub payments: Vec<CellReview>,
    pub receipts: Vec<CellReview>,
    pub unknown_conditions: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuardCondition {
    pub operator: &'static str,
    pub value: String,
}

impl Authorization {
    pub fn to_json_string(&self) -> String {
        format!(
            "{{\"schema\":{},\"address\":{},\"cells\":[{}]}}",
            field::json_escape(&self.schema),
            field::json_escape(&self.address),
            self.cells.join(",")
        )
    }
}

impl Inspection {
    pub fn to_json_string(&self) -> String {
        let cells = self
            .cells
            .iter()
            .map(|c| {
                format!(
                    "{{\"index\":{},\"cellid\":{},\"cell\":{},\"category\":{},\"description\":{}}}",
                    c.index,
                    c.cellid,
                    c.cell,
                    field::json_escape(c.category),
                    field::json_escape(&c.description)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let conditions = |values: &[GuardCondition]| {
            values
                .iter()
                .map(|c| {
                    format!(
                        "{{\"operator\":{},\"value\":{}}}",
                        field::json_escape(c.operator),
                        field::json_escape(&c.value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let facts = |values: &[CellReview]| {
            values
                .iter()
                .map(|c| {
                    format!(
                        "{{\"index\":{},\"cellid\":{},\"cell\":{}}}",
                        c.index, c.cellid, c.cell
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let unknown = self
            .unknown_conditions
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schema\":\"hacash.tex/inspection@1\",\"authorization\":{},\"address\":{},\"cells\":[{}],\"payments\":[{}],\"receipts\":[{}],\"chain_id_condition\":[{}],\"height_conditions\":[{}],\"repeatable\":true,\"unknown_conditions\":[{}],\"execution_guaranteed\":false}}",
            self.authorization.to_json_string(),
            field::json_escape(&self.authorization.address),
            cells,
            facts(&self.payments),
            facts(&self.receipts),
            conditions(&self.chain_id_condition),
            conditions(&self.height_conditions),
            unknown
        )
    }
}

/// Produce the canonical protocol-owned TEX digest by decoding a default-sign
/// kind-22 action and asking the action implementation for its signing hash.
pub fn authorization_digest(auth: &Authorization) -> Result<String, SdkError> {
    let address = field::Address::from_readable(&auth.address).map_err(SdkError::from)?;
    let addr_json = address.to_json();
    let sign_json = field::Sign::default().to_json();
    let raw = format!(
        "{{\"kind\":22,\"addr\":{addr_json},\"cells\":[{}],\"sign\":{sign_json}}}",
        auth.cells.join(",")
    );
    let mut action = protocol::action_std::TexCellExecute::default();
    action.from_json(&raw).map_err(SdkError::from)?;
    Ok(hex::encode(action.sign_hash().0))
}

const DOMAIN_TEX_BODY: &[u8] = b"hacash.sdk/tex-authorization@1";
const DOMAIN_TEX_REVIEW: &[u8] = b"hacash.sdk/tex-review@1";

fn authorization_body_hash(auth: &Authorization) -> String {
    let body = auth.to_json_string();
    let mut bytes = DOMAIN_TEX_BODY.to_vec();
    bytes.extend_from_slice(body.as_bytes());
    hex::encode(sys::calculate_hash(bytes))
}

fn review_binding(auth: &Authorization, signer: &str) -> Result<String, SdkError> {
    let inspected = inspect(auth)?.to_json_string();
    let material = format!(
        "{{\"authorization\":{},\"signer_address\":{},\"review\":{inspected}}}",
        auth.to_json_string(),
        field::json_escape(signer)
    );
    let mut bytes = DOMAIN_TEX_REVIEW.to_vec();
    bytes.extend_from_slice(material.as_bytes());
    Ok(hex::encode(sys::calculate_hash(bytes)))
}

pub fn build(auth: &Authorization) -> Result<String, SdkError> {
    let digest = authorization_digest(auth)?;
    let inspection = inspect(auth)?.to_json_string();
    Ok(format!(
        "{{\"schema\":\"hacash.tex/bundle@1\",\"authorization\":{},\"address\":{},\"cells\":[{}],\"digest\":{},\"inspection\":{},\"repeatable\":true,\"execution_guaranteed\":false}}",
        auth.to_json_string(),
        field::json_escape(&auth.address),
        auth.cells.join(","),
        field::json_escape(&digest),
        inspection
    ))
}

pub fn prepare_signature(
    auth: &Authorization,
    signer_address: &str,
    origin: Option<&str>,
    expires_at: Option<u64>,
) -> Result<String, SdkError> {
    let signer = field::Address::from_readable(signer_address).map_err(SdkError::from)?;
    if signer.to_readable() != auth.address {
        return Err(SdkError::new(
            SdkErrorCode::ReviewBindingMismatch,
            "TEX authorization address must match signer_address",
        ));
    }
    let mut request = crate::attach::SigningRequest {
        schema: crate::schema::SCHEMA_SIGNING_REQUEST.to_owned(),
        id: String::new(),
        purpose: "tex_authorization".to_owned(),
        algorithm: "secp256k1-rfc6979-sha256".to_owned(),
        signer_address: signer.to_readable(),
        digest: authorization_digest(auth)?,
        body_hash: Some(authorization_body_hash(auth)),
        review_binding: Some(review_binding(auth, &signer.to_readable())?),
        policy_decision: None,
        origin: origin.map(str::to_owned),
        expires_at,
        request_binding: String::new(),
    };
    let binding = crate::attach::request_binding_of(&request);
    request.id = binding.clone();
    request.request_binding = binding;
    Ok(request.to_json_string())
}

fn verify_tex_proof(
    auth: &Authorization,
    proof: &crate::attach::SignatureProof,
    request: &crate::attach::SigningRequest,
) -> Result<(), SdkError> {
    if request.schema != crate::schema::SCHEMA_SIGNING_REQUEST {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            "unsupported signing request schema",
        ));
    }
    crate::attach::verify_request_integrity(request)?;
    crate::attach::validate_proof_format(proof)?;
    if request.purpose != "tex_authorization"
        || request.signer_address != auth.address
        || request.digest != authorization_digest(auth)?
        || request.body_hash.as_deref() != Some(authorization_body_hash(auth).as_str())
        || request.review_binding.as_deref()
            != Some(review_binding(auth, &request.signer_address)?.as_str())
        || proof.request_id != request.id
        || proof.request_binding != request.request_binding
    {
        return Err(SdkError::new(
            SdkErrorCode::InvalidSigningRequest,
            "TEX signature request does not bind this authorization",
        ));
    }
    let (sign, signer) = crate::attach::parse_proof(proof)?;
    if signer.to_readable() != auth.address || signer.to_readable() != request.signer_address {
        return Err(SdkError::new(
            SdkErrorCode::BadSignature,
            "TEX signature public key does not match authorization address",
        ));
    }
    let digest: [u8; 32] = hex::decode(&request.digest)
        .map_err(|e| crate::jsonparse::parse_failed(e.to_string()))?
        .try_into()
        .map_err(|_| crate::jsonparse::parse_failed("TEX digest must be 32 bytes"))?;
    if !sys::Account::verify_signature(&digest, &sign.publickey, &sign.signature) {
        return Err(SdkError::new(
            SdkErrorCode::BadSignature,
            "TEX authorization signature is invalid",
        ));
    }
    Ok(())
}

pub fn attach_signature(
    auth: &Authorization,
    proof: &crate::attach::SignatureProof,
    request: &crate::attach::SigningRequest,
) -> Result<String, SdkError> {
    verify_tex_proof(auth, proof, request)?;
    let address = field::Address::from_readable(&auth.address).map_err(SdkError::from)?;
    let raw = format!(
        "{{\"kind\":22,\"addr\":{},\"cells\":[{}],\"sign\":{{\"publickey\":\"0x{}\",\"signature\":\"0x{}\"}}}}",
        address.to_json(),
        auth.cells.join(","),
        proof.public_key,
        proof.signature
    );
    let mut action = protocol::action_std::TexCellExecute::default();
    action.from_json(&raw).map_err(SdkError::from)?;
    Ok(format!(
        "{{\"schema\":\"hacash.tex/signed-bundle@1\",\"authorization\":{},\"digest\":{},\"action\":\"0x{}\"}}",
        auth.to_json_string(),
        field::json_escape(&request.digest),
        hex::encode(action.encode())
    ))
}

pub fn verify_action(action_hex: &str) -> Result<String, SdkError> {
    let wire =
        hex::decode(action_hex.strip_prefix("0x").unwrap_or(action_hex)).map_err(|error| {
            jsonparse::parse_failed(format!("signed TEX action is not hex: {error}"))
        })?;
    let (action, used) =
        protocol::action_std::TexCellExecute::decode(&wire).map_err(SdkError::from)?;
    if action.kind() != 22 {
        return Err(jsonparse::parse_failed("signed action is not TEX kind 22"));
    }
    if used != wire.len() {
        return Err(jsonparse::parse_failed(
            "trailing bytes after signed TEX action",
        ));
    }
    if action.encode() != wire {
        return Err(jsonparse::parse_failed(
            "signed TEX action is not canonically encoded",
        ));
    }
    let action_json = action.to_json();
    let fields = jsonparse::object_pairs(&action_json, "signed TEX action")?;
    let address = jsonparse::string_value(
        jsonparse::required(&fields, "addr", "signed TEX action")?,
        "addr",
        "signed TEX action",
    )?;
    let cells = jsonparse::required(&fields, "cells", "signed TEX action")?;
    let sign = jsonparse::object_pairs(
        jsonparse::required(&fields, "sign", "signed TEX action")?,
        "TEX sign",
    )?;
    let public_key = jsonparse::string_value(
        jsonparse::required(&sign, "publickey", "TEX sign")?,
        "publickey",
        "TEX sign",
    )?;
    let signature = jsonparse::string_value(
        jsonparse::required(&sign, "signature", "TEX sign")?,
        "signature",
        "TEX sign",
    )?;
    let auth = decode_authorization_json(&format!(
        "{{\"schema\":{},\"address\":{},\"cells\":{cells}}}",
        field::json_escape(SCHEMA_TEX_AUTHORIZATION),
        field::json_escape(&address)
    ))?;
    let pk: [u8; 33] = crate::inspect::decode_hex_fixed(
        public_key.trim_start_matches("0x"),
        SdkErrorCode::InvalidPublicKey,
        "TEX public key must be 33 bytes",
    )?;
    let sig: [u8; 64] = crate::inspect::decode_hex_fixed(
        signature.trim_start_matches("0x"),
        SdkErrorCode::BadSignature,
        "TEX signature must be 64 bytes",
    )?;
    let derived = field::Address::from(sys::Account::get_address_by_public_key(pk));
    if derived.to_readable() != auth.address {
        return Err(SdkError::new(
            SdkErrorCode::BadSignature,
            "TEX public key does not match authorization address",
        ));
    }
    let digest = action.sign_hash().0;
    if !sys::Account::verify_signature(&digest, &pk, &sig) {
        return Err(SdkError::new(
            SdkErrorCode::BadSignature,
            "TEX action signature is invalid",
        ));
    }
    Ok(format!("{{\"schema\":\"hacash.tex/verify-result@1\",\"ok\":true,\"address\":{},\"digest\":{},\"authorization\":{}}}",
        field::json_escape(&auth.address), field::json_escape(&hex::encode(digest)), auth.to_json_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::SdkJsonFrom;
    use field::{Address, Fold64, ToJSON};

    #[test]
    fn protocol_digest_and_external_signature_roundtrip() {
        let account = sys::Account::create_by_password("sdk tex fixture").unwrap();
        let addr = Address::from(*account.address());
        let cell = protocol::action_std::TexCell::ZhuPay {
            haczhu: Fold64::from(123456).unwrap(),
        };
        let chain = protocol::action_std::TexCell::CondChainIdEq {
            chainid: field::Uint4::from(0),
        };
        let action = protocol::action_std::TexCellExecute::create_signed(
            addr,
            vec![cell.clone(), chain.clone()],
            &account,
        )
        .unwrap();
        let raw = format!(
            "{{\"schema\":{},\"address\":{},\"cells\":[{},{}]}}",
            field::json_escape(SCHEMA_TEX_AUTHORIZATION),
            addr.to_json(),
            cell.to_json(),
            chain.to_json()
        );
        let auth = decode_authorization_json(&raw).unwrap();
        assert_eq!(
            authorization_digest(&auth).unwrap(),
            hex::encode(action.sign_hash().0)
        );

        let request: crate::attach::SigningRequest = crate::attach::SigningRequest::from_json_str(
            &prepare_signature(&auth, &addr.to_readable(), None, None).unwrap(),
        )
        .unwrap();
        let signature = account.do_sign(&action.sign_hash().0);
        let proof = crate::attach::SignatureProof {
            schema: crate::schema::SCHEMA_SIGNATURE_PROOF.to_owned(),
            request_id: request.id.clone(),
            request_binding: request.request_binding.clone(),
            public_key: hex::encode(account.public_key().serialize_compressed()),
            signature: hex::encode(signature),
            algorithm: request.algorithm.clone(),
        };
        let signed = attach_signature(&auth, &proof, &request).unwrap();
        let signed_json = jsonparse::object_pairs(&signed, "signed bundle").unwrap();
        let action_hex = jsonparse::string_value(
            jsonparse::required(&signed_json, "action", "signed bundle").unwrap(),
            "action",
            "signed bundle",
        )
        .unwrap();
        assert_eq!(
            hex::decode(action_hex.trim_start_matches("0x")).unwrap(),
            action.encode()
        );
        assert!(verify_action(&action_hex).unwrap().contains("\"ok\":true"));
        let mut wrong_kind = action.encode();
        wrong_kind[1] = 21;
        assert!(verify_action(&hex::encode(wrong_kind)).is_err());
        assert!(verify_action(&action_hex[0..action_hex.len() - 2]).is_err());
        assert!(verify_action(&format!("{action_hex}00")).is_err());

        let mut changed_auth = auth.clone();
        changed_auth.cells[0] = changed_auth.cells[0].replace("123456", "123457");
        assert!(attach_signature(&changed_auth, &proof, &request).is_err());

        let mut changed_request = request.clone();
        changed_request.origin = Some("changed origin".to_owned());
        assert!(attach_signature(&auth, &proof, &changed_request).is_err());

        let mut bad_proof = proof.clone();
        bad_proof.signature.replace_range(
            0..2,
            if &bad_proof.signature[0..2] == "00" {
                "01"
            } else {
                "00"
            },
        );
        assert!(attach_signature(&auth, &bad_proof, &request).is_err());

        let other = sys::Account::create_by_password("wrong tex signer").unwrap();
        let mut wrong_signer = proof.clone();
        wrong_signer.public_key = hex::encode(other.public_key().serialize_compressed());
        wrong_signer.signature = hex::encode(other.do_sign(&action.sign_hash().0));
        assert!(attach_signature(&auth, &wrong_signer, &request).is_err());

        let expired: crate::attach::SigningRequest = crate::attach::SigningRequest::from_json_str(
            &prepare_signature(&auth, &addr.to_readable(), None, Some(1)).unwrap(),
        )
        .unwrap();
        let expired_proof = crate::attach::SignatureProof {
            request_id: expired.id.clone(),
            request_binding: expired.request_binding.clone(),
            ..proof.clone()
        };
        assert!(attach_signature(&auth, &expired_proof, &expired).is_err());
        // The on-chain bundle remains verifiable after the wallet request expires.
        assert!(verify_action(&action_hex).unwrap().contains("\"ok\":true"));
    }

    #[test]
    fn authorization_rejects_unknown_structure_and_preserves_large_values() {
        let address = Address::default().to_readable();
        let raw = format!(
            r#"{{"schema":"{SCHEMA_TEX_AUTHORIZATION}","address":"{address}","cells":[{{"cellid":24,"height":"1099511627775"}},{{"cellid":25,"chainid":4294967295}}]}}"#
        );
        let auth = decode_authorization_json(&raw).unwrap();
        assert!(auth.cells[0].contains("1099511627775"));
        let review = inspect(&auth).unwrap();
        assert_eq!(review.cells.len(), 2);
        assert_eq!(
            review.height_conditions,
            vec![GuardCondition {
                operator: "at_least",
                value: "1099511627775".to_owned()
            }]
        );
        assert_eq!(
            review.chain_id_condition,
            vec![GuardCondition {
                operator: "eq",
                value: "4294967295".to_owned()
            }]
        );
        let unknown = raw.replace("\"cells\":", "\"surprise\":1,\"cells\":");
        assert!(decode_authorization_json(&unknown).is_err());
        let unknown_cell = format!(
            r#"{{"schema":"{SCHEMA_TEX_AUTHORIZATION}","address":"{address}","cells":[{{"cellid":99,"value":1}}]}}"#
        );
        assert!(decode_authorization_json(&unknown_cell).is_err());
    }

    #[test]
    fn large_asset_integers_are_strings_in_every_review_and_roundtrip_digest() {
        let address = Address::default().to_readable();
        let n = "9007199254740993";
        let raw = format!(
            r#"{{"schema":"{SCHEMA_TEX_AUTHORIZATION}","address":"{address}","cells":[{{"cellid":7,"asset":{{"serial":"{n}","amount":"{n}"}}}}]}}"#
        );
        let auth = decode_authorization_json(&raw).unwrap();
        assert!(
            auth.cells[0].contains(&format!("\"serial\":\"{n}\"")),
            "{}",
            auth.cells[0]
        );
        assert!(
            auth.cells[0].contains(&format!("\"amount\":\"{n}\"")),
            "{}",
            auth.cells[0]
        );
        let inspection = inspect(&auth).unwrap().to_json_string();
        assert!(
            inspection.contains(&format!("\"serial\":\"{n}\"")),
            "{inspection}"
        );
        assert!(
            inspection.contains(&format!("\"amount\":\"{n}\"")),
            "{inspection}"
        );
        let digest = authorization_digest(&auth).unwrap();
        let roundtripped = decode_authorization_json(&auth.to_json_string()).unwrap();
        assert_eq!(authorization_digest(&roundtripped).unwrap(), digest);
    }

    #[test]
    fn every_tex_cell_serializes_precision_sensitive_values_as_strings() {
        let address = Address::default().to_readable();
        let huge = "9007199254740993";
        let raw = format!(
            r#"{{"schema":"{SCHEMA_TEX_AUTHORIZATION}","address":"{address}","cells":[
                {{"cellid":1,"haczhu":{huge}}},{{"cellid":2,"haczhu":{huge}}},
                {{"cellid":3,"satnum":{huge}}},{{"cellid":4,"satnum":{huge}}},
                {{"cellid":5,"diamonds":["WTYUIA"]}},{{"cellid":6,"dianum":16777215}},
                {{"cellid":7,"asset":{{"serial":{huge},"amount":{huge}}}}},
                {{"cellid":8,"asset":{{"serial":{huge},"amount":{huge}}}}},
                {{"cellid":11,"haczhu":{huge}}},{{"cellid":12,"haczhu":{huge}}},{{"cellid":13,"haczhu":{huge}}},
                {{"cellid":14,"satoshi":{huge}}},{{"cellid":15,"satoshi":{huge}}},{{"cellid":16,"satoshi":{huge}}},
                {{"cellid":17,"diamond":{huge}}},{{"cellid":18,"diamond":{huge}}},{{"cellid":19,"diamond":{huge}}},
                {{"cellid":20,"asset":{{"serial":{huge},"amount":{huge}}}}},
                {{"cellid":21,"asset":{{"serial":{huge},"amount":{huge}}}}},
                {{"cellid":22,"asset":{{"serial":{huge},"amount":{huge}}}}},
                {{"cellid":23,"height":1099511627775}},{{"cellid":24,"height":1099511627775}},
                {{"cellid":25,"chainid":4294967295}}
            ]}}"#
        );
        let auth = decode_authorization_json(&raw).unwrap();
        assert_eq!(auth.cells.len(), 23);
        for cell in &auth.cells {
            let pairs = jsonparse::object_pairs(cell, "cell").unwrap();
            assert!(pairs.iter().any(|(key, _)| *key == "cellid"));
            assert!(!cell.contains(":9007199254740993"), "{cell}");
            assert!(!cell.contains(":1099511627775"), "{cell}");
            assert!(!cell.contains(":4294967295"), "{cell}");
        }
        assert!(auth.cells[0].contains("\"haczhu\":\"9007199254740993\""));
        assert!(auth.cells[4].contains("WTYUIA"), "{}", auth.cells[4]);
        assert!(auth.cells[5].contains("\"dianum\":\"16777215\""));
        assert!(auth.cells[6].contains("\"serial\":\"9007199254740993\""));
        assert!(auth.cells[20].contains("\"height\":\"1099511627775\""));
        assert!(auth.cells[22].contains("\"chainid\":\"4294967295\""));
        let roundtrip = decode_authorization_json(&auth.to_json_string()).unwrap();
        assert_eq!(
            authorization_digest(&auth).unwrap(),
            authorization_digest(&roundtrip).unwrap()
        );
    }
}

fn split_array(raw: &str) -> Result<Vec<&str>, SdkError> {
    let s = raw.trim();
    if !s.starts_with('[') || !s.ends_with(']') {
        return Err(jsonparse::parse_failed(
            "authorization.cells must be an array",
        ));
    }
    let inner = &s[1..s.len() - 1];
    let mut out = Vec::new();
    let (mut start, mut depth, mut quoted, mut escaped) = (0usize, 0i32, false, false);
    for (i, ch) in inner.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let item = inner[start..i].trim();
                if item.is_empty() {
                    return Err(jsonparse::parse_failed(
                        "authorization.cells contains an empty item",
                    ));
                }
                out.push(item);
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        out.push(last);
    } else if !inner.trim().is_empty() {
        return Err(jsonparse::parse_failed(
            "authorization.cells contains an empty item",
        ));
    }
    Ok(out)
}

pub fn decode_authorization_json(raw: &str) -> Result<Authorization, SdkError> {
    let pairs = jsonparse::object_pairs(raw, "TEX authorization")?;
    jsonparse::reject_unknown(&pairs, &["schema", "address", "cells"], "TEX authorization")?;
    let schema = jsonparse::required_string(&pairs, "schema", "TEX authorization")?;
    if schema != SCHEMA_TEX_AUTHORIZATION {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            format!("unsupported TEX authorization schema {schema:?}"),
        ));
    }
    let address = jsonparse::required_string(&pairs, "address", "TEX authorization")?;
    field::Address::from_readable(&address).map_err(SdkError::from)?;
    let raw_cells = jsonparse::required(&pairs, "cells", "TEX authorization")?;
    let cells = split_array(raw_cells)?;
    if cells.is_empty() {
        return Err(jsonparse::parse_failed(
            "TEX authorization needs at least one cell",
        ));
    }
    let mut canonical = Vec::with_capacity(cells.len());
    for (i, raw_cell) in cells.into_iter().enumerate() {
        let mut cell = protocol::action_std::TexCell::default();
        cell.from_json(raw_cell)
            .map_err(|e| SdkError::new(SdkErrorCode::ParseFailed, format!("cells[{i}]: {e}")))?;
        canonical.push(canonical_cell_json(&cell));
    }
    Ok(Authorization {
        schema,
        address,
        cells: canonical,
    })
}

/// Serialize each known cell from its typed value so precision-sensitive
/// integers cross the JSON/JavaScript boundary as decimal strings. Cell IDs
/// remain numbers, matching the protocol JSON schema.
fn canonical_cell_json(cell: &protocol::action_std::TexCell) -> String {
    use protocol::action_std::TexCell as C;
    let integer = |value: u64| field::json_escape(&value.to_string());
    let scalar =
        |id: u8, key: &str, value: u64| format!("{{\"cellid\":{id},\"{key}\":{}}}", integer(value));
    let asset = |id: u8, value: &field::AssetAmt| {
        format!(
            "{{\"cellid\":{id},\"asset\":{{\"serial\":{},\"amount\":{}}}}}",
            integer(value.serial.uint()),
            integer(value.amount.uint())
        )
    };
    match cell {
        C::ZhuPay { haczhu } => scalar(1, "haczhu", haczhu.uint()),
        C::ZhuGet { haczhu } => scalar(2, "haczhu", haczhu.uint()),
        C::SatPay { satnum } => scalar(3, "satnum", satnum.uint()),
        C::SatGet { satnum } => scalar(4, "satnum", satnum.uint()),
        C::DiaPay { diamonds } => format!("{{\"cellid\":5,\"diamonds\":{}}}", diamonds.to_json()),
        C::DiaGet { dianum } => scalar(6, "dianum", dianum.uint() as u64),
        C::AssetPay { asset: value } => asset(7, value),
        C::AssetGet { asset: value } => asset(8, value),
        C::CondZhuAtMost { haczhu } => scalar(11, "haczhu", haczhu.uint()),
        C::CondZhuAtLeast { haczhu } => scalar(12, "haczhu", haczhu.uint()),
        C::CondZhuEq { haczhu } => scalar(13, "haczhu", haczhu.uint()),
        C::CondSatAtMost { satoshi } => scalar(14, "satoshi", satoshi.uint()),
        C::CondSatAtLeast { satoshi } => scalar(15, "satoshi", satoshi.uint()),
        C::CondSatEq { satoshi } => scalar(16, "satoshi", satoshi.uint()),
        C::CondDiaAtMost { diamond } => scalar(17, "diamond", diamond.uint()),
        C::CondDiaAtLeast { diamond } => scalar(18, "diamond", diamond.uint()),
        C::CondDiaEq { diamond } => scalar(19, "diamond", diamond.uint()),
        C::CondAssetAtMost { asset: value } => asset(20, value),
        C::CondAssetAtLeast { asset: value } => asset(21, value),
        C::CondAssetEq { asset: value } => asset(22, value),
        C::CondHeightAtMost { height } => scalar(23, "height", height.uint() as u64),
        C::CondHeightAtLeast { height } => scalar(24, "height", height.uint() as u64),
        C::CondChainIdEq { chainid } => scalar(25, "chainid", chainid.uint() as u64),
    }
}

pub fn inspect(authorization: &Authorization) -> Result<Inspection, SdkError> {
    let mut cells = Vec::with_capacity(authorization.cells.len());
    let mut chain_id_condition = Vec::new();
    let mut height_conditions = Vec::new();
    let mut payments = Vec::new();
    let mut receipts = Vec::new();
    let mut unknown_conditions = Vec::new();
    for (index, raw) in authorization.cells.iter().enumerate() {
        let pairs = jsonparse::object_pairs(raw, "canonical TEX cell")?;
        let cellid: u8 = jsonparse::number_value(
            jsonparse::required(&pairs, "cellid", "TEX cell")?,
            "cellid",
            "TEX cell",
        )?;
        let category = match cellid {
            1 | 3 | 5 | 7 => "pay",
            2 | 4 | 6 | 8 => "get",
            11..=25 => "condition",
            _ => "unknown",
        };
        let value = pairs
            .iter()
            .find(|(k, _)| *k != "cellid")
            .map(|(_, v)| *v)
            .unwrap_or("");
        let display_value = value.trim_matches('"');
        let hac_value = display_value
            .parse::<u64>()
            .map(|zhu| field::Amount::zhu(zhu).to_hac_string())
            .unwrap_or_else(|_| display_value.to_owned());
        let description = match cellid {
            1 => format!(
                "Pay {} HAC ({} zhu) into TEX escrow",
                hac_value, display_value
            ),
            2 => format!(
                "Get {} HAC ({} zhu) from TEX escrow",
                hac_value, display_value
            ),
            3 => format!("Pay {} SAT into TEX escrow", value),
            4 => format!("Get {} SAT from TEX escrow", value),
            5 => format!("Pay HACD {} into TEX escrow", value),
            6 => format!("Get {} HACD from TEX escrow", value),
            7 => format!("Pay asset {} into TEX escrow", value),
            8 => format!("Get asset {} from TEX escrow", value),
            23 => format!("Require block height at most {}", value),
            24 => format!("Require block height at least {}", value),
            25 => format!("Require chain ID equal to {}", value),
            11 => format!(
                "Require HAC balance at most {} HAC ({} zhu)",
                hac_value, display_value
            ),
            12 => format!(
                "Require HAC balance at least {} HAC ({} zhu)",
                hac_value, display_value
            ),
            13 => format!(
                "Require HAC balance equal to {} HAC ({} zhu)",
                hac_value, display_value
            ),
            14 => format!("Require SAT balance at most {}", display_value),
            15 => format!("Require SAT balance at least {}", display_value),
            16 => format!("Require SAT balance equal to {}", display_value),
            17 => format!("Require HACD count at most {}", display_value),
            18 => format!("Require HACD count at least {}", display_value),
            19 => format!("Require HACD count equal to {}", display_value),
            20 => format!("Require asset balance at most {}", value),
            21 => format!("Require asset balance at least {}", value),
            22 => format!("Require asset balance equal to {}", value),
            _ => {
                unknown_conditions.push(index);
                format!("Unknown TEX cell {}", cellid)
            }
        };
        if cellid == 25 {
            chain_id_condition.push(GuardCondition {
                operator: "eq",
                value: display_value.to_owned(),
            });
        }
        if cellid == 23 || cellid == 24 {
            height_conditions.push(GuardCondition {
                operator: if cellid == 23 { "at_most" } else { "at_least" },
                value: display_value.to_owned(),
            });
        }
        let fact = CellReview {
            index,
            cellid,
            cell: raw.clone(),
            category,
            description,
        };
        if category == "pay" {
            payments.push(fact.clone());
        }
        if category == "get" {
            receipts.push(fact.clone());
        }
        cells.push(fact);
    }
    Ok(Inspection {
        authorization: authorization.clone(),
        cells,
        chain_id_condition,
        height_conditions,
        payments,
        receipts,
        unknown_conditions,
    })
}
