use std::sync::Arc;

use axum::{
    extract::{Query, State}, 
    response::IntoResponse,
    routing::get,
    Router,
};

use protocol::block::create_tx_info;
use serde_json::json;

use server::*;
use server::ctx::*;

use super::*;
use super::ContractAddress;



////////////////// test //////////////////




api_querys_define!{ Q8365,
    contract, String, s!(""),
    funcname, String, s!(""),
    paramhex, Option<String>, None,
    rtvabi, Option<String>, None, // U1 U2 .. U16, S1, S2, S3 ... S32, STR, B1. .. B32, BUF  [a:U1,b:U3,C:BUF]

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
    let mut ctxobj = ContextInst::new(env, substa, &tx);

    // call contract
    let Ok(addr) = Address::from_readable(&q.contract) else {
        return api_error("contract address format error")
    };
    let Ok(ctrladdr) = ContractAddress::from_addr(addr) else {
        return api_error("contract address version error")
    };
    let param = hex::decode(q.paramhex.clone().unwrap_or(s!(""))).unwrap_or(vec![]);
    let callres = machine::sandbox_call(&mut ctxobj, ctrladdr, q.funcname.clone(), param);
    if let Err(e) = callres {
        return api_error(&format!("contract call error: {}", e))
    }
    let (gasuse, retval) = callres.unwrap();

    // return
    let data = jsondata!{
        "gasuse", gasuse,
        "return", retval.hex(),
    };
    api_data(data)
}






pub fn extend_api_routes() -> Router<ApiCtx> {

    Router::new().route(&query("contract/sandboxcall"), get(contract_sandbox_call))

}