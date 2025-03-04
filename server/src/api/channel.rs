

/******************* channel *******************/



api_querys_define!{ Q7542,
    id, Option<String>, None,
}

async fn channel(State(ctx): State<ApiCtx>, q: Query<Q7542>) -> impl IntoResponse {
    ctx_mint_state!(ctx, state);
    q_unit!(q, unit);
    q_must!(q, id, s!(""));
    // id

    let Ok(id) = hex::decode(&id) else {
        return api_error("channel id format error")
    };
    if id.len() != ChannelId::SIZE {
        return api_error("channel id format error")
    }
    let chid = ChannelId::must(&id);
    let Some(channel) = state.channel(&chid) else {
        return api_error("channel not find")
    };

    // return data
    let status = *channel.status;
    let mut data = jsondata!{
        "id", chid.to_hex(),
        "status", status,
        "open_height", *channel.open_height,
        "reuse_version", *channel.reuse_version,
        "arbitration_lock", *channel.arbitration_lock_block,
        "interest_attribution", *channel.interest_attribution,
        "left", json!(jsondata!{
            "address", channel.left_bill.address.readable(),
            "hacash", channel.left_bill.hacsat.amount.to_unit_string(&unit),
            "satoshi", *channel.left_bill.hacsat.satoshi.value(),
        }),
        "right", json!(jsondata!{
            "address", channel.right_bill.address.readable(),
            "hacash", channel.right_bill.hacsat.amount.to_unit_string(&unit),
            "satoshi", *channel.right_bill.hacsat.satoshi.value(),
        }),
    };

    // if status == 1 // closed  status == 2 || status == 3 
    if let Some(challenging) = channel.if_challenging.if_value() {
        let l_or_r = challenging.assert_address_is_left_or_right.check();
        let assaddr = match l_or_r {
            true => channel.left_bill.address.readable(),
            false => channel.right_bill.address.readable(),
        };
        data.insert("challenging", json!(jsondata!{
            "launch_height", *challenging.challenge_launch_height,
            "assert_bill_auto_number", *challenging.assert_bill_auto_number,
            "assert_address_is_left_or_right", l_or_r,
            "assert_bill", json!(jsondata!{
                "address", assaddr,
                "hacash", challenging.assert_bill.amount.to_unit_string(&unit),
                "satoshi", challenging.assert_bill.satoshi.value().uint(),
            }),
        }));
    }

    // if status == 2 or 3 // closed  status == 2 || status == 3 
    if let Some(distribution) = channel.if_distribution.if_value() {
        data.insert("distribution", json!(jsondata!{
            "hacash", distribution.left_bill.amount.to_unit_string(&unit),
            "satoshi", distribution.left_bill.satoshi.value().uint(),
        }));
    }


    api_data(data)
}
