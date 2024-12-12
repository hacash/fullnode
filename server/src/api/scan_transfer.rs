
defineQueryObject!{ Q4538,
    height, u64, 1,
    txposi, isize, -1,
    filter_from, Option<String>, None,
    filter_to, Option<String>, None,
}

async fn scan_coin_transfer(State(ctx): State<ApiCtx>, q: Query<Q4538>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    q_unit!(q, unit);
    q_coinkind!(q, coinkind);
    let blkpkg = ctx.load_block(&store, &q.height.to_string());
    if let Err(e) = blkpkg {
        return  api_error(&e)
    }
    let blkobj = blkpkg.unwrap();
    let blkobj = &blkobj.objc;
    let trs = blkobj.transactions();
    if trs.len() == 0 {
        return api_error("transaction len error")
    }
    if q.txposi < 0 {
        return api_error("txposi error")
    }
    let txposi = q.txposi as usize;
    let trs = &trs[1..];
    if txposi >= trs.len() {
        return api_error("txposi overflow")
    }
    let tartrs = trs[txposi].as_read();
    let mainaddr_readable = tartrs.main().readable();
    let mut dtlist = Vec::new();
    // scan actions
    for act in tartrs.actions()  {
        append_transfer_scan(tartrs, act.as_ref(), &mut dtlist, &unit, &coinkind );
    }
    // ok
    let data = jsondata!{
        "tx_hash", tartrs.hash().hex(),
        "tx_timestamp", tartrs.timestamp().uint(),
        "block_hash", blkobj.hash().hex(),
        "block_timestamp", blkobj.timestamp().uint(),
        "main_address", mainaddr_readable,
        "transfers", dtlist,
    };
    api_data(data)
}



fn append_transfer_scan(tx: &dyn TransactionRead, act: &dyn Action, 
    transfers: &mut Vec<JsonObject>, unit: &String, ck: &CoinKind,
) {
    let trace = match act.kind() {

        HacToTransfer::KIND |
        HacFromTransfer::KIND |
        HacFromToTransfer::KIND => ck.hacash,

        DiamondSingleTransfer::KIND |
        DiamondFromToTransfer::KIND |
        DiamondToTransfer::KIND |
        DiamondFromTransfer::KIND => ck.diamond,

        SatoshiToTransfer::KIND |
        SatoshiFromTransfer::KIND |
        SatoshiFromToTransfer::KIND => ck.satoshi,

        _ => false,
    };

    // append
    if trace {
        transfers.push( action_to_json_desc(tx, act, unit, false, false) );
    }
}
