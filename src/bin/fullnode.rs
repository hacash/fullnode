use std::sync::*;

use sys::*;
use chain::{engine::ChainEngine, interface::*};
use node::interface::*;
use node::node::*;
use server::*;


include!{"../version.rs"}


struct EmptyBlockScaner {}
impl Scaner for EmptyBlockScaner {}

/*
* fullnode main
*/ 
fn main() {
    fullnode()
}


/*
* fullnode main
*/ 
pub fn fullnode() {
    
    let cnfp = "./hacash.config.ini".to_string();
    let inicnf = load_config(cnfp);

    println!("[Version] full node v{}, build time: {}, database type: {}.", 
        HACASH_NODE_VERSION, HACASH_NODE_BUILD_TIME, HACASH_STATE_DB_UPDT
    );

    fullnode_with_ini(inicnf)
}


pub fn fullnode_with_ini(iniobj: IniObj) {
    let empty_scaner = Box::new(EmptyBlockScaner{});
    fullnode_with_scaner(iniobj, empty_scaner)
}


pub fn fullnode_with_scaner(iniobj: IniObj, scaner: Box<dyn Scaner>) {
    let minter = Box::new(mint::HacashMinter::create(&iniobj));
    fullnode_with_minter_scaner(iniobj, minter, scaner)
}


pub fn fullnode_with_minter_scaner(iniobj: IniObj, 
    minter: Box<dyn Minter>,
    scaner: Box<dyn Scaner>
) {

    // setup ctrl+c to quit
    let (cltx, clrx) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || cltx.send(()).unwrap()).unwrap();

    // engine
    let dbv = HACASH_STATE_DB_UPDT;
    let engine: Arc<dyn Engine> = Arc::new(ChainEngine::open(&iniobj, dbv, minter, scaner));
    
    // node & server
    let hanode = Arc::new(HacashNode::open(&iniobj, engine.clone()));
    let server = HttpServer::open(&iniobj, engine.clone(), hanode.clone());

    // start all
    let hn = hanode.clone();
    std::thread::spawn(move||{
        HacashNode::start(hn).unwrap();
    });
    std::thread::spawn(move||{
        server.start(); // http rpc 
    });

    // wait to ctrl+c to quit
    clrx.recv().unwrap();
    engine.exit();
    hanode.exit();

    // on closed
    println!("\nHacash node closed.");
}

