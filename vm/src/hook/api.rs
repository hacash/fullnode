use std::sync::Arc;

use axum::{
    extract::{Query, State}, 
    response::IntoResponse,
    routing::get,
    Router,
};

use protocol::block::create_tx_info;
// use serde_json::json;

use server::*;
use server::ctx::*;

use super::*;
use super::ContractAddress;



////////////////// contract logs read & del//////////////////



api_querys_define!{ Q9264,
    height,  u64, 0,
    index,   usize, 0,
    address, Option<String>, None,
    topic0,  Option<String>, None,
    topic1,  Option<String>, None,
    topic2,  Option<String>, None,
    topic3,  Option<String>, None,
}

async fn vm_logs_read(State(ctx): State<ApiCtx>, q: Query<Q9264>) -> impl IntoResponse {

    let ck_hei = q.height;
    let rc_hei = ctx.engine.latest_block().height().uint() - ctx.engine.config().unstable_block;
    if ck_hei > rc_hei {
        return api_data_raw(s!(r#""unstable":true"#))
    }
    // find logs
    let logs = ctx.engine.logs();
    let Some(itdts) = logs.load(ck_hei, q.index) else {
        return api_data_raw(s!(r#""end":true"#))
    };
    let Ok(item) = VmLog::build(&itdts).map_ire(LogError) else {
        return api_error("log format error")
    };
    let ignore = api_data_raw(s!(r#""ignore":true"#));
    // filter address
    if let Some(qadr) = &q.address {
        if q_addr!(qadr) != item.addr {
            return ignore
        }
    }
    // filter topic
    macro_rules! filter_topic { ($k: ident) => {
        if let Some(tp) = &q.$k {
            if q_hex!(tp) != item.$k.raw() {
                return ignore
            }
        }
    }}

    filter_topic!{ topic0 }
    filter_topic!{ topic1 }
    filter_topic!{ topic2 }
    filter_topic!{ topic3 }

    // ok
    let res = item.render("");
    api_data_raw(res)

}

api_querys_define!{ Q8375,
    height,  u64, 0,
}

async fn vm_logs_del(State(ctx): State<ApiCtx>, q: Query<Q8375>) -> impl IntoResponse {
    let logs = ctx.engine.logs();
    logs.remove(q.height);
    // return
    let data = r##""ok":true""##.to_owned();
    api_data_raw(data)

}




////////////////// contract sandbox call //////////////////




api_querys_define!{ Q8365,
    contract, String, s!(""),
    function, String, s!(""),
    params,   Option<String>, None,
    rtvabi,   Option<String>, None, // U1 U2 .. U16, S1, S2, S3 ... S32, STR, B1. .. B32, BUF  [a:U1,b:U3,C:BUF]

}

async fn contract_sandbox_call(State(ctx): State<ApiCtx>, q: Query<Q8365>) -> impl IntoResponse {
    use field::*;
    use protocol::context::*;
    use protocol::transaction::*;

    let height = ctx.engine.latest_block().height().uint() + 1; // next height
    let engcnf = ctx.engine.config();
    let staptr = ctx.engine.state();
    let substa = staptr.fork_sub(Arc::downgrade(&staptr));
    let tx = TransactionType3::default();

    // ctx
    let env = Env {
        chain: ChainInfo {
            id: engcnf.chain_id,
            diamond_form: false,
            fast_sync: false,
        },
        block: BlkInfo {
            height,
            hash: Hash::default(),
            coinbase: Address::default(),
        },
        tx: create_tx_info(&tx),
    };
    let mut ctxobj = ContextInst::new(env, substa, Box::new(EmptyLogs{}) ,&tx);

    // call contract
    let Ok(addr) = Address::from_readable(&q.contract) else {
        return api_error("contract address format error")
    };
    let Ok(ctrladdr) = ContractAddress::from_addr(addr) else {
        return api_error("contract address version error")
    };
    let pmempty = s!("");
    let params = q.params.as_ref().unwrap_or(&pmempty);
    let callres = machine::sandbox_call(&mut ctxobj, ctrladdr, q.function.clone(), params);
    if let Err(e) = callres {
        return api_error(&format!("contract call error: {}", e))
    }
    let (gasuse, retval) = callres.unwrap();

    // return
    let data = format!(r##""gasuse":{},"return":{}"##, gasuse, retval);
    api_data_raw(data)
}





////////////////// api routes //////////////////




pub fn extend_api_routes() -> Router<ApiCtx> {

    Router::new()
        .route(&query("contract/sandboxcall"), get(contract_sandbox_call))
        .route(&query("contract/logs"), get(vm_logs_read))
        .route(&operate("contract/logs/delete"), get(vm_logs_del))

}