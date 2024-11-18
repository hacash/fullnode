


////////////////// test //////////////////




defineQueryObject!{ Q86489,
    name, Option<String>, None,
}

async fn testapi1234563847653475(State(_ctx): State<ApiCtx>, _q: Query<Q86489>) -> impl IntoResponse {

    let data = jsondata!{
        "test", 1,
    };
    api_data(data)
}

