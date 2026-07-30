//! Status API service.

use base::{ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService};

// =============================================================
// APIStatusApi
// =============================================================

fn status_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    let height = ctx.engine.latest_height();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let uptime = now.saturating_sub(ctx.launch_time);
    ApiResponse::json(format!(
        "{{\"height\":{},\"uptime\":{},\"peers\":{}}}",
        height,
        uptime,
        ctx.node.all_peer_prints().len()
    ))
}

pub struct StatusApi;
impl ApiService for StatusApi {
    fn name(&self) -> &str {
        "status"
    }
    fn routes(&self) -> Vec<ApiRoute> {
        vec![ApiRoute::get("/status", status_handler)]
    }
}
