use std::collections::HashMap;

use base::{ApiExecCtx, ApiRequest, ApiResponse, CoreStateRead};
use sys::Ret;

use crate::api::util::*;

use field::{Address, AssetAmt, AssetSmelt, Balance, Fold64};

pub(crate) fn asset_list_json(assets: &[AssetAmt]) -> String {
    let items = assets
        .iter()
        .map(|item| {
            format!(
                "{{\"serial\":{},\"amount\":{}}}",
                item.serial.uint(),
                item.amount.uint()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{}]", items)
}

fn asset_item_meta_json(item: &AssetAmt, meta: Option<&AssetSmelt>) -> String {
    let serial = item.serial.uint();
    let amount = item.amount.uint();
    let serial_js = json_string(&serial.to_string());
    let amount_js = json_string(&amount.to_string());
    match meta {
        Some(smelt) if smelt.decimal.uint() <= 16 => format!(
            "{{\"serial\":{},\"amount\":{},\"decimal\":{},\"name\":{},\"ticket\":{},\"supply\":{}}}",
            serial_js,
            amount_js,
            smelt.decimal.uint(),
            json_string(&smelt.name.to_readable_or_hex()),
            json_string(&smelt.ticket.to_readable_or_hex()),
            json_string(&smelt.supply.uint().to_string()),
        ),
        _ => format!(
            "{{\"serial\":{},\"amount\":{},\"metadata\":false,\"name\":{},\"decimal\":null}}",
            serial_js,
            amount_js,
            json_string(&format!("Asset #{}", serial)),
        ),
    }
}

pub(crate) fn asset_list_json_meta(
    assets: &[AssetAmt],
    cache: &mut HashMap<u64, Option<AssetSmelt>>,
    mut load: impl FnMut(Fold64) -> Ret<Option<AssetSmelt>>,
) -> Ret<String> {
    let items = assets
        .iter()
        .map(|item| -> Ret<String> {
            let serial = item.serial.uint();
            let meta = if let Some(meta) = cache.get(&serial) {
                meta.clone()
            } else {
                let meta = load(item.serial)?;
                cache.insert(serial, meta.clone());
                meta
            };
            Ok(asset_item_meta_json(item, meta.as_ref()))
        })
        .collect::<Ret<Vec<_>>>()?
        .join(",");
    Ok(format!("[{}]", items))
}

fn listed_assets(
    balance: &Balance,
    asset: Option<&str>,
    include_all: bool,
) -> Option<Vec<AssetAmt>> {
    let mut listed = None;
    if let Some(asset) = asset {
        match asset.parse::<u64>() {
            Ok(serial) => {
                listed = Some(
                    Fold64::from(serial)
                        .ok()
                        .and_then(|serial| balance.asset(serial))
                        .into_iter()
                        .collect::<Vec<_>>(),
                );
            }
            Err(_) => listed = Some(balance.assets.as_list().clone()),
        }
    }
    if include_all {
        listed = Some(balance.assets.as_list().clone());
    }
    listed
}

pub(crate) fn balance_item_json(
    balance: &Balance,
    diamonds: Option<String>,
    assets: Option<String>,
    unit: &str,
    show_hacash: bool,
    show_satoshi: bool,
    show_diamond: bool,
) -> String {
    let mut fields = Vec::new();
    if show_hacash {
        fields.push(format!(
            "\"hacash\":{}",
            json_string(&balance.hacash.to_unit_string(unit))
        ));
    }
    if show_diamond {
        fields.push(format!("\"diamond\":{}", balance.diamond.uint()));
    }
    if show_satoshi {
        fields.push(format!("\"satoshi\":{}", balance.satoshi.uint()));
    }
    if let Some(diamonds) = diamonds {
        fields.push(format!("\"diamonds\":{}", json_string(&diamonds)));
    }
    if let Some(assets) = assets {
        fields.push(format!("\"assets\":{}", assets));
    }
    format!("{{{}}}", fields.join(","))
}

pub(crate) fn balance_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let include_diamond_names = q_bool(&req, "diamonds", false);
    let include_all_assets = q_bool(&req, "assets", false);
    let include_asset_meta = q_bool(&req, "asset_meta", false);
    let asset = req.query("asset").map(|s| s.to_owned());
    let (show_hacash, show_satoshi, show_diamond) = match q_coinkind_hsd(&req) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let addresses = q_string(&req, "address", "")
        .replace(' ', "")
        .replace('\n', "");
    let parts = addresses.split(',').collect::<Vec<_>>();
    if parts.is_empty() || (parts.len() == 1 && parts[0].is_empty()) {
        return api_error("address format invalid");
    }
    if parts.len() > 200 {
        return api_error("address count must not exceed 200");
    }

    // §13: multi-read API uses optimistic snapshot; validate at the end
    // after both balance and optional asset-metadata reads.
    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let core = CoreStateRead::wrap(snapshot.view());
    let mut meta_cache: HashMap<u64, Option<AssetSmelt>> = HashMap::new();
    let mut out = Vec::with_capacity(parts.len());
    for raw in parts {
        let addr = match Address::from_readable(raw) {
            Ok(v) => v,
            Err(_) => return api_error(&format!("address {} format invalid", raw)),
        };
        let balance = match core.balance(&addr) {
            Ok(balance) => balance.unwrap_or_default(),
            Err(e) => return api_state_read_error(&e),
        };
        let diamond_names = if include_diamond_names && show_diamond {
            match core.diamond_owned(&addr) {
                Ok(owned) => Some(owned.unwrap_or_default().readable()),
                Err(e) => return api_state_read_error(&e),
            }
        } else {
            None
        };
        let listed = listed_assets(&balance, asset.as_deref(), include_all_assets);
        let asset_json = match listed {
            Some(list) if include_asset_meta => {
                match asset_list_json_meta(&list, &mut meta_cache, |serial| core.asset(&serial)) {
                    Ok(json) => Some(json),
                    Err(e) => return api_state_read_error(&e),
                }
            }
            Some(list) => Some(asset_list_json(&list)),
            None => None,
        };
        out.push(balance_item_json(
            &balance,
            diamond_names,
            asset_json,
            &unit,
            show_hacash,
            show_satoshi,
            show_diamond,
        ));
    }
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    ApiResponse::json(format!("{{\"ret\":0,\"list\":[{}]}}", out.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::{BytesW1, Uint1};
    use serde_json::{Value, json};

    fn amt(serial: u64, amount: u64) -> AssetAmt {
        AssetAmt {
            serial: Fold64::from(serial).unwrap(),
            amount: Fold64::from(amount).unwrap(),
        }
    }

    fn smelt(serial: u64, supply: u64, decimal: u8, ticket: &[u8], name: &[u8]) -> AssetSmelt {
        AssetSmelt {
            serial: Fold64::from(serial).unwrap(),
            supply: Fold64::from(supply).unwrap(),
            decimal: Uint1::from(decimal),
            issuer: Address::default(),
            ticket: BytesW1::from(ticket.to_vec()).unwrap(),
            name: BytesW1::from(name.to_vec()).unwrap(),
        }
    }

    fn parse_list(s: &str) -> Value {
        serde_json::from_str(s).expect("valid JSON array")
    }

    #[test]
    fn compact_assets_keep_numeric_serial_and_amount() {
        let list = asset_list_json(&[amt(1001, 123456)]);
        assert_eq!(list, r#"[{"serial":1001,"amount":123456}]"#);
        let value = parse_list(&list);
        assert!(value[0]["serial"].is_number());
        assert!(value[0]["amount"].is_number());
        assert!(value[0].get("name").is_none());
        assert!(value[0].get("decimal").is_none());
    }

    #[test]
    fn empty_asset_list_is_empty_array() {
        assert_eq!(asset_list_json(&[]), "[]");
        let mut cache = HashMap::new();
        assert_eq!(
            asset_list_json_meta(&[], &mut cache, |_| panic!("no load")).unwrap(),
            "[]"
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn asset_meta_returns_strings_and_name_ticket_decimal() {
        let meta = smelt(1001, 1_000_000, 2, b"USDX", b"USD Coin");
        let mut cache = HashMap::new();
        let list =
            asset_list_json_meta(&[amt(1001, 123456)], &mut cache, |_| Ok(Some(meta.clone())))
                .unwrap();
        let value = parse_list(&list);
        assert_eq!(value[0]["serial"], "1001");
        assert_eq!(value[0]["amount"], "123456");
        assert_eq!(value[0]["decimal"], 2);
        assert_eq!(value[0]["name"], "USD Coin");
        assert_eq!(value[0]["ticket"], "USDX");
        assert_eq!(value[0]["supply"], "1000000");
        assert!(value[0]["serial"].is_string());
        assert!(value[0]["amount"].is_string());
        assert!(value[0]["supply"].is_string());
        assert!(value[0]["decimal"].is_number());
    }

    #[test]
    fn asset_meta_large_integers_stay_exact_decimal_strings() {
        let serial = Fold64::MAX;
        let amount = Fold64::MAX;
        let meta = smelt(serial, amount, 0, b"BIG", b"Big");
        let mut cache = HashMap::new();
        let list = asset_list_json_meta(&[amt(serial, amount)], &mut cache, |_| {
            Ok(Some(meta.clone()))
        })
        .unwrap();
        let value = parse_list(&list);
        let serial_s = serial.to_string();
        let amount_s = amount.to_string();
        assert_eq!(value[0]["serial"], serial_s);
        assert_eq!(value[0]["amount"], amount_s);
        assert_eq!(value[0]["supply"], amount_s);
        assert_eq!(serial_s, "2305843009213693951");
        assert_ne!(serial, serial as f64 as u64); // would lose precision as JSON number
    }

    #[test]
    fn asset_meta_missing_or_invalid_does_not_invent_decimal() {
        let mut cache = HashMap::new();
        let missing =
            parse_list(&asset_list_json_meta(&[amt(9, 1)], &mut cache, |_| Ok(None)).unwrap());
        assert_eq!(missing[0]["metadata"], false);
        assert_eq!(missing[0]["decimal"], Value::Null);
        assert_eq!(missing[0]["name"], "Asset #9");
        assert_eq!(missing[0]["serial"], "9");
        assert_eq!(missing[0]["amount"], "1");

        cache.clear();
        let bad_decimal = smelt(9, 1, 17, b"X", b"Bad");
        let invalid = parse_list(
            &asset_list_json_meta(&[amt(9, 1)], &mut cache, |_| Ok(Some(bad_decimal.clone())))
                .unwrap(),
        );
        assert_eq!(invalid[0]["metadata"], false);
        assert_eq!(invalid[0]["decimal"], Value::Null);
        assert_eq!(invalid[0]["name"], "Asset #9");
    }

    #[test]
    fn asset_meta_name_falls_back_to_hex_when_not_readable() {
        let meta = smelt(7, 1, 0, b"OK", &[b'A', 0, b'B']);
        let mut cache = HashMap::new();
        let value = parse_list(
            &asset_list_json_meta(&[amt(7, 5)], &mut cache, |_| Ok(Some(meta.clone()))).unwrap(),
        );
        assert_eq!(value[0]["ticket"], "OK");
        assert_eq!(value[0]["name"], "410042");
        assert_eq!(value[0]["decimal"], 0);
    }

    #[test]
    fn asset_meta_caches_duplicate_serials() {
        let meta = smelt(1001, 10, 2, b"USDX", b"USD Coin");
        let mut loads = 0u32;
        let mut cache = HashMap::new();
        let first = amt(1001, 1);
        let second = amt(1001, 2);
        let _ = asset_list_json_meta(&[first.clone(), second.clone()], &mut cache, |_| {
            loads += 1;
            Ok(Some(meta.clone()))
        })
        .unwrap();
        let _ = asset_list_json_meta(&[first], &mut cache, |_| {
            loads += 1;
            Ok(Some(meta.clone()))
        })
        .unwrap();
        assert_eq!(loads, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn asset_meta_propagates_state_read_errors() {
        let mut cache = HashMap::new();
        let result = asset_list_json_meta(&[amt(7, 5)], &mut cache, |_| {
            Err(sys::Error::normal("asset state unavailable"))
        });
        assert!(result.is_err());
        assert!(cache.is_empty());
    }

    #[test]
    fn listed_assets_preserves_asset_filter_and_assets_true() {
        let mut balance = Balance::default();
        balance.asset_set(amt(1001, 10)).unwrap();
        balance.asset_set(amt(1002, 20)).unwrap();
        let filtered = listed_assets(&balance, Some("1001"), false).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].serial.uint(), 1001);
        let missing = listed_assets(&balance, Some("9"), false).unwrap();
        assert!(missing.is_empty());
        let all = listed_assets(&balance, Some("1001"), true).unwrap();
        assert_eq!(all.len(), 2);
        let none = listed_assets(&balance, None, false);
        assert!(none.is_none());
        let empty = listed_assets(&Balance::default(), None, true).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn compact_and_meta_json_are_valid_objects() {
        let compact = balance_item_json(
            &Balance::default(),
            None,
            Some(asset_list_json(&[amt(1, 2)])),
            "mei",
            true,
            true,
            true,
        );
        let value: Value = serde_json::from_str(&compact).unwrap();
        assert_eq!(value["assets"], json!([{"serial": 1, "amount": 2}]));
    }
}
