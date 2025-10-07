
use mint::HacashMinter;
use server::HttpServer;
use sys::*;
use app::{fullnode::NilScaner, *};
use app::fullnode::Builder;
use protocol::{interface::*, EngineConf};
use chain::engine::*;
use node::{memtxpool::*, node::HacashNode};


/*

* fullnode main
*/ 
#[allow(dead_code)]
fn main() {

    println!("[Version] full node v{}, build time: {}, database type: {}.", 
        HACASH_NODE_VERSION, HACASH_NODE_BUILD_TIME, HACASH_STATE_DB_UPDT
    );

    run();

}


pub fn run() {
    run_with_scaner("./hacash.config.ini", Box::new(NilScaner{}));
}


pub fn run_with_config(cnfpath: &str) {
    run_with_scaner(cnfpath, Box::new(NilScaner{}));
}


pub fn run_with_scaner(cnfpath: &str, scan: Box<dyn Scaner>) {

    // setup hook
    // mint hook
    protocol::block::setup_block_hasher( x16rs::block_hash );
    protocol::action::setup_extend_actions_try_create(1, mint::action::try_create);
    // vm hook
    #[cfg(feature = "tex")]
    {
        protocol::action::setup_extend_actions_try_create(1, protocol::tex::try_create);
    }
    // vm hook
    #[cfg(feature = "hvm")]
    {
        protocol::action::setup_extend_actions_try_create(3, vm::action::try_create);
        protocol::action::setup_action_hook(vm::hook::try_action_hook);
        server::extend::setup_extend_api_routes(vm::hook::extend_api_routes);
    }

    // build & setup
    let mut builder =  Builder::new(cnfpath);

    builder.diskdb(|dir|Box::new(db::DiskKV::open(dir)))
        .scaner(scan)
        .txpool(build_txpool)
        .minter(|ini|Box::new(HacashMinter::create(ini)))
        .engine(|dbfn, cnf, minter, scaner|Box::new(ChainEngine::open(dbfn, cnf, minter, scaner)))
        .hnoder(|ini, txpool, engine|Box::new(HacashNode::open(ini, txpool, engine)))
        .server(|ini, hnoder|Box::new(HttpServer::open(ini, hnoder)))
        .app(diabider::start_diamond_auto_bidding)
        ;

    // start run
    builder.run();
} 


fn build_txpool(engcnf: &EngineConf) -> Box<dyn TxPool> {
    let mut tpmaxs = maybe!(engcnf.miner_enable,
        vec![2000, 100], // miner node
        vec![10, 10]     // normal node
    );
    let fpmds  = vec![true, false]; // is sort by fee_purity, normal or diamint
    cover(&mut tpmaxs, &engcnf.txpool_maxs);
    Box::new(MemTxPool::new(
        engcnf.lowest_fee_purity, 
        tpmaxs, 
        fpmds
    ))
}
