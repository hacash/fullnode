use std::sync::*;

use sys::*;
use chain::{engine::ChainEngine, interface::*};
use node::node::*;


include!{"../version.rs"}


struct EmptyBlockScaner {}
impl Scaner for EmptyBlockScaner {}

/*
* fullnode main
*/ 
fn main() {
    
    let cnfp = "./hacash.config.ini".to_string();
    let inicnf = load_config(cnfp);

    println!("[Version] full node v{}, build time: {}, database type: {}.", 
        HACASH_NODE_VERSION, HACASH_NODE_BUILD_TIME, HACASH_STATE_DB_UPDT
    );

    fullnode(inicnf)
}


pub fn fullnode(iniobj: IniObj) {
    let empty_scaner = Box::new(EmptyBlockScaner{});
    fullnode_with_scaner(iniobj, empty_scaner)
}


pub fn fullnode_with_scaner(iniobj: IniObj, scaner: Box<dyn Scaner>) {
    let minter = Box::new(mint::HacashMinter::create(&iniobj));
    fullnode_with_minter_scaner(iniobj, minter, scaner)
}


pub fn fullnode_with_minter_scaner(iniobj: IniObj, 
    minter: Box<dyn Minter>,
    _scaner: Box<dyn Scaner>
) {

    // use std::sync::mpsc::channel;
    // let (cltx, clrx) = channel();
    // ctrlc::set_handler(move || cltx.send(()).unwrap()); // ctrl+c to quit

    // engine
    let dbv = HACASH_STATE_DB_UPDT;
    let engine = ChainEngine::open(&iniobj, dbv, minter);
    let engptr: Arc<dyn Engine> = Arc::new(engine);
    
    // node
    let _hnode = Arc::new(HacashNode::open(&iniobj, engptr.clone()));




    todo!()
}

