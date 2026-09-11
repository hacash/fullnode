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
    // Contract storage discount, state-aware: the full price `p_max` and, once the
    // mechanism is active, the discount periods applying to the next block's
    // deploy/update fees. Quoting wallets use `/query/contract/storage_fee` for the
    // full fact set (B, C, R, quota, schedule, params hash).
    let (csf_enabled, csf_periods) = match ctx.engine.services().vm_params() {
        Ok(vp) => {
            let vp = *vp;
            let p_max = vp.contract_store_perm_periods;
            if !vp.contract_storage_fee.is_active_at(height + 1) {
                (false, p_max)
            } else {
                let periods = ctx
                    .engine
                    .state_canonical()
                    .ok()
                    .flatten()
                    .and_then(|session| {
                        base::contract_storage_fee_facts(session.view(), &vp, height + 1).ok()
                    })
                    .map(|facts| facts.periods)
                    .unwrap_or(p_max);
                (true, periods)
            }
        }
        Err(_) => (false, 0),
    };
    ApiResponse::json(format!(
        "{{\"height\":{},\"uptime\":{},\"peers\":{},\"vm_fee_purity_unit\":{},\"vm_fee_purity_floor\":{},\"vm_contract_store_perm_periods\":{},\"vm_contract_storage_enabled\":{},\"vm_contract_storage_periods\":{}}}",
        height,
        uptime,
        ctx.node.all_peer_prints().len(),
        base::FEE_PRICING_UNIT,
        fee_purity_floor,
        perm_periods,
        csf_enabled,
        csf_periods,
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
