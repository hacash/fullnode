

defineQueryObject!{ Q4376,
    __nnn_, Option<bool>, None,
}

async fn latest(State(ctx): State<ApiCtx>, _q: Query<Q4376>) -> impl IntoResponse {
    ctx_state!(ctx, state);
    //
    let lasthei = ctx.engine.latest_block().height().uint();
    let lastdia = state.get_latest_diamond();
    // return data
    let data = jsondata!{
        "height", lasthei,
        "diamond", *lastdia.number,
    };
    api_data(data)

}

