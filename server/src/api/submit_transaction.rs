
defineQueryObject!{ Q4396,
    __nnn__, Option<bool>, None,
}

async fn submit_transaction(State(ctx): State<ApiCtx>, q: Query<Q4396>, body: Bytes) -> impl IntoResponse {
    // body bytes
    let bddts = q_body_data_may_hex!(q, body);
    // println!("get tx body: {}", hex::encode(&bddts));
    // parse
    let txpkg = TxPkg::build( bddts );
    if let Err(e) = txpkg {
        return api_error(&format!("transaction parse error: {}", &e))
    }
    let txpkg = txpkg.unwrap();
    // try submit
    let is_async = true;
    if let Err(e) = ctx.hcshnd.submit_transaction(&txpkg, is_async) {
        return api_error(&e)
    }
    // ok
    api_data(jsondata!{
        "hash", txpkg.hash.hex(),
    })
}
