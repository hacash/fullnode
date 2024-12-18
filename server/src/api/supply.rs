

api_querys_define!{ Q9364,
    __nnn_, Option<bool>, None,
}

async fn supply(State(ctx): State<ApiCtx>, _q: Query<Q9364>) -> impl IntoResponse {
    ctx_state!(ctx, state);
    //
    let lasthei = ctx.engine.latest_block().height().uint();
    let lastdia = state.get_latest_diamond();
    // total supply
    const ZHU: u64 = 1_0000_0000;
    let supply = state.get_total_count();
    let blk_rwd = cumulative_block_reward(lasthei) * ZHU;
    let burn_fee = *supply.hacd_bid_burn_zhu + *supply.diamond_insc_burn_zhu;
    let curr_ccl = blk_rwd + *supply.channel_interest_zhu - burn_fee;
    //
    let z2m = |zhu|zhu as f64  / ZHU as f64;
    
    // return data
    let data = jsondata!{
        "latest_height", lasthei,
        "current_circulation", z2m(curr_ccl),

        "burned_fee",  z2m(burn_fee),
        "burned_diamond_bid", z2m(*supply.hacd_bid_burn_zhu),
        
        "channel_deposit", z2m(*supply.channel_deposit_zhu),
        "channel_interest", z2m(*supply.channel_interest_zhu),
        "channel_opening", *supply.opening_channel,
        
        "diamond_engraved", *supply.diamond_engraved,

        "transferred_bitcoin", 0,
        "trsbtc_subsidy", 0,

        "block_reward", z2m(blk_rwd),
        "minted_diamond", *lastdia.number,
    };
    api_data(data)
}