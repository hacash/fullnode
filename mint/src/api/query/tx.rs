use base::{ApiExecCtx, ApiRequest, ApiResponse, CoreStateRead};

use crate::api::util::*;

use sys::ToHex;

pub(crate) fn transaction_query_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let hash = q_string(&req, "hash", "");
    let body = q_bool(&req, "body", false);
    let action = q_bool(&req, "action", false);
    let signature = q_bool(&req, "signature", false);
    let description = q_bool(&req, "description", false);
    let last_height = ctx.engine.latest_height();

    let hx = match hex::decode(&hash) {
        Ok(v) => v,
        Err(_) => return api_error("transaction hash format invalid"),
    };
    if hx.len() != field::Hash::SIZE {
        return api_error("transaction hash format invalid");
    }
    let mut raw = [0u8; field::Hash::SIZE];
    raw.copy_from_slice(&hx);
    let tx_hash = field::Hash::from(raw);

    if let Some(pkg) = ctx.node.txpool().find(tx_hash.as_ref()) {
        return ApiResponse::json(transaction_basic_json(
            pkg.tx(),
            None,
            last_height,
            &unit,
            body,
            action,
            signature,
            description,
            true,
        ));
    }

    let Some(snapshot) = ctx.engine.optimistic_canonical() else {
        return api_error("state changed");
    };
    let last_height = snapshot.head_height;
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    let Some(height) = state.tx_exist(&tx_hash) else {
        if !ctx.engine.validate_optimistic(start_epoch) {
            return api_error("state changed");
        }
        return api_error("transaction not found");
    };
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    let pkg = match load_block_by_key(ctx, &height.uint().to_string()) {
        Ok(v) => v,
        Err(_) => return api_error("cannot find block by transaction ptr"),
    };
    let Some(tx) = pkg
        .block()
        .transactions()
        .iter()
        .find(|tx| tx.hash() == tx_hash)
    else {
        return api_error("transaction not found in the block");
    };
    ApiResponse::json(transaction_basic_json(
        tx.as_ref(),
        Some(pkg.block()),
        last_height,
        &unit,
        body,
        action,
        signature,
        description,
        false,
    ))
}

pub(crate) fn submit_transaction_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let data = match body_data_may_hex(&req) {
        Ok(v) => v,
        Err(_) => return api_error("transaction body invalid"),
    };
    let pkg = match base::TxPkg::from_bytes(
        ctx.engine.services().as_ref(),
        data,
        base::PkgSource::new(base::PkgOrigin::Api),
    ) {
        Ok(v) => v,
        Err(_) => return api_error("transaction parse failed"),
    };
    if let Some(resp) = crate::api::transaction::reject_non_canonical_dia_insc_push(pkg.tx()) {
        return resp;
    }
    let only_pool = q_bool(&req, "only_insert_txpool", false);
    if let Err(e) = ctx.node.submit_transaction(&pkg, false, only_pool) {
        return api_error(&e.to_string());
    }
    api_ok(vec![("hash", json_string(&pkg.hash().as_ref().to_hex()))])
}

pub(crate) fn submit_block_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let data = match body_data_may_hex(&req) {
        Ok(v) => v,
        Err(_) => return api_error("block body invalid"),
    };
    let pkg = match base::BlkPkg::from_bytes(
        ctx.engine.services().as_ref(),
        data,
        base::PkgSource::new(base::PkgOrigin::Api),
    ) {
        Ok(v) => v,
        Err(_) => return api_error("block parse failed"),
    };
    if let Err(e) = ctx.node.submit_block(&pkg, true) {
        return api_error(&format!("submit block failed: {}", e));
    }
    api_ok(vec![("ok", "true".to_owned())])
}
