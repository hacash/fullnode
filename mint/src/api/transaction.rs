//! Transaction build / check / sign APIs (ported from fullnodedev mint api).

use std::collections::HashMap;

use base::{ActionRef, ApiExecCtx, ApiRequest, ApiResponse, Transaction, TransactionBuild, TxPkg};
use field::{
    Address, Amount, Encode, Hash, Sign, Uint1, json_decode_array, json_decode_object,
    json_expect_quoted_decoded, json_expect_unquoted,
};
use protocol::tx_std::{TransactionType2, TransactionType3};
use sys::ToHex;

use crate::action::diamond_insc::DiaInscPush;
use crate::api::util::*;

fn create_transaction_error_response(
    code: &str,
    message: &str,
    stage: &str,
    details: &[(&str, String)],
) -> ApiResponse {
    let mut fields = vec![
        "\"ret\":1".to_owned(),
        format!("\"err\":{}", json_string(message)),
        format!("\"error\":{}", json_string(message)),
        format!("\"code\":{}", json_string(code)),
        format!("\"message\":{}", json_string(message)),
        format!("\"stage\":{}", json_string(stage)),
    ];
    for (k, v) in details {
        fields.push(format!("\"{}\":{}", k, v));
    }
    ApiResponse::json(format!("{{{}}}", fields.join(",")))
}

fn parse_addr_value(v: &str) -> sys::Ret<Address> {
    let s = json_expect_quoted_decoded(v)?;
    Address::from_readable(&s)
}

fn parse_amount_value(v: &str) -> sys::Ret<Amount> {
    Amount::from(&json_expect_quoted_decoded(v)?)
}

fn parse_hex_bytes(v: &str) -> sys::Ret<Vec<u8>> {
    let raw = json_expect_quoted_decoded(v)?;
    let trimmed = raw.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    hex::decode(hex).map_err(|e| sys::Error::fault(e.to_string()))
}

fn action_from_json_obj(
    reg: &dyn base::ExecutionServices,
    json: &str,
    obj: &HashMap<String, String>,
) -> sys::Ret<ActionRef> {
    if let Some(body) = obj.get("body") {
        let action = reg.decode_action_exact(&parse_hex_bytes(body)?)?;
        if let Some(kind) = obj.get("kind") {
            let kind: u16 = json_expect_unquoted(kind)?
                .parse()
                .map_err(|_| sys::Error::fault("kind format invalid"))?;
            if kind != action.kind() {
                return sys::errf!(
                    "action body kind {} does not match declared kind {}",
                    action.kind(),
                    kind
                );
            }
        }
        return Ok(action);
    }
    let kind_s = obj
        .get("kind")
        .map(String::as_str)
        .ok_or_else(|| sys::Error::fault("missing required field(s): kind"))?;
    let kind: u16 = json_expect_unquoted(kind_s)?
        .parse()
        .map_err(|_| sys::Error::fault("kind format invalid"))?;
    reg.decode_action_json(kind, json)?.ok_or_else(|| {
        sys::Error::fault(format!(
            "action kind {} not supported by create/transaction subset (Type3/VM/AST stubbed)",
            kind
        ))
    })
}

pub(crate) fn reject_non_canonical_dia_insc_push(tx: &dyn Transaction) -> Option<ApiResponse> {
    for act in tx.actions() {
        if let Some(a) = act.as_any().downcast_ref::<DiaInscPush>() {
            if let Err(e) = a.protocol_cost.require_canonical_wire() {
                return Some(api_error(&format!(
                    "DiaInscPush protocol_cost must use canonical amount encoding: {}",
                    e
                )));
            }
        }
    }
    None
}

enum OwnedTx {
    Type2(TransactionType2),
    Type3(TransactionType3),
}

impl OwnedTx {
    fn as_build(&mut self) -> &mut dyn TransactionBuild {
        match self {
            Self::Type2(tx) => tx,
            Self::Type3(tx) => tx,
        }
    }

    fn as_tx(&self) -> &dyn Transaction {
        match self {
            Self::Type2(tx) => tx,
            Self::Type3(tx) => tx,
        }
    }

    fn fill_sign_account(&mut self, acc: &sys::Account) -> sys::Ret<Sign> {
        match self {
            Self::Type2(tx) => tx.fill_sign_account(acc),
            Self::Type3(tx) => tx.fill_sign_account(acc),
        }
    }

    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Type2(tx) => tx.encode(),
            Self::Type3(tx) => tx.encode(),
        }
    }

    fn from_bytes(reg: &dyn base::BinaryCodecs, data: &[u8]) -> sys::Ret<Self> {
        let tx = reg.decode_transaction_exact(data)?;
        match tx.ty() {
            v if v == TransactionType2::TYPE => {
                let owned = tx
                    .as_any()
                    .downcast_ref::<TransactionType2>()
                    .ok_or_else(|| sys::Error::fault("transaction type2 downcast failed"))?
                    .clone();
                Ok(Self::Type2(owned))
            }
            v if v == TransactionType3::TYPE => {
                let owned = tx
                    .as_any()
                    .downcast_ref::<TransactionType3>()
                    .ok_or_else(|| sys::Error::fault("transaction type3 downcast failed"))?
                    .clone();
                Ok(Self::Type3(owned))
            }
            other => sys::errf!("unsupported transaction type {}", other),
        }
    }
}

pub(crate) fn transaction_build_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let action = q_bool(&req, "action", false);
    let signature = q_bool(&req, "signature", false);
    let description = q_bool(&req, "description", false);

    let Ok(txjsondts) = body_data_may_hex(&req) else {
        return create_transaction_error_response(
            "create_transaction_invalid_json_body",
            "transaction JSON body invalid",
            "parse_body",
            &[],
        );
    };
    let Ok(jsonstr) = std::str::from_utf8(&txjsondts) else {
        return create_transaction_error_response(
            "create_transaction_invalid_json_body",
            "transaction JSON body invalid",
            "parse_body",
            &[],
        );
    };
    let Ok(root) = json_decode_object(jsonstr) else {
        return create_transaction_error_response(
            "create_transaction_invalid_json_body",
            "transaction JSON body invalid",
            "parse_body",
            &[],
        );
    };

    let Some(main_raw) = root.get("main_address") else {
        return create_transaction_error_response(
            "create_transaction_invalid_main_address",
            "main_address format invalid",
            "parse_main_address",
            &[("field", json_string("main_address"))],
        );
    };
    let Ok(main_addr) = parse_addr_value(main_raw) else {
        return create_transaction_error_response(
            "create_transaction_invalid_main_address",
            "main_address format invalid",
            "parse_main_address",
            &[("field", json_string("main_address"))],
        );
    };
    let Some(fee_raw) = root.get("fee") else {
        return create_transaction_error_response(
            "create_transaction_invalid_fee",
            "fee format invalid",
            "parse_fee",
            &[("field", json_string("fee"))],
        );
    };
    let Ok(fee) = parse_amount_value(fee_raw) else {
        return create_transaction_error_response(
            "create_transaction_invalid_fee",
            "fee format invalid",
            "parse_fee",
            &[("field", json_string("fee"))],
        );
    };

    let tx_type = root
        .get("tx_type")
        .or_else(|| root.get("type"))
        .and_then(|v| json_expect_unquoted(v).ok()?.parse::<u64>().ok())
        .unwrap_or(TransactionType2::TYPE as u64);
    let timestamp = root
        .get("timestamp")
        .and_then(|v| json_expect_unquoted(v).ok()?.parse::<u64>().ok())
        .unwrap_or_else(sys::curtimes);

    let mut owned = match tx_type {
        v if v == TransactionType2::TYPE as u64 => {
            OwnedTx::Type2(TransactionType2::new_by(main_addr, fee, timestamp))
        }
        v if v == TransactionType3::TYPE as u64 => {
            let gas_max = root
                .get("gas_max")
                .and_then(|v| json_expect_unquoted(v).ok()?.parse::<u64>().ok())
                .unwrap_or(0);
            if gas_max > base::TX_GAS_BUDGET_CAP_BYTE as u64 {
                return create_transaction_error_response(
                    "create_transaction_invalid_gas_max",
                    "gas_max exceeds the current Type3 cap",
                    "parse_gas_max",
                    &[
                        ("field", json_string("gas_max")),
                        ("max", base::TX_GAS_BUDGET_CAP_BYTE.to_string()),
                    ],
                );
            }
            let mut tx = TransactionType3::new_by(main_addr, fee, timestamp);
            tx.gas_max = Uint1::from(gas_max as u8);
            OwnedTx::Type3(tx)
        }
        _ => {
            return create_transaction_error_response(
                "create_transaction_invalid_type",
                "transaction type must be 2 or 3",
                "parse_type",
                &[("field", json_string("tx_type"))],
            );
        }
    };

    let Some(acts_raw) = root.get("actions") else {
        return create_transaction_error_response(
            "create_transaction_invalid_actions",
            "actions array format invalid",
            "parse_actions",
            &[("field", json_string("actions"))],
        );
    };
    let Ok((acts, _)) = json_decode_array(acts_raw) else {
        return create_transaction_error_response(
            "create_transaction_invalid_actions",
            "actions array format invalid",
            "parse_actions",
            &[("field", json_string("actions"))],
        );
    };

    for (action_index, act_raw) in acts.iter().enumerate() {
        let Ok(act_obj) = json_decode_object(act_raw) else {
            return create_transaction_error_response(
                "create_transaction_invalid_action",
                &format!("transaction action[{action_index}] invalid: action json parse failed"),
                "action_decode",
                &[
                    ("action_index", action_index.to_string()),
                    ("cause", json_string("action json parse failed")),
                ],
            );
        };
        let action_kind = act_obj
            .get("kind")
            .and_then(|v| json_expect_unquoted(v).ok()?.parse::<u64>().ok());
        let a = match action_from_json_obj(ctx.engine.services().as_ref(), act_raw, &act_obj) {
            Ok(v) => v,
            Err(e) => {
                let message = match action_kind {
                    Some(kind) => {
                        format!("transaction action[{action_index}] kind {kind} invalid: {e}")
                    }
                    None => format!("transaction action[{action_index}] invalid: {e}"),
                };
                let mut details = vec![
                    ("action_index", action_index.to_string()),
                    ("cause", json_string(&e.to_string())),
                ];
                if let Some(kind) = action_kind {
                    details.push(("action_kind", kind.to_string()));
                }
                return create_transaction_error_response(
                    "create_transaction_invalid_action",
                    &message,
                    "action_decode",
                    &details,
                );
            }
        };
        if let Err(e) = owned.as_build().push_action(a) {
            let message = match action_kind {
                Some(kind) => {
                    format!("transaction action[{action_index}] kind {kind} rejected: {e}")
                }
                None => format!("transaction action[{action_index}] rejected: {e}"),
            };
            let mut details = vec![
                ("action_index", action_index.to_string()),
                ("cause", json_string(&e.to_string())),
            ];
            if let Some(kind) = action_kind {
                details.push(("action_kind", kind.to_string()));
            }
            return create_transaction_error_response(
                "create_transaction_action_rejected",
                &message,
                "action_push",
                &details,
            );
        }
    }

    if reject_non_canonical_dia_insc_push(owned.as_tx()).is_some() {
        return create_transaction_error_response(
            "create_transaction_non_canonical_protocol_cost",
            "DiaInscPush protocol_cost must use canonical amount encoding",
            "validate_protocol_cost_wire",
            &[],
        );
    }

    ApiResponse::json(transaction_basic_json(
        owned.as_tx(),
        None,
        0,
        &unit,
        true,
        action,
        signature,
        description,
        false,
    ))
}

pub(crate) fn transaction_check_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let set_fee = q_string(&req, "set_fee", "");
    let sign_address = q_string(&req, "sign_address", "");
    let body = q_bool(&req, "body", false);
    let signature = q_bool(&req, "signature", false);
    let description = q_bool(&req, "description", false);

    let Ok(txdts) = body_data_may_hex(&req) else {
        return api_error("transaction body invalid");
    };
    let mut owned = match OwnedTx::from_bytes(ctx.engine.services().as_ref(), &txdts) {
        Ok(v) => v,
        Err(_) => return api_error("transaction body invalid"),
    };
    if let Some(resp) = reject_non_canonical_dia_insc_push(owned.as_tx()) {
        return resp;
    }

    if !set_fee.is_empty() {
        let Ok(fee) = Amount::from(&set_fee) else {
            return api_error("fee format invalid");
        };
        owned.as_build().set_fee(fee);
    }

    let tx = owned.as_tx();
    let mut fields = transaction_fields_json(
        tx,
        None,
        0,
        &unit,
        body,
        true,
        signature,
        description,
        false,
    );
    if !sign_address.is_empty() {
        let Ok(addr) = Address::from_readable(&sign_address) else {
            return api_error("sign_address format invalid");
        };
        let sign_hash = if tx.main() == addr {
            tx.hash_with_fee()
        } else {
            tx.hash()
        };
        fields.push_str(&format!(
            ",\"sign_hash\":{}",
            json_string(&sign_hash.as_ref().to_hex())
        ));
    }
    ApiResponse::json(format!("{{\"ret\":0,{}}}", fields))
}

pub(crate) fn transaction_sign_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let prikey = q_string(&req, "prikey", "");
    let pubkey = q_string(&req, "pubkey", "");
    let sigdts = q_string(&req, "sigdts", "");
    let signature = q_bool(&req, "signature", false);
    let description = q_bool(&req, "description", false);
    let lasthei = ctx.engine.latest_height();

    let Ok(txdts) = body_data_may_hex(&req) else {
        return api_error("transaction body invalid");
    };
    let mut owned = match OwnedTx::from_bytes(ctx.engine.services().as_ref(), &txdts) {
        Ok(v) => v,
        Err(_) => return api_error("transaction body invalid"),
    };
    if let Some(resp) = reject_non_canonical_dia_insc_push(owned.as_tx()) {
        return resp;
    }

    let (address, signobj) = if prikey.len() == 64 {
        let Ok(prik) = hex::decode(&prikey) else {
            return api_error("prikey format invalid");
        };
        let Ok(key32): Result<[u8; 32], _> = prik.try_into() else {
            return api_error("prikey data invalid");
        };
        let Ok(acc) = sys::Account::create_by_secret_key_value(key32) else {
            return api_error("prikey data invalid");
        };
        let fres = owned.fill_sign_account(&acc);
        if let Err(e) = fres {
            return api_error(&format!("fill sign failed: {}", e));
        }
        (Address::from(*acc.address()), fres.unwrap())
    } else {
        if pubkey.len() != 33 * 2 || sigdts.len() != 64 * 2 {
            return api_error("pubkey or signature data invalid");
        }
        let Ok(pbk) = hex::decode(&pubkey) else {
            return api_error("pubkey format invalid");
        };
        let Ok(sig) = hex::decode(&sigdts) else {
            return api_error("sigdts format invalid");
        };
        let Ok(pbk): Result<[u8; 33], _> = pbk.try_into() else {
            return api_error("pubkey format invalid");
        };
        let Ok(sig): Result<[u8; 64], _> = sig.try_into() else {
            return api_error("sigdts format invalid");
        };
        let signobj = Sign {
            publickey: pbk,
            signature: sig,
        };
        if let Err(e) = owned.as_build().push_sign(signobj.clone()) {
            return api_error(&format!("fill sign failed: {}", e));
        }
        (
            Address::from(sys::Account::get_address_by_public_key(pbk)),
            signobj,
        )
    };

    let fields = transaction_fields_json(
        owned.as_tx(),
        None,
        lasthei,
        &unit,
        true,
        false,
        signature,
        description,
        false,
    );
    ApiResponse::json(format!(
        "{{\"ret\":0,{},\"sign_data\":{{\"address\":{},\"pubkey\":{},\"sigdts\":{}}}}}",
        fields,
        json_string(&address.to_readable()),
        json_string(&signobj.publickey.to_hex()),
        json_string(&signobj.signature.to_hex()),
    ))
}

pub(crate) fn fee_raise_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let fee_s = q_string(&req, "fee", "");
    let fee_prikey = q_string(&req, "fee_prikey", "");
    let hash = q_string(&req, "hash", "");
    let Ok(fee) = Amount::from(&fee_s) else {
        return api_error("fee format invalid");
    };
    let Ok(acc) = sys::Account::create_by(&fee_prikey) else {
        return api_error("fee_prikey format invalid");
    };

    let bddts = if !hash.is_empty() {
        let Ok(hx) = hex::decode(&hash) else {
            return api_error("hash parse failed");
        };
        if hx.len() != Hash::SIZE {
            return api_error("hash size invalid");
        }
        let mut raw = [0u8; Hash::SIZE];
        raw.copy_from_slice(&hx);
        let txhx = Hash::from(raw);
        let Some(tx) = ctx.node.txpool().find(txhx.as_ref()) else {
            return api_error(&format!("cannot find tx by hash {} in tx pool", hash));
        };
        tx.data().as_ref().to_vec()
    } else {
        let Ok(b) = body_data_may_hex(&req) else {
            return api_error("tx body invalid");
        };
        b
    };

    let mut owned = match OwnedTx::from_bytes(ctx.engine.services().as_ref(), &bddts) {
        Ok(v) => v,
        Err(_) => return api_error("transaction parse failed"),
    };
    if let Some(resp) = reject_non_canonical_dia_insc_push(owned.as_tx()) {
        return resp;
    }

    let old_fee = owned.as_tx().fee().clone();
    if fee < old_fee {
        return api_error(&format!(
            "fee {} cannot be less than previous fee {}",
            fee, old_fee
        ));
    }
    owned.as_build().set_fee(fee.clone());
    if owned.fill_sign_account(&acc).is_err() {
        return api_error("fill sign failed");
    }
    let txhash = owned.as_tx().hash();
    let txhashwf = owned.as_tx().hash_with_fee();
    let body = owned.encode();
    let txpkg = match TxPkg::from_bytes(
        ctx.engine.services().as_ref(),
        body,
        base::PkgSource::new(base::PkgOrigin::Api),
    ) {
        Ok(v) => v,
        Err(e) => return api_error(&format!("transaction package failed: {}", e)),
    };

    if let Err(e) = ctx.node.submit_transaction(&txpkg, true, false) {
        return api_error(&e.to_string());
    }

    api_ok(vec![
        ("hash", json_string(&txhash.as_ref().to_hex())),
        ("hash_with_fee", json_string(&txhashwf.as_ref().to_hex())),
        ("fee", json_string(&fee.to_fin_string())),
        ("tx_body", json_string(&txpkg.tx().encode().to_hex())),
    ])
}
