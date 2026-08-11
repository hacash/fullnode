use base::{ApiExecCtx, ApiRequest, ApiResponse, CoreStateRead};

use crate::api::util::*;

use field::{Address, AssetAmt, Balance, Fold64};

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

    // §13: multi-read API uses optimistic snapshot; validate at the end.
    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let core = CoreStateRead::wrap(snapshot.view());
    let mut out = Vec::with_capacity(parts.len());
    for raw in parts {
        let addr = match Address::from_readable(raw) {
            Ok(v) => v,
            Err(_) => return api_error(&format!("address {} format invalid", raw)),
        };
        let balance = core.balance(&addr).unwrap_or_default();
        let diamond_names = if include_diamond_names && show_diamond {
            Some(core.diamond_owned(&addr).unwrap_or_default().readable())
        } else {
            None
        };
        let mut asset_json = None;
        if let Some(asset) = asset.as_ref() {
            match asset.parse::<u64>() {
                Ok(serial) => {
                    let list = Fold64::from(serial)
                        .ok()
                        .and_then(|serial| balance.asset(serial))
                        .into_iter()
                        .collect::<Vec<_>>();
                    asset_json = Some(asset_list_json(&list));
                }
                Err(_) => asset_json = Some(asset_list_json(balance.assets.as_list())),
            }
        }
        if include_all_assets {
            asset_json = Some(asset_list_json(balance.assets.as_list()));
        }
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
