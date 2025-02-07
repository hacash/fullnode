use std::sync::*;
use std::thread::*;

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
#[allow(dead_code)]
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
    let (cltx, clrx) = mpsc::channel();
    ctrlc::set_handler(move||{ let _ = cltx.send(()); }).unwrap();

    let scaner = init_block_scaner(&iniobj, Some(scaner));

    // engine
    let dbv = HACASH_STATE_DB_UPDT;
    let engine = Arc::new(ChainEngine::open(&iniobj, dbv, minter, scaner));
    
    // node & server
    let hxnode = Arc::new(HacashNode::open(&iniobj, engine.clone()));
    let hnptr = hxnode.clone();
    let server = HttpServer::open(&iniobj, engine.clone(), hnptr.clone());

    // start all
    spawn(move||{ server.start() }); // start http server
    spawn(move||{ HacashNode::start(hnptr) }); // start p2p node

    // wait to ctrl+c to exit
    clrx.recv().unwrap();
    hxnode.exit(); // wait something
    engine.exit(); // wait something

    // all exit
    println!("\n[Exit] Hacash blockchain and P2P node have been closed.");

}




/*
* init block scaner
*/
fn init_block_scaner(inicnf: &IniObj, blkscaner: Option<Box<dyn Scaner>>) -> Arc<dyn Scaner> {

    // scaner
    let scaner: Arc<dyn Scaner> = match blkscaner {
        Some(mut scan) => {
            scan.init(inicnf).unwrap(); // init block scaner
            scan.into()
        },
        _ => Arc::new(EmptyBlockScaner{}),
    };

    // start block scaner
    let scanercp1 = scaner.clone();
    std::thread::spawn(move||{
        scanercp1.start().unwrap();
    });
    let scanercp2 = scaner.clone();
    std::thread::spawn(move||{
        scanercp2.serve().unwrap();
    });
    
    // ok
    scaner
}
