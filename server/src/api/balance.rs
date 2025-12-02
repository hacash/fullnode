// use crate::mint::component::DiamondOwnedForm;


api_querys_define!{ Q8364,
    address, String, s!(""),
    diamonds, Option<bool>, None,
    asset, Option<String>, None,
    assets, Option<bool>, None,
}

async fn balance(State(ctx): State<ApiCtx>, q: Query<Q8364>) -> impl IntoResponse  {
    ctx_state!(ctx, state);
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
        if let Some(true) = &q.diamonds {
            let diaowned = state.diamond_owned(&adr).unwrap_or_default();
            let dianames = diaowned.readable();
            resj["diamonds"] = dianames.into();
        }
        // asset
        if let Some(ast) = &q.asset {
            let mut astlist = Vec::new();
            let mut astptr = &astlist;
            match ast.parse::<u64>() {
                Ok(astn) => {
                    if let Some(ast) = bls.asset(Fold64::from(astn).unwrap_or(Fold64::max())) {
                        astlist.push(ast);
                        astptr = &astlist;
                    }
                },
                _ => astptr = bls.assets.list()
            };
            let mut assets = vec![];
            for a in astptr {
                assets.push(json!({
                    "serial": *a.serial,
                    "amount": *a.amount,
                }));
            }
            resj["assets"] = assets.into();
        }
        // asset
        if let Some(true) = &q.assets {
            let mut assets = vec![];
            for a in bls.assets.list() {
                assets.push(json!({
                    "serial": *a.serial,
                    "amount": *a.amount,
                }));
            }
            resj["assets"] = assets.into();
        }
        resbls.push(resj);
    }
    // ok
    api_data_list(resbls)
}