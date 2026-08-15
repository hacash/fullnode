//!
//!
//!

use std::sync::Arc;

use base::{ApiResponse, ApiRoute, ApiService};

use crate::HacashConsensus;

mod console;
mod miner;
mod query;
mod transaction;
mod transfer;
mod util;

use console::console_handler;
use miner::{
    diamondminer_init_handler, diamondminer_success_handler, miner_notice_handler,
    miner_pending_handler, miner_success_handler,
};
use query::{
    balance_handler, block_datas_handler, block_intro_handler, block_pool_stats_handler,
    block_recents_handler, block_views_handler, channel_handler, diamond_bidding_handler,
    diamond_engrave_handler, diamond_handler, diamond_inscription_protocol_cost_append_handler,
    diamond_inscription_protocol_cost_drop_handler, diamond_inscription_protocol_cost_edit_handler,
    diamond_inscription_protocol_cost_handler, diamond_inscription_protocol_cost_move_handler,
    diamond_views_handler, fee_average_handler, hashrate_handler, hashrate_logs_handler,
    latest_handler, submit_block_handler, submit_transaction_handler, supply_handler,
    transaction_query_handler,
};
use transaction::{
    fee_raise_handler, transaction_build_handler, transaction_check_handler,
    transaction_sign_handler,
};
use transfer::{create_coin_transfer_handler, scan_coin_transfer_handler};

pub struct MintApi {
    consensus: Arc<HacashConsensus>,
}

impl ApiService for MintApi {
    fn name(&self) -> &str {
        "mint"
    }
    fn routes(&self) -> Vec<ApiRoute> {
        let cons = self.consensus.clone();
        let pending_cons = self.consensus.clone();
        let success_cons = self.consensus.clone();
        let notice_cons = self.consensus.clone();
        let diamond_init_cons = self.consensus.clone();
        let diamond_success_cons = self.consensus.clone();
        let console_cons = self.consensus.clone();
        vec![
            // Documented console verification entry (fullnode_api_doc_v2).
            ApiRoute::get("/", move |ctx, req| {
                console_handler(console_cons.clone(), ctx, req)
            }),
            ApiRoute::get("/miner/pending_replay", move |_ctx, _req| {
                ApiResponse::json(format!(
                    "{{\"pending_replay\":{}}}",
                    cons.pending_replay_count()
                ))
            }),
            ApiRoute::get("/query/miner/pending", move |ctx, req| {
                miner_pending_handler(pending_cons.clone(), ctx, req)
            }),
            ApiRoute::get("/submit/miner/success", move |ctx, req| {
                miner_success_handler(success_cons.clone(), ctx, req)
            }),
            ApiRoute::get_async("/query/miner/notice", move |ctx, req| {
                miner_notice_handler(notice_cons.clone(), ctx, req)
            }),
            ApiRoute::get("/query/diamondminer/init", move |ctx, req| {
                diamondminer_init_handler(diamond_init_cons.clone(), ctx, req)
            }),
            ApiRoute::post("/submit/diamondminer/success", move |ctx, req| {
                diamondminer_success_handler(diamond_success_cons.clone(), ctx, req)
            }),
            // Worker-facing latest alias; api also has `/query/block/height/latest`.
            // Do not delete either without a client migration.
            ApiRoute::get("/query/latest", latest_handler),
            ApiRoute::get("/query/supply", supply_handler),
            ApiRoute::get("/query/balance", balance_handler),
            ApiRoute::get("/query/diamond", diamond_handler),
            ApiRoute::get("/query/diamond/bidding", diamond_bidding_handler),
            ApiRoute::get("/query/diamond/views", diamond_views_handler),
            ApiRoute::get("/query/diamond/engrave", diamond_engrave_handler),
            ApiRoute::get(
                "/query/diamond/inscription_protocol_cost",
                diamond_inscription_protocol_cost_handler,
            ),
            ApiRoute::get(
                "/query/diamond/inscription_protocol_cost/append",
                diamond_inscription_protocol_cost_append_handler,
            ),
            ApiRoute::get(
                "/query/diamond/inscription_protocol_cost/move",
                diamond_inscription_protocol_cost_move_handler,
            ),
            ApiRoute::get(
                "/query/diamond/inscription_protocol_cost/edit",
                diamond_inscription_protocol_cost_edit_handler,
            ),
            ApiRoute::get(
                "/query/diamond/inscription_protocol_cost/drop",
                diamond_inscription_protocol_cost_drop_handler,
            ),
            ApiRoute::get("/query/channel", channel_handler),
            ApiRoute::get("/query/fee/average", fee_average_handler),
            ApiRoute::get("/query/hashrate", hashrate_handler),
            ApiRoute::get("/query/hashrate/logs", hashrate_logs_handler),
            ApiRoute::get("/query/block/intro", block_intro_handler),
            ApiRoute::get("/query/block/datas", block_datas_handler),
            ApiRoute::get("/query/block/views", block_views_handler),
            ApiRoute::get("/query/block/pools", block_pool_stats_handler),
            ApiRoute::get("/query/block/recents", block_recents_handler),
            ApiRoute::get("/query/transaction", transaction_query_handler),
            ApiRoute::post("/create/transaction", transaction_build_handler),
            ApiRoute::post("/submit/transaction", submit_transaction_handler),
            ApiRoute::post("/submit/block", submit_block_handler),
            ApiRoute::post("/operate/fee/raise", fee_raise_handler),
            ApiRoute::post("/util/transaction/check", transaction_check_handler),
            ApiRoute::post("/util/transaction/sign", transaction_sign_handler),
            // Hacash transfer build / scan (moved from the generic `api` crate;
            // these encode Hacash transfer semantics and depend on `protocol`).
            ApiRoute::get("/create/coin/transfer", create_coin_transfer_handler),
            ApiRoute::get("/query/coin/transfer", scan_coin_transfer_handler),
        ]
    }
}

pub fn api_services(consensus: Arc<HacashConsensus>) -> Vec<Arc<dyn ApiService>> {
    vec![Arc::new(MintApi { consensus })]
}
