//! `GET /` node console page (documented verification entry, see
//! fullnode_api_doc_v2). Ported from fullnodedev mint api console.rs; the
//! miner worker notice count comes from `HacashConsensus` instead of the old
//! ApiExecCtx counter.

use std::sync::Arc;

use base::{ApiExecCtx, ApiRequest, ApiResponse, Consensus};

use crate::HacashConsensus;

use super::util::api_html;

pub(crate) fn console_handler(
    cons: Arc<HacashConsensus>,
    ctx: &ApiExecCtx,
    _req: ApiRequest,
) -> ApiResponse {
    let mtcnf = cons.mint_params();
    let latest = ctx.engine.latest_block();
    let lathei = latest.height() as i64;
    let latts = latest.timestamp();

    let cyln = mtcnf.difficulty_adjust_blocks as i64;
    let secnp = ["day", "week", "month", "quarter", "year", "all"];
    let secn = [cyln, cyln * 7, cyln * 30, cyln * 90, cyln * 365, lathei - 1];
    let mut target_time = Vec::with_capacity(secn.len());

    for i in 0..secn.len() {
        let sb = secn[i];
        let hei = lathei - sb;
        if hei <= 0 {
            break;
        }
        let blkt = match ctx.engine.block_history().block_at_height(hei as u64) {
            Ok(Some(block)) => block.timestamp(),
            _ => break,
        };
        target_time.push(format!("{}: {}s", secnp[i], (latts - blkt) / (sb as u64)));
    }

    let poworkers = cons.miner_notice_count();

    api_html(format!(
        r#"<html><head><title>Hacash node console</title></head><body>
        <h3>Hacash console</h3>
        <p>Latest height {} time {}</p>
        <p>Block span times: {}</p>
        <p>P2P peers: {}</p>
        <p>{}</p>
        <p>Miner worker notice connected: {}</p>
    </body></html>"#,
        latest.height(),
        sys::timeshow(latest.timestamp()),
        target_time.join(", "),
        ctx.node.all_peer_prints().join(", "),
        ctx.node.txpool().print(),
        poworkers,
    ))
}
