use std::path::*;
use std::sync::*;
use std::thread::*;


use super::*;

use sys::*;
use protocol::*;
use protocol::interface::*;



/***************************************/


pub type FnBuildDB = fn(_: &PathBuf)->Box<dyn DiskDB>;
pub type FnExtendApp = fn(_: Worker, _: Arc<dyn HNoder>);



/***************************************/



struct NilDB {}
impl DiskDB for NilDB {}
pub struct NilScaner {}
impl Scaner for NilScaner {}

struct NilTxPool {}
impl TxPool for NilTxPool {}

struct NilMinter {}
impl Minter for NilMinter {}

struct NilEngine {}
impl EngineRead for NilEngine {}
impl Engine for NilEngine {}

struct NilHNoder {}
impl HNoder for NilHNoder {}

struct NilServer {}
impl Server for NilServer {}


/***************************************/



#[allow(dead_code)]
pub struct Builder {
    cnfini: IniObj,
    datadir: PathBuf,
    engcnf: Arc<EngineConf>,
    nodcnf: Arc<NodeConf>,
    diskdb: FnBuildDB,
    extapp: Vec<FnExtendApp>,
    minter: Arc<dyn Minter>,
    engine: Arc<dyn Engine>,
    txpool: Arc<dyn TxPool>,
    scaner: Arc<dyn Scaner>,
    hnoder: Arc<dyn HNoder>,
    server: Arc<dyn Server>,
}

impl Builder {

    pub fn new(inipath: &str) -> Self {
        let cnfpath =  inipath.to_owned(); // "./hacash.config.ini".to_owned();
        let cnfini = load_config(cnfpath);
        let datadir = get_mainnet_data_dir(&cnfini);
        let engcnf = Arc::new(EngineConf::new(&cnfini, HACASH_STATE_DB_UPDT));
        let nodcnf = Arc::new(NodeConf::new(&cnfini));
        let build_nil_db: FnBuildDB = |_|Box::new(NilDB{});
        Self {
            cnfini,
            datadir,
            engcnf,
            nodcnf,
            diskdb: build_nil_db,
            extapp: Vec::new(),
            scaner: Arc::new(NilScaner{}),
            txpool: Arc::new(NilTxPool{}),
            minter: Arc::new(NilMinter{}),
            engine: Arc::new(NilEngine{}),
            hnoder: Arc::new(NilHNoder{}),
            server: Arc::new(NilServer{}),
        }
    }

    pub fn diskdb(&mut self, f: FnBuildDB) -> &mut Self {
        self.diskdb = f;
        self
    }

    pub fn txpool(&mut self, f: fn(_: &EngineConf)->Box<dyn TxPool>) -> &mut Self {
        self.txpool = f(&self.engcnf).into();
        self
    }
    
    pub fn minter(&mut self, f: fn(_: &IniObj)->Box<dyn Minter>) -> &mut Self {
        self.minter = f(&self.cnfini).into();
        self
    }
    
    pub fn scaner(&mut self, scn: Box<dyn Scaner>) -> &mut Self {
        self.scaner = scn.into();
        self
    }

    pub fn engine(&mut self, 
        f: fn(
            _: FnBuildDB,
            _: Arc<EngineConf>,
            _: Arc<dyn Minter>,
            _: Arc<dyn Scaner>
        )->Box<dyn Engine>
    ) -> &mut Self {
        self.engine = f(self.diskdb, self.engcnf.clone(), self.minter.clone(), self.scaner.clone()).into();
        self
    }

    pub fn hnoder(&mut self, 
        f: fn(
            _: &IniObj, _: Arc<dyn TxPool>, _: Arc<dyn Engine>
        )->Box<dyn HNoder>
    ) ->  &mut Self {
        self.hnoder = f(&self.cnfini, self.txpool.clone(), self.engine.clone()).into();
        self
    }

    pub fn server(&mut self, 
        f: fn(
            _: &IniObj, _: Arc<dyn HNoder>
        )->Box<dyn Server>
    ) ->  &mut Self {
        self.server = f(&self.cnfini, self.hnoder.clone()).into();
        self
    }
    

    pub fn app(&mut self, f: FnExtendApp) -> &mut Self {
        self.extapp.push(f);
        self
    }

    // do start all
    pub fn run(self) {

        run_fullnode(self)
    }

}





fn run_fullnode(builder: Builder) {

    let exiter = Exiter::new();
    let exitdo = exiter.clone();
    let _ = ctrlc::set_handler(move||{
        exitdo.exit();
    });

    // unpack
    // let _txpool = builder.txpool.clone();
    // let _scaner = builder.scaner.clone();
    // let _minter = builder.minter.clone();
    // let _engine = builder.engine.clone();
    let server  = builder.server.clone();
    let scaner1 = builder.scaner.clone();
    let scaner2 = builder.scaner.clone();
    let hnoder  = builder.hnoder.clone();


    // start server
    let wkr1 = exiter.work();
    spawn(move||{ server.start(wkr1) });

    // start scaner
    let wkr2 = exiter.work();
    spawn(move||{ scaner1.start(wkr2) });
    let wkr3 = exiter.work();
    spawn(move||{ scaner2.serve(wkr3) });

    // start node
    let wkr4 = exiter.work();
    spawn(move||{ hnoder.start(wkr4) });

    // run extend app
    for app in builder.extapp {
        let wkr = exiter.work();
        let hnoder = builder.hnoder.clone();
        spawn(move||{ app(wkr, hnoder) });
    }


    // start all and wait them exit
    exiter.wait();
    println!("[Exit] Hacash fullnode closed.");

}







/*
// fullnode main
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




// init block scaner
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

*/
