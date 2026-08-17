//! `tx.build`: declarative construction of unsigned Type-2/3 bodies from a
//! kind-keyed action spec (Unified SDK 2.0, doc 14 §4.6/§5). New action kinds
//! extend the registry, never the operation shape.

use std::sync::Arc;

use base::TransactionBuild;
use field::{Address, Amount, BytesW1, BytesW2, DiamondNameListMax200, Encode, Satoshi};
use protocol::action_std::{
    AssetFromToTrs, AssetToTrs, ChainAllow, DiaFromToTrs, DiaSingleTrs, DiaToTrs, HacFromToTrs,
    HacToTrs, HeightScope, ReqSignList, SatFromToTrs, SatToTrs, TxBlob, TxMessage,
};
use protocol::tx_std::{TransactionType2, TransactionType3};
use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};
use crate::inspect::decode_tx;
use crate::profile::{MAX_TX_SIZE, TX_ACTIONS_MAX};
use crate::schema::{SCHEMA_BUILT_TRANSACTION, SCHEMA_TRANSACTION_SPEC};

/// One action in a build spec. The `kind` tag is the stable data contract;
/// unknown kinds fail with `UnsupportedSchema` instead of being ignored.
///
/// Transfer actions carry an optional `from` address: when absent the action
/// transfers from the transaction main address (`*ToTrs`); when present and
/// different from `main` the action is a `*FromToTrs` transfer out of that
/// address, which then becomes a required signer.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionSpec {
    HacTransfer {
        #[serde(default)]
        from: Option<String>,
        to: String,
        amount: String,
    },
    SatTransfer {
        #[serde(default)]
        from: Option<String>,
        to: String,
        satoshi: u64,
    },
    HacdTransfer {
        #[serde(default)]
        from: Option<String>,
        to: String,
        names: Vec<String>,
    },
    AssetTransfer {
        #[serde(default)]
        from: Option<String>,
        to: String,
        serial: u64,
        amount: String,
    },
    HeightScope { start: u64, end: u64 },
    ChainAllow { chains: Vec<u32> },
    ReqSignList { signers: Vec<String> },
    TxMessage { data: String },
    TxBlob { data: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionSpec {
    #[serde(default)]
    pub schema: Option<String>,
    pub tx_type: u8,
    pub main: String,
    pub fee: String,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub gas_max: Option<u8>,
    pub actions: Vec<ActionSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltTransaction {
    pub schema: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
    pub hash: String,
    pub hash_with_fee: String,
    pub unsigned_body_hash: String,
    pub body: String,
}

pub fn build_transaction(spec: &TransactionSpec) -> Result<BuiltTransaction, SdkError> {
    if let Some(schema) = &spec.schema {
        if schema != SCHEMA_TRANSACTION_SPEC {
            return Err(SdkError::new(
                SdkErrorCode::UnsupportedSchema,
                format!("unsupported spec schema {schema:?}"),
            ));
        }
    }
    if !matches!(spec.tx_type, 2 | 3) {
        return Err(SdkError::with_detail(
            SdkErrorCode::UnsupportedTxType,
            format!("build supports type 2/3 only, got {}", spec.tx_type),
            serde_json::json!({ "actual": spec.tx_type }),
        ));
    }
    if spec.actions.len() > TX_ACTIONS_MAX {
        return Err(SdkError::with_detail(
            SdkErrorCode::LimitExceeded,
            format!("action count {} exceeds protocol maximum {}", spec.actions.len(), TX_ACTIONS_MAX),
            serde_json::json!({ "expected": TX_ACTIONS_MAX }),
        ));
    }
    if spec.tx_type == 2 && spec.gas_max.is_some_and(|gas| gas != 0) {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "type 2 transactions require gas_max = 0",
        ));
    }
    let main = Address::from_readable(&spec.main).map_err(|error| SdkError::from(error))?;
    let fee = Amount::from(&spec.fee).map_err(|error| SdkError::from(error))?;
    let fee_fin = fee.to_fin_string();
    let timestamp = spec.timestamp.unwrap_or_else(sys::curtimes);

    let mut actions = Vec::with_capacity(spec.actions.len());
    for action in &spec.actions {
        actions.push(build_action(action, &main)?);
    }

    let body = if spec.tx_type == 3 {
        let mut tx = TransactionType3::new_by(main, fee, timestamp);
        if let Some(gas) = spec.gas_max {
            tx.gas_max = field::Uint1::from(gas);
        }
        for action in actions {
            tx.push_action(action)
                .map_err(|error| SdkError::from(error))?;
        }
        tx.encode()
    } else {
        let mut tx = TransactionType2::new_by(main, fee, timestamp);
        for action in actions {
            tx.push_action(action)
                .map_err(|error| SdkError::from(error))?;
        }
        tx.encode()
    };

    if body.len() > MAX_TX_SIZE {
        return Err(SdkError::with_detail(
            SdkErrorCode::LimitExceeded,
            format!("built body {} bytes exceeds protocol maximum {}", body.len(), MAX_TX_SIZE),
            serde_json::json!({ "expected": MAX_TX_SIZE }),
        ));
    }

    // Round-trip invariant: encode(decode(body)) == body must hold.
    let body_hex = hex::encode(&body);
    let decoded = decode_tx(&body)?;
    let re_encoded = decoded.encode();
    if re_encoded != body {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "built body failed the encode(decode(body)) == body round-trip",
        ));
    }
    let unsigned_body_hash = crate::audit::unsigned_body_hash(&body_hex)?;
    Ok(BuiltTransaction {
        schema: SCHEMA_BUILT_TRANSACTION.to_owned(),
        tx_type: spec.tx_type,
        timestamp,
        main: main.to_readable(),
        fee: fee_fin,
        hash: hex::encode(decoded.hash().0),
        hash_with_fee: hex::encode(decoded.hash_with_fee().0),
        unsigned_body_hash,
        body: body_hex,
    })
}

/// Build one action. `main` is the transaction main address: a `from` equal
/// to it collapses to the `*ToTrs` form (same wire bytes as the historical
/// single-signer SDK), a different `from` yields the `*FromToTrs` form.
fn build_action(spec: &ActionSpec, main: &Address) -> Result<base::ActionRef, SdkError> {
    let action: base::ActionRef = match spec {
        ActionSpec::HacTransfer { from, to, amount } => {
            let amount = Amount::from(amount).map_err(|error| SdkError::from(error))?;
            let to_address = parse_address(to)?;
            match parse_from(from, main)? {
                Some(from_address) => Arc::new(HacFromToTrs::new(from_address, to_address, amount)),
                None => Arc::new(HacToTrs::new(to_address, amount)),
            }
        }
        ActionSpec::SatTransfer { from, to, satoshi } => {
            let satoshi = Satoshi::from(*satoshi);
            let to_address = parse_address(to)?;
            match parse_from(from, main)? {
                Some(from_address) => Arc::new(SatFromToTrs::new(from_address, to_address, satoshi)),
                None => Arc::new(SatToTrs::new(to_address, satoshi)),
            }
        }
        ActionSpec::HacdTransfer { from, to, names } => {
            let list = DiamondNameListMax200::from_readable(&names.join(","))
                .map_err(|error| SdkError::from(error))?;
            let to_address = parse_address(to)?;
            match parse_from(from, main)? {
                Some(from_address) => Arc::new(DiaFromToTrs::new(from_address, to_address, list)),
                None if list.length() == 1 => {
                    Arc::new(DiaSingleTrs::new(list.as_list()[0], to_address))
                }
                None => Arc::new(DiaToTrs::new(to_address, list)),
            }
        }
        ActionSpec::AssetTransfer { from, to, serial, amount } => {
            let asset = field::AssetAmt {
                serial: field::Fold64::from(*serial).map_err(SdkError::from)?,
                amount: field::Fold64::from(parse_asset_amount(amount)?)
                    .map_err(SdkError::from)?,
            }
            .checked()
            .map_err(SdkError::from)?;
            let to_address = parse_address(to)?;
            match parse_from(from, main)? {
                Some(from_address) => {
                    Arc::new(AssetFromToTrs::new(from_address, to_address, asset))
                }
                None => Arc::new(AssetToTrs::new(to_address, asset)),
            }
        }
        ActionSpec::HeightScope { start, end } => Arc::new(HeightScope::new(
            field::BlockHeight::from(*start),
            field::BlockHeight::from(*end),
        )),
        ActionSpec::ChainAllow { chains } => {
            let list = field::ChainIDList::from(
                chains
                    .iter()
                    .map(|id| field::Uint4::from(*id))
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| SdkError::from(error))?;
            Arc::new(ChainAllow::new(list))
        }
        ActionSpec::ReqSignList { signers } => {
            let addrs = signers
                .iter()
                .map(|signer| parse_address(signer))
                .collect::<Result<Vec<_>, _>>()?;
            Arc::new(
                ReqSignList::create_by_addrs(addrs).map_err(|error| SdkError::from(error))?,
            )
        }
        ActionSpec::TxMessage { data } => Arc::new(TxMessage::new(
            BytesW1::from(decode_hex_data(data, "tx_message")?)
                .map_err(|error| SdkError::from(error))?,
        )),
        ActionSpec::TxBlob { data } => Arc::new(TxBlob::new(
            BytesW2::from(decode_hex_data(data, "tx_blob")?)
                .map_err(|error| SdkError::from(error))?,
        )),
    };
    Ok(action)
}

fn parse_address(raw: &str) -> Result<Address, SdkError> {
    Address::from_readable(raw).map_err(|error| SdkError::from(error))
}

/// `from` collapses to `None` when absent or equal to the transaction main
/// address (single-signer `*ToTrs` form); a different address stays `Some`.
fn parse_from(from: &Option<String>, main: &Address) -> Result<Option<Address>, SdkError> {
    match from {
        None => Ok(None),
        Some(raw) => {
            let address = parse_address(raw)?;
            if address == *main {
                Ok(None)
            } else {
                Ok(Some(address))
            }
        }
    }
}

fn parse_asset_amount(raw: &str) -> Result<u64, SdkError> {
    raw.parse::<u64>()
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, format!("asset amount {raw:?} invalid")))
}

fn decode_hex_data(data: &str, name: &str) -> Result<Vec<u8>, SdkError> {
    hex::decode(data.trim_start_matches("0x").trim_start_matches("0X"))
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, format!("{name} data must be hex")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn sample_spec() -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 2,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![
                ActionSpec::HacTransfer {
                    from: None,
                    to: MAIN.to_owned(),
                    amount: "12:244".to_owned(),
                },
                ActionSpec::HeightScope {
                    start: 1_000_000,
                    end: 0,
                },
            ],
        }
    }

    #[test]
    fn builds_type2_and_round_trips() {
        let built = build_transaction(&sample_spec()).unwrap();
        assert_eq!(built.tx_type, 2);
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(hex::encode(decoded.encode()), built.body);
        assert_eq!(decoded.action_count(), 2);
    }

    #[test]
    fn rejects_unknown_tx_type() {
        let mut spec = sample_spec();
        spec.tx_type = 1;
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "unsupported_tx_type");
    }

    #[test]
    fn rejects_type2_with_gas_max() {
        let mut spec = sample_spec();
        spec.tx_type = 2;
        spec.gas_max = Some(10);
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "parse_failed");
    }

    #[test]
    fn explicit_from_builds_from_to_transfer_and_becomes_signer() {
        use base::Transaction;
        let other = sys::Account::create_by("654321").unwrap();
        let mut spec = sample_spec();
        spec.actions[0] = ActionSpec::HacTransfer {
            from: Some(other.readable().to_owned()),
            to: MAIN.to_owned(),
            amount: "12:244".to_owned(),
        };
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::HacFromToTrs::KIND
        );
        // The explicit from address becomes a required signer.
        let required = decoded.req_sign().unwrap();
        let other_address = field::Address::from_readable(other.readable()).unwrap();
        assert!(required.contains(&other_address));
    }
}
