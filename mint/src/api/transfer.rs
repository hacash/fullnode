//! Hacash-specific transfer-build / transfer-scan API (TransactionType2 + the 13
//! standard transfer actions). Scan dispatches via `base::TransferLike`, so the JSON shape comes from `TransferPayload`.

use base::{
    Action, AddrOrPtr, ApiExecCtx, ApiRequest, ApiResponse, BlkPkg, Transaction, TransactionSign,
    TransferPayload,
};
use field::{Address, Amount, Decode, DiamondName, DiamondNameListMax200, Encode, Satoshi};
use protocol::action_std::{
    DiaFromToTrs, DiaSingleTrs, DiaToTrs, HacFromToTrs, HacToTrs, SatFromToTrs, SatToTrs,
};
use protocol::tx_std::TransactionType2;
use sys::ToHex;

use super::util::{api_error, diamond_names_readable, json_string, q_string};

// =============================================================
// CoinKind
// =============================================================

#[derive(Clone)]
pub(crate) struct CoinKind {
    hacash: bool,
    satoshi: bool,
    diamond: bool,
    assets_all: bool,
    assets: Vec<u64>,
}

impl CoinKind {
    pub(crate) fn parse(raw: &str) -> sys::Ret<Self> {
        let compact = raw.to_ascii_lowercase().replace([' ', ',', ';', '|'], "");
        if compact.is_empty() || compact == "all" || compact == "hsda" {
            return Ok(Self {
                hacash: true,
                satoshi: true,
                diamond: true,
                assets_all: true,
                assets: Vec::new(),
            });
        }
        let (kind_part, asset_part) = if let Some(start) = compact.find('(') {
            let Some(end) = compact.rfind(')') else {
                return sys::errf!("coinkind assets list format invalid");
            };
            (&compact[..start], Some(&compact[start + 1..end]))
        } else {
            (compact.as_str(), None)
        };
        if !kind_part
            .chars()
            .all(|c| c == 'h' || c == 's' || c == 'd' || c == 'a')
        {
            return sys::errf!("coinkind format invalid");
        }
        let mut assets = Vec::new();
        if let Some(asset_part) = asset_part {
            if !kind_part.contains('a') {
                return sys::errf!("coinkind assets list requires 'a'");
            }
            for item in asset_part.split(',').filter(|s| !s.is_empty()) {
                assets.push(item.parse::<u64>().map_err(|_| {
                    sys::Error::fault(format!("asset serial {} format invalid", item))
                })?);
            }
        }
        Ok(Self {
            hacash: kind_part.contains('h'),
            satoshi: kind_part.contains('s'),
            diamond: kind_part.contains('d'),
            assets_all: kind_part.contains('a') && assets.is_empty(),
            assets,
        })
    }
}

// =============================================================
// shared helpers (moved from api util.rs)
// =============================================================

pub(crate) fn load_block_by_height(ctx: &ApiExecCtx, height: u64) -> sys::Ret<BlkPkg> {
    let Some((_hash, data)) = ctx.engine.store().block_data_by_height(height)? else {
        return sys::errf!("block not found");
    };
    BlkPkg::from_bytes(
        ctx.engine.services().as_ref(),
        data.to_vec(),
        base::PkgSource::new(base::PkgOrigin::Api),
    )
    .map_err(|e| sys::Error::fault(format!("block parse failed: {}", e)))
}

fn real_addr(ptr: &AddrOrPtr, addrs: &[Address]) -> sys::Ret<Address> {
    ptr.real(addrs)
}

// =============================================================
// transfer_json -- dispatched via TransferLike, no downcast_ref
// =============================================================

/// Build the JSON for a single transfer action, or `None` if not a transfer / filtered out.
/// All values derive from `TransferPayload` (no `downcast_ref`); `from` resolves via `transfer_from()`, falling back to the tx main address.
fn transfer_json(
    tx: &dyn Transaction,
    act: &dyn Action,
    unit: &str,
    ck: &CoinKind,
    from_filter: Option<&str>,
    to_filter: Option<&str>,
) -> Option<String> {
    let t = act.as_transfer_like()?;
    let addrs = tx.addrs();
    let main = tx.main();

    let mut fields = vec![format!("\"kind\":{}", act.kind())];

    let from = match t.transfer_from() {
        Some(ptr) => real_addr(&ptr, &addrs).ok()?,
        None => main,
    };
    let to = match t.transfer_to_ptr() {
        Some(ptr) => real_addr(&ptr, &addrs).ok()?,
        None => main,
    };

    match t.transfer_payload() {
        TransferPayload::Hac { amount } => {
            if !ck.hacash {
                return None;
            }
            // `amount` is the wire encoding of the Amount (unit + dist + bytes);
            // decode it back to recover the unit for `to_unit_string`.
            let amt = match Amount::decode(&amount) {
                Ok((a, _)) => a,
                Err(_) => Amount::zero(),
            };
            fields.push(format!(
                "\"hacash\":{}",
                json_string(&amt.to_unit_string(unit))
            ));
        }
        TransferPayload::Sat { satoshi } => {
            if !ck.satoshi {
                return None;
            }
            fields.push(format!("\"satoshi\":{}", satoshi));
        }
        TransferPayload::Hacd { count, names } => {
            if !ck.diamond {
                return None;
            }
            fields.push(format!("\"diamond\":{}", count));
            // Prefer the payload's raw name bytes for the readable list.
            fields.push(format!(
                "\"diamonds\":{}",
                json_string(&diamond_names_readable(&names))
            ));
        }
        TransferPayload::Asset { serial, amount } => {
            if !(ck.assets_all || ck.assets.contains(&serial)) {
                return None;
            }
            fields.push(format!(
                "\"asset\":{{\"serial\":{},\"amount\":{}}}",
                serial, amount
            ));
        }
    }

    let from_readable = from.to_readable();
    let to_readable = to.to_readable();
    if from_filter.is_some_and(|v| v != from_readable) {
        return None;
    }
    if to_filter.is_some_and(|v| v != to_readable) {
        return None;
    }
    fields.push(format!("\"from\":{}", json_string(&from_readable)));
    fields.push(format!("\"to\":{}", json_string(&to_readable)));
    Some(format!("{{{}}}", fields.join(",")))
}

// =============================================================
// /query/coin/transfer (scan a tx's transfers)
// =============================================================

pub(crate) fn scan_coin_transfer_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let height = req.query_u64("height").unwrap_or(1);
    let txposi = req
        .query("txposi")
        .and_then(|v| v.parse::<isize>().ok())
        .unwrap_or(-1);
    if txposi < 0 {
        return api_error("txposi error");
    }
    let unit = q_string(&req, "unit", "fin");
    let ck = match CoinKind::parse(&q_string(&req, "coinkind", "hsda")) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let pkg = match load_block_by_height(ctx, height) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let block = pkg.block();
    let txs = block.transactions();
    if txs.is_empty() {
        return api_error("transaction length invalid");
    }
    let normal_txs = &txs[1..];
    let idx = txposi as usize;
    if idx >= normal_txs.len() {
        return api_error("txposi overflow");
    }
    let tx = normal_txs[idx].as_ref();
    let from_filter = req.query("from").or_else(|| req.query("filter_from"));
    let to_filter = req.query("to").or_else(|| req.query("filter_to"));
    let transfers = tx
        .actions()
        .iter()
        .filter_map(|act| transfer_json(tx, act.as_ref(), &unit, &ck, from_filter, to_filter))
        .collect::<Vec<_>>()
        .join(",");
    ApiResponse::json(format!(
        concat!(
            "{{\"ret\":0,",
            "\"tx_hash\":{},",
            "\"tx_timestamp\":{},",
            "\"block_hash\":{},",
            "\"block_timestamp\":{},",
            "\"main_address\":{},",
            "\"transfers\":[{}]}}"
        ),
        json_string(&tx.hash().as_ref().to_hex()),
        tx.timestamp().value(),
        json_string(&block.hash().as_ref().to_hex()),
        block.timestamp(),
        json_string(&tx.main().to_readable()),
        transfers,
    ))
}

// =============================================================
// /create/coin/transfer (build a signed transfer tx)
// =============================================================

pub(crate) fn create_coin_transfer_handler(_ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let fee = q_string(&req, "fee", "");
    let main_prikey = q_string(&req, "main_prikey", "");
    let from_prikey = q_string(&req, "from_prikey", "");
    let to_address = q_string(&req, "to_address", "");
    let hacash = q_string(&req, "hacash", "");
    let diamonds = q_string(&req, "diamonds", "");
    let satoshi = req.query_u64("satoshi").unwrap_or(0);
    let timestamp = req.query_u64("timestamp").unwrap_or_else(sys::curtimes);

    let to_addr = match Address::from_readable(&to_address) {
        Ok(v) => v,
        Err(e) => return api_error(&format!("address {} format invalid: {}", to_address, e)),
    };
    let fee = match Amount::from(&fee) {
        Ok(v) => v,
        Err(e) => return api_error(&format!("amount {} format invalid: {}", fee, e)),
    };
    let main_acc = match sys::Account::create_by(&main_prikey) {
        Ok(v) => v,
        Err(e) => return api_error(&format!("private key invalid: {}", e)),
    };
    let from_acc = if from_prikey.is_empty() {
        main_acc.clone()
    } else {
        match sys::Account::create_by(&from_prikey) {
            Ok(v) => v,
            Err(e) => return api_error(&format!("private key invalid: {}", e)),
        }
    };
    let is_from = from_acc != main_acc;
    let main_addr = Address::from(*main_acc.address());
    let from_addr = Address::from(*from_acc.address());
    let mut tx = TransactionType2::new_by(main_addr, fee, timestamp);

    if satoshi > 0 {
        let sat = Satoshi::from(satoshi);
        if is_from {
            tx.push_action_in(std::sync::Arc::new(SatFromToTrs::new(
                from_addr, to_addr, sat,
            )));
        } else {
            tx.push_action_in(std::sync::Arc::new(SatToTrs::new(to_addr, sat)));
        }
    }

    if diamonds.len() >= DiamondName::SIZE {
        let list = match DiamondNameListMax200::from_readable(&diamonds) {
            Ok(v) => v,
            Err(e) => return api_error(&format!("diamonds invalid: {}", e)),
        };
        if is_from {
            tx.push_action_in(std::sync::Arc::new(DiaFromToTrs::new(
                from_addr, to_addr, list,
            )));
        } else if list.length() == 1 {
            tx.push_action_in(std::sync::Arc::new(DiaSingleTrs::new(
                list.as_list()[0],
                to_addr,
            )));
        } else {
            tx.push_action_in(std::sync::Arc::new(DiaToTrs::new(to_addr, list)));
        }
    }

    if !hacash.is_empty() {
        let amount = match Amount::from(&hacash) {
            Ok(v) => v,
            Err(e) => return api_error(&format!("hacash amount {} invalid: {}", hacash, e)),
        };
        if is_from {
            tx.push_action_in(std::sync::Arc::new(HacFromToTrs::new(
                from_addr, to_addr, amount,
            )));
        } else {
            tx.push_action_in(std::sync::Arc::new(HacToTrs::new(to_addr, amount)));
        }
    }

    if let Err(e) = tx.fill_sign_account(&main_acc) {
        return api_error(&format!("fill main sign failed: {}", e));
    }
    if is_from {
        if let Err(e) = tx.fill_sign_account(&from_acc) {
            return api_error(&format!("fill from sign failed: {}", e));
        }
    }

    ApiResponse::json(format!(
        "{{\"ret\":0,\"hash\":{},\"hash_with_fee\":{},\"timestamp\":{},\"body\":{}}}",
        json_string(&tx.hash().as_ref().to_hex()),
        json_string(&tx.hash_with_fee().as_ref().to_hex()),
        tx.timestamp.value(),
        json_string(&tx.encode().to_hex()),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn transfer_json_keeps_readable_endpoints_and_payload_shape() {
        let from = Address::from([
            0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let to = Address::from([
            0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let mut tx = TransactionType2::new_by(from, Amount::zero(), 1);
        tx.addrlist = field::AddrOrList::from_list(vec![from, to]).expect("address list");
        let action: Arc<dyn Action> = Arc::new(SatFromToTrs {
            kind: field::Uint2::from(SatFromToTrs::KIND),
            from: AddrOrPtr::Ptr(0),
            to: AddrOrPtr::Ptr(1),
            satoshi: Satoshi::from(7),
        });

        let json = transfer_json(
            &tx,
            action.as_ref(),
            "fin",
            &CoinKind::parse("s").expect("coin kind"),
            None,
            None,
        )
        .expect("transfer JSON");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(value["kind"], SatFromToTrs::KIND);
        assert_eq!(value["satoshi"], 7);
        assert_eq!(value["from"], from.to_readable());
        assert_eq!(value["to"], to.to_readable());
    }
}
