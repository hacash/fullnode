// use crate::mint::component::DiamondOwnedForm;


defineQueryObject!{ Q8364,
    address, String, s!(""),
    diamonds, Option<bool>, None,
}

async fn balance(State(ctx): State<ApiCtx>, q: Query<Q8364>) -> impl IntoResponse  {
    ctx_state!(ctx, state);
    ctx_mintstate!(ctx, mintstate);
    q_unit!(q, unit);
    let ads = q.address.replace(" ","").replace("\n","");
    let addrs: Vec<_> = ads.split(",").collect();
    let adrsz = addrs.len();
    if adrsz == 0 || (adrsz==1 && addrs[0].len()==0) {
        return api_error("address format error")
    }
    if adrsz > 200 {
        return api_error("address max 200")
    }
    let mut resbls = Vec::with_capacity(adrsz);
    for a in addrs {
        let adr = Address::from_readable(a);
        if let Err(e) = adr {
            return api_error(&format!("address {} format error: {}", a, e))
        }
        let adr = adr.unwrap();
        // balance
        let bls = state.balance(&adr).unwrap_or_default();
        let mut resj = json!({
            "hacash": bls.hacash.to_unit_string(&unit),
            "diamond": *bls.diamond,
            "satoshi": *bls.satoshi,
        });
        // dianames
        if let Some(true) = q.diamonds {
            let diaowned = mintstate.diamond_owned(&adr).unwrap_or_default();
            let dianames = diaowned.readable();
            resj["diamonds"] = dianames.into();
        }
        resbls.push(resj);
    }
    // ok
    api_data_list(resbls)
}