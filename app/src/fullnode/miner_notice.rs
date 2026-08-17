//! `/query/miner/notice` 长轮询端点（async 版）。
//!
//! mint 已移除 tokio（`miner_notice_wait_async` 删除，仅保留同步
//! `miner_notice_wait` 与公开的 `begin_miner_notice` guard）。该 async 轮询
//! 随 tokio 运行环境迁入 app——HTTP 路由由 server crate 的 tokio 运行时驱动，
//! 此处只提供处理 future。语义与旧 mint 实现一致：登记等待计数、
//! 轮询高度直到目标或超时。

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
