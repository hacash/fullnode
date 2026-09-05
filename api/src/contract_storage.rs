//! Contract storage fee discount observability API (§8.D): read-only facts —
//! active parameters, remaining budget `B`, capacity `C`, refill `R`, current
//! periods, next-block discount quota, and the next-block estimated price
//! interval. Wallets use `periods` for discount quotes and `p_max` (full price)
//! for guaranteed inclusion; the params hash pins the rule version on chain.

use base::{ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService};

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        s.push_str(&format!("{:02x}", b));
        s
    })
}

fn contract_storage_fee_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let vp = match ctx.engine.services().vm_params() {
        Ok(vp) => *vp,
        Err(e) => return ApiResponse::err(500, &format!("vm params unavailable: {}", e)),
    };
    let snapshot = match ctx.engine.optimistic_canonical() {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return ApiResponse::err(503, "state is busy or unavailable"),
        Err(e) => return base::api_state_read_error(&e),
    };
    // Facts are computed as they will apply to the next executed block (§8.D).
    let facts = match base::contract_storage_fee_facts(
        snapshot.view(),
        &vp,
        snapshot.head_height + 1,
    ) {
        Ok(facts) => facts,
        Err(e) => return base::api_state_read_error(&e),
    };
    // Fee quote for a concrete payload: full price and, when active, the discount
    // minimum. Both scale linearly with the transaction's fee purity over the
    // reported `fee_purity_floor` (see §3.4).
    let charge_bytes = req.query_u64("charge_bytes").unwrap_or(0);
    let quote = match base::contract_storage_fee_quote(
        snapshot.view(),
        &vp,
        snapshot.head_height + 1,
        charge_bytes,
    ) {
        Ok(quote) => quote,
        Err(e) => return base::api_state_read_error(&e),
    };
    let params_hash = ctx
        .engine
        .services()
        .execution_profile()
        .ok()
        .and_then(hacash_params::as_hacash_params)
        .map(|p| format!("\"{}\"", hex_bytes(&hacash_params::params_hash(p))))
        .unwrap_or_else(|| "null".to_string());
    let storage = vp.contract_storage_fee;
    let schedule: Vec<String> = storage
        .supplement_schedule
        .iter()
        .map(|(h, r)| format!("[{},{}]", h, r))
        .collect();
    ApiResponse::json(format!(
        concat!(
            "{{\"ret\":0,\"enabled\":{},\"rule_version\":{},",
            "\"params_hash\":{},\"activation_height\":{},\"head_height\":{},\"pending_height\":{},",
            "\"target_capacity_blocks\":{},\"capacity\":{},\"rate\":{},\"remaining\":{},",
            "\"p_min\":{},\"p_max\":{},\"periods\":{},\"k_max\":{},",
            "\"next_block_quota\":{},\"next_remaining_lo\":{},\"next_remaining_hi\":{},",
            "\"next_periods_lo\":{},\"next_periods_hi\":{},\"schedule\":[{}],",
            "\"legacy_fixed_periods\":{},\"fee_purity_floor\":{},\"fee_full_u238\":{},",
            "\"fee_discount_u238\":{}}}"
        ),
        facts.enabled,
        facts.rule_version,
        params_hash,
        storage.activation_height,
        snapshot.head_height,
        snapshot.head_height + 1,
        storage.target_capacity_blocks,
        facts.capacity,
        facts.rate,
        facts.remaining,
        storage.period_floor,
        vp.contract_store_perm_periods,
        facts.periods,
        storage.max_block_discount_bytes,
        facts.next_block_quota,
        facts.next_remaining_lo,
        facts.next_remaining_hi,
        facts.next_periods_lo,
        facts.next_periods_hi,
        schedule.join(","),
        vp.contract_store_perm_periods,
        quote.floor_purity,
        quote.full_u238,
        quote
            .discount_u238
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string()),
    ))
}

pub struct ContractStorageApi;
impl ApiService for ContractStorageApi {
    fn name(&self) -> &str {
        "contract_storage"
    }
    fn routes(&self) -> Vec<ApiRoute> {
        vec![ApiRoute::get(
            "/query/contract/storage_fee",
            contract_storage_fee_handler,
        )]
    }
}
