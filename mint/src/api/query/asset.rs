//! `/query/asset` — address-independent asset metadata lookup by serial.
//!
//! `/query/balance?asset=<serial>&asset_meta=true` only lists an asset when the
//! queried address actually holds it, so a signer that does not hold the asset
//! (federation key, multisig co-signer, receiving-only account) cannot resolve
//! `decimal/name/ticket` for display. This handler resolves the AssetSmelt record
//! straight from chain state, no account involved.

use base::{ApiExecCtx, ApiRequest, ApiResponse, CoreStateRead};
use field::{AssetSmelt, Fold64};

use crate::api::util::*;

fn asset_meta_json(serial: u64, meta: Option<&AssetSmelt>) -> String {
    let serial_js = json_string(&serial.to_string());
    match meta {
        // decimal > 16 cannot be rendered losslessly by clients; report metadata:false
        // instead of inventing a display scale (same rule as /query/balance asset_meta).
        Some(smelt) if smelt.decimal.uint() <= 16 => format!(
            "{{\"serial\":{},\"decimal\":{},\"name\":{},\"ticket\":{},\"supply\":{},\"metadata\":true}}",
            serial_js,
            smelt.decimal.uint(),
            json_string(&smelt.name.to_readable_or_hex()),
            json_string(&smelt.ticket.to_readable_or_hex()),
            json_string(&smelt.supply.uint().to_string()),
        ),
        _ => format!("{{\"serial\":{},\"metadata\":false}}", serial_js),
    }
}

pub(crate) fn asset_query_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let Some(serial) = req.query_u64("serial") else {
        return api_error("serial format invalid");
    };
    let serial_fold = match Fold64::from(serial) {
        Ok(v) => v,
        Err(_) => return api_error("serial out of range"),
    };
    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let core = CoreStateRead::wrap(snapshot.view());
    let meta = match core.asset(&serial_fold) {
        Ok(meta) => meta,
        Err(e) => return api_state_read_error(&e),
    };
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    ApiResponse::json(format!(
        "{{\"ret\":0,\"asset\":{}}}",
        asset_meta_json(serial, meta.as_ref())
    ))
}

#[cfg(test)]
mod tests {
    use super::asset_meta_json;
    use field::{AssetSmelt, Fold64};

    fn smelt(decimal: u8) -> AssetSmelt {
        let mut s = AssetSmelt::default();
        s.decimal = decimal.into();
        s
    }

    #[test]
    fn missing_meta_reports_metadata_false() {
        let json = asset_meta_json(5, None);
        assert_eq!(json, "{\"serial\":\"5\",\"metadata\":false}");
    }

    #[test]
    fn unrenderable_decimal_does_not_invent_scale() {
        let json = asset_meta_json(5, Some(&smelt(17)));
        assert_eq!(json, "{\"serial\":\"5\",\"metadata\":false}");
    }

    #[test]
    fn meta_serial_and_decimal_are_strings_and_numbers() {
        let json = asset_meta_json(5, Some(&smelt(6)));
        assert!(json.contains("\"serial\":\"5\""), "{json}");
        assert!(json.contains("\"decimal\":6"), "{json}");
        assert!(json.contains("\"metadata\":true"), "{json}");
    }

    #[test]
    fn serial_stays_exact_decimal_string() {
        // Fold64 values exceed 2^53: the serial must travel as a quoted decimal string.
        let big = (1u64 << 61) - 1;
        let json = asset_meta_json(big, None);
        assert!(json.contains(&format!("\"serial\":\"{big}\"")), "{json}");
        assert_eq!(Fold64::from(big).map(|v| v.uint()).unwrap(), big);
    }
}
