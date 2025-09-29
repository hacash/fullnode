

pub fn routes(ctx: ApiCtx) -> Router {

    use ctx::*;
    
    let lrt = Router::new().route("/", get(console))
    
    // query
    .route(&query("latest"), get(latest))
    .route(&query("supply"), get(supply))
    .route(&query("hashrate"), get(hashrate))
    .route(&query("hashrate/logs"), get(hashrate_logs))
    .route(&query("balance"), get(balance))
    .route(&query("channel"), get(channel))
    .route(&query("coin/transfer"), get(scan_coin_transfer))

    .route(&query("block/intro"), get(block_intro))
    .route(&query("block/recents"), get(block_recents))
    .route(&query("block/views"), get(block_views))
    .route(&query("block/datas"), get(block_datas))

    .route(&query("transaction"), get(transaction_exist))

    .route(&query("diamond"), get(diamond))
    .route(&query("diamond/bidding"), get(diamond_bidding))
    .route(&query("diamond/views"), get(diamond_views))
    .route(&query("diamond/engrave"), get(diamond_engrave))
    .route(&query("diamond/inscription_protocol_cost"), get(diamond_inscription_protocol_cost))

    .route(&query("fee/average"), get(fee_average))

    .route(&query("miner/notice"), get(miner_notice))
    .route(&query("miner/pending"), get(miner_pending))
    .route(&query("diamondminer/init"), get(diamondminer_init))

    // create
    .route(&create("account"), get(account))
    .route(&create("transaction"), post(transaction_build))
    .route(&create("coin/transfer"), get(create_coin_transfer))
    
    // submit
    .route(&submit("transaction"), post(submit_transaction))
    .route(&submit("block"), post(submit_block))
    .route(&submit("miner/success"), get(miner_success))
    .route(&submit("diamondminer/success"), post(diamondminer_success))

    // operate
    .route(&operate("fee/raise"), post(fee_raise))

    // util
    .route(&util("transaction/check"), post(transaction_check))
    .route(&util("transaction/sign"), post(transaction_sign))


    ;

    // merge unstable & extend
    Router::new().merge(lrt)
    .merge(unstable::routes())
    .merge(extend::routes())
    .with_state(ctx)
    
}



