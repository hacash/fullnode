//! `/query/miner/notice` long-poll endpoint (async version).
//!
//! mint has dropped tokio (`miner_notice_wait_async` removed; only the sync
//! `miner_notice_wait` and the public `begin_miner_notice` guard remain). This async poll
//! moved into app with the tokio runtime — HTTP routes are driven by the server crate's
//! tokio runtime; here we only provide the handling future. Semantics match the old mint
//! implementation: bump the waiting count, poll the height until target or timeout.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base::{ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService, ChainView};

pub struct MinerNoticeApi {
    pub consensus: Arc<mint::HacashConsensus>,
}

impl ApiService for MinerNoticeApi {
    fn name(&self) -> &str {
        "miner_notice"
    }

    fn routes(&self) -> Vec<ApiRoute> {
        let consensus = self.consensus.clone();
        vec![ApiRoute::get_async("/query/miner/notice", move |ctx, req| {
            miner_notice_long_poll(consensus.clone(), ctx, req)
        })]
    }
}

async fn miner_notice_long_poll(
    consensus: Arc<mint::HacashConsensus>,
    ctx: ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    let target_height = req.query_u64("height").unwrap_or(0);
    let wait = req.query_u64("wait").unwrap_or(45);
    let height = wait_notice_async(consensus, ctx.engine.as_ref(), target_height, wait).await;
    ApiResponse::json(format!("{{\"ret\":0,\"height\":{}}}", height))
}

async fn wait_notice_async(
    consensus: Arc<mint::HacashConsensus>,
    view: &dyn ChainView,
    target_height: u64,
    wait_secs: u64,
) -> u64 {
    let _guard = consensus.begin_miner_notice();
    let wait_secs = wait_secs.clamp(1, 300);
    let start = Instant::now();
    let wait = Duration::from_secs(wait_secs);
    let poll = Duration::from_millis(250);
    loop {
        let current_height = view.latest_height();
        if target_height > 0 && current_height >= target_height {
            return current_height;
        }
        if start.elapsed() >= wait {
            return current_height;
        }
        let sleep_for = poll.min(wait.saturating_sub(start.elapsed()));
        tokio::time::sleep(sleep_for).await;
    }
}
