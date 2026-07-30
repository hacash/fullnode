//! Mempool API service.

use base::{ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService};

// =============================================================
// APIPoolApi + debug
// =============================================================

fn pool_summary_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    let pool = ctx.node.txpool();
    let groups = pool
        .group_ids()
        .into_iter()
        .map(|id| format!("\"group{}\":{}", id.get(), pool.count(id)))
        .collect::<Vec<_>>()
        .join(",");
    ApiResponse::json(format!("{{{groups}}}"))
}

fn pool_print_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    ApiResponse::text(ctx.node.txpool().print())
}

pub struct PoolApi;
impl ApiService for PoolApi {
    fn name(&self) -> &str {
        "pool"
    }
    fn routes(&self) -> Vec<ApiRoute> {
        vec![
            ApiRoute::get("/pool_summary", pool_summary_handler),
            ApiRoute::debug_get("/pool_print", pool_print_handler),
        ]
    }
}
