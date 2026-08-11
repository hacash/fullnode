use base::{ApiExecCtx, ApiRequest, ApiResponse, CoreStateRead};

use crate::state::MintStateRead;

use crate::api::util::*;

pub(crate) fn supply_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    let mint_state = MintStateRead::wrap(snapshot.view());
    let result = supply_json(
        snapshot.head_height,
        &state.get_base_total(),
        &mint_state.get_mint_total(),
    );
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    match result {
        Ok(body) => ApiResponse::json(body),
        Err(e) => ApiResponse::err(500, &e.to_string()),
    }
}

pub(crate) fn latest_json(height: u64, diamond: u32) -> String {
    format!(
        "{{\"ret\":0,\"height\":{},\"diamond\":{}}}",
        height, diamond
    )
}

pub(crate) fn latest_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    let latest_diamond = state.latest_diamond().unwrap_or_default();
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    ApiResponse::json(latest_json(
        snapshot.head_height,
        latest_diamond.number.uint(),
    ))
}
