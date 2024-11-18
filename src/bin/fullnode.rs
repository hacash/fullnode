use std::sync::*;

use sys::*;
use chain::{engine::ChainEngine, interface::*};
use node::node::*;
use server::*;


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
    let empty_scaner = Arc::new(EmptyBlockScaner{});
    fullnode_with_scaner(iniobj, empty_scaner)
}


pub fn fullnode_with_scaner(iniobj: IniObj, scaner: Arc<dyn Scaner>) {
    let minter = Box::new(mint::HacashMinter::create(&iniobj));
    fullnode_with_minter_scaner(iniobj, minter, scaner)
}


pub fn fullnode_with_minter_scaner(iniobj: IniObj, 
    minter: Box<dyn Minter>,
    scaner: Arc<dyn Scaner>
) {

    use std::sync::mpsc::channel;
    let (cltx, clrx) = channel();
    ctrlc::set_handler(move || cltx.send(()).unwrap()).unwrap(); // ctrl+c to quit

    // engine
    let dbv = HACASH_STATE_DB_UPDT;
    let engine = ChainEngine::open(&iniobj, dbv, minter);
    let engptr: Arc<dyn Engine> = Arc::new(engine);
    
    // node
    let hnode = Arc::new(HacashNode::open(&iniobj, engptr.clone()));

    // server
    let server = DataServer::open(&iniobj, engptr.clone(), hnode.clone());
    std::thread::spawn(move||{
        server.start(); // http rpc 
    });

    // handle ctr+c to close
    let hn2 = hnode.clone();
    std::thread::spawn(move||{ loop{
        let _ = clrx.recv();
        let _ = scaner.exit();
        hn2.close(); // ctrl+c to quit
    }});

    // start
    let _ = HacashNode::start(hnode);

    // on closed
    println!("\nHacash node closed.");


    todo!()
}

