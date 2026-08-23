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
    // Registered profile VM parameters: offline tools (fitshc / SDK) estimate
    // HVM deployment protocol_cost against the node's real storage-rent rules.
    let (fee_purity_floor, perm_periods) = ctx
        .engine
        .services()
        .execution_profile()
        .ok()
        .and_then(hacash_params::as_hacash_params)
        .map(|p| {
            (
                p.protocol.vm.initial_fee_purity_floor,
                p.protocol.vm.contract_store_perm_periods,
            )
        })
        .unwrap_or((0, 0));
    ApiResponse::json(format!(
        "{{\"height\":{},\"uptime\":{},\"peers\":{},\"vm_fee_purity_floor\":{},\"vm_contract_store_perm_periods\":{}}}",
        height,
        uptime,
        ctx.node.all_peer_prints().len(),
        fee_purity_floor,
        perm_periods,
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
