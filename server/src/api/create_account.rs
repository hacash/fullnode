

defineQueryObject!{ Q8936,
    quantity, Option<u64>, None,
}

async fn account(State(_ctx): State<ApiCtx>, q: Query<Q8936>) -> impl IntoResponse {
    q_must!(q, quantity, 1);
    if quantity == 0 {
        return api_error("quantity error")
    }
    if quantity > 200 {
        return api_error("quantity max 200")
    }
    let mut resbls = Vec::with_capacity(quantity as usize);
    for _ in 0..quantity {
        let acc = Account::create_randomly();
        if let Err(e) = acc {
            return api_error(&e)
        }
        let acc = acc.unwrap();
        resbls.push(json!({
            "address": acc.readable(),
            "prikey": hex::encode(&acc.secret_key().serialize()),
            "pubkey": hex::encode(&acc.public_key().serialize_compressed()),
        }));
    }

    // ok
    api_data_list(resbls)
}

