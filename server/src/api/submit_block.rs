

api_querys_define!{ Q3735,
    __nnn__, Option<bool>, None,
}

async fn submit_block(State(ctx): State<ApiCtx>, q: Query<Q3735>, body: Bytes) -> impl IntoResponse {
    // body bytes
    let bddts = q_body_data_may_hex!(q, body);
    // println!("get block body: {}", hex::encode(&bddts));
    // parse
    let blkpkg = BlockPkg::build(bddts);
    if let Err(e) = blkpkg {
        return api_error(&format!("block parse error: {}", &e))
    }
    let blkpkg = blkpkg.unwrap();
    // try submit
    let is_async = true;
    if let Err(e) = ctx.hcshnd.submit_block(&blkpkg, is_async) {
        return api_error(&format!("submit block error: {}", &e))
    }
    // ok
    api_ok()
}