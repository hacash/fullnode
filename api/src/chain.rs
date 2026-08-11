//! Chain query API service.

use base::{ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService};

use super::util::hex_short;

fn block_by_height_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let h = match req.query_u64("height") {
        Some(v) => v,
        None => return ApiResponse::err(400, "missing query 'height'"),
    };
    match ctx.engine.store().block_data_by_height(h) {
        Ok(Some((hash, data))) => ApiResponse::json(format!(
            "{{\"height\":{},\"hash\":\"{}\",\"size\":{}}}",
            h,
            hex_short(hash.as_bytes()),
            data.len()
        )),
        Ok(None) => ApiResponse::err(404, "block not found"),
        Err(e) => ApiResponse::err(500, &format!("block read failed: {}", e)),
    }
}

fn latest_height_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    ApiResponse::json(format!(
        "{{\"ret\":0,\"height\":{}}}",
        ctx.engine.latest_height()
    ))
}

pub struct ChainApi;
impl ApiService for ChainApi {
    fn name(&self) -> &str {
        "chain"
    }
    fn routes(&self) -> Vec<ApiRoute> {
        vec![
            ApiRoute::get("/block_by_height", block_by_height_handler),
            // Generic latest-height path. Mint also exposes `/query/latest` as a
            // worker-facing alias — do not delete either without a client migration.
            ApiRoute::get("/query/block/height/latest", latest_height_handler),
        ]
    }
}
