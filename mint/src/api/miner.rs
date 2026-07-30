use std::sync::Arc;

#[cfg(not(debug_assertions))]
use base::TxPolicy;
use base::{ApiExecCtx, ApiRequest, ApiResponse};
use sys::ToHex;

use crate::HacashConsensus;

use super::util::*;

pub(crate) fn miner_pending_handler(
    cons: Arc<HacashConsensus>,
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    let detail = q_bool(&req, "detail", false);
    let transaction = q_bool(&req, "transaction", false);
    let stuff = q_bool(&req, "stuff", false);
    let base64 = q_bool(&req, "base64", false);

    if !cons.miner_enabled() {
        return api_error("miner not enabled");
    }

    // Mainnet warm-up: refuse pending until 30s after launch unless a diamond
    // mint tx is already in the pool (same guard as fullnodedev).
    #[cfg(not(debug_assertions))]
    {
        let chain_id = ctx.engine.consensus().chain_id();
        let diam_group = cons
            .tx_pool_groups()
            .into_iter()
            .find_map(|spec| {
                if spec.relay_service_bit == Some(HacashConsensus::SERVICE_BIT_DIAMOND_RELAY) {
                    Some(spec.id)
                } else {
                    None
                }
            })
            .unwrap_or(HacashConsensus::TX_GROUP_DIAMOND_MINT);
        let got_diam = ctx.node.txpool().first(diam_group).is_some();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if chain_id.is_mainnet() && !got_diam && now < ctx.launch_time.saturating_add(30) {
            return api_error("miner worker must be launched at least 30 secs after node start");
        }
    }

    match cons.miner_pending_work(ctx.engine.as_ref(), ctx.node.txpool().as_ref()) {
        Ok(work) => ApiResponse::json(miner_pending_json(work, detail, transaction, stuff, base64)),
        Err(e) => api_error(&e.to_string()),
    }
}

pub(crate) fn miner_success_handler(
    cons: Arc<HacashConsensus>,
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    let height = req.query_u64("height").unwrap_or(0);
    let block_nonce = q_u32(&req, "block_nonce", 0);
    let coinbase_nonce = q_string(&req, "coinbase_nonce", "");
    let Ok(bytes) = hex::decode(coinbase_nonce.as_bytes()) else {
        return api_error("coinbase nonce format invalid");
    };
    let Ok(arr) = <[u8; field::Hash::SIZE]>::try_from(bytes.as_slice()) else {
        return api_error("coinbase nonce length invalid");
    };
    let pkg = match cons.miner_success_block(
        ctx.engine.services().as_ref(),
        height,
        block_nonce,
        field::Hash::from(arr),
    ) {
        Ok(pkg) => pkg,
        Err(e) => return api_error(&e.to_string()),
    };
    if let Err(e) = ctx.node.submit_block(&pkg, false) {
        return api_error(&format!("submit block failed: {}", e));
    }
    cons.miner_mark_block_submitted(height);
    api_ok(vec![
        ("height", height.to_string()),
        ("mining", json_string("success")),
    ])
}

pub(crate) async fn miner_notice_handler(
    cons: Arc<HacashConsensus>,
    ctx: ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    let target_height = req.query_u64("height").unwrap_or(0);
    let wait = req.query_u64("wait").unwrap_or(45);
    let height = cons
        .miner_notice_wait_async(ctx.engine.as_ref(), target_height, wait)
        .await;
    api_ok(vec![("height", height.to_string())])
}

pub(crate) fn diamondminer_init_handler(
    cons: Arc<HacashConsensus>,
    _ctx: &ApiExecCtx,
    _req: ApiRequest,
) -> ApiResponse {
    if !cons.diamond_miner_enabled() {
        return api_error("diamond miner not enabled");
    }
    api_ok(vec![
        (
            "bid_address",
            json_string(&cons.diamond_miner_bid_address().to_readable()),
        ),
        (
            "reward_address",
            json_string(&cons.diamond_miner_reward_address().to_readable()),
        ),
    ])
}

pub(crate) fn diamondminer_success_handler(
    cons: Arc<HacashConsensus>,
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    let data = match body_data_may_hex(&req) {
        Ok(v) => v,
        Err(_) => return api_error("hex format invalid"),
    };
    let pkg = match cons.diamond_miner_success_tx(
        ctx.engine.services().as_ref(),
        ctx.engine.as_ref(),
        ctx.node.txpool().as_ref(),
        ctx.node.as_ref(),
        data,
    ) {
        Ok(pkg) => pkg,
        Err(e) => return api_error(&e.to_string()),
    };
    api_ok(vec![(
        "tx_hash",
        json_string(&pkg.hash().as_ref().to_hex()),
    )])
}
