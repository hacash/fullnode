use mint::MintConf;



async fn console(State(ctx): State<ApiCtx>, _req: Request) -> impl IntoResponse {
    ctx_store!(ctx, store);

    let mtcnf = ctx_mintcnf!(ctx);

    let latest = ctx.engine.latest_block();
    let lathei = latest.height().uint() as i64;
    let latts = latest.timestamp().uint();

    // target time
    let cyln = mtcnf.difficulty_adjust_blocks as i64;
    let secnp = ["day", "week", "month", "quarter", "year", "all"];
    let secn = [cyln, cyln*7, cyln*30, cyln*90, cyln*365, lathei-1];
    let mut target_time = Vec::with_capacity(secn.len());
    
    for i in 0..secn.len() {
        let sb = secn[i];
        let hei = lathei - sb;
        if hei <= 0 {
            break
        }
        let Some((_, blkdts)) = store.block_data_by_height(&BlockHeight::from(hei as u64)) else { 
            break
        };
        let mut bhd = BlockIntro::default();
        if let Err(..) = bhd.parse(&blkdts) {
            break
        }
        let blkt = bhd.timestamp().uint();
        target_time.push(format!(
            "{}: {}s", secnp[i], (latts-blkt)/(sb as u64),
        ));
    }

    let poworkers = { 
        let n = ctx.miner_worker_notice_count.lock().unwrap();
        *n
    };


    // render
    (html_headers(), format!(r#"<html><head><title>Hacash node console</title></head><body>
        <h3>Hacash console</h3>
        <p>Latest height {} time {}</p>
        <p>Block span times: {}</p>
        <p>P2P peers: {}</p>
        <p>{}</p>
        <p>Miner worker notice connected: {}</p>
    </body></html>"#, 
        latest.height().uint(),
        timeshow(latest.timestamp().uint()),
        target_time.join(", "),
        ctx.hcshnd.all_peer_prints().join(", "),
        ctx.hcshnd.txpool().print(),
        poworkers,
    ))
}

