use std::sync::{Arc, RwLock, mpsc};
use std::sync::atomic::{AtomicU64, Ordering::*};

use std::time::*;
use std::thread::*;


use reqwest::blocking::Client as HttpClient;
use serde_json::Value as JV;



use sys::*;
use field::*;
use field::interface::*;
use protocol::interface::*;
use protocol::block::*;
use protocol::transaction::*;
use protocol::difficulty::*;
use mint::genesis::*;



include!{"util.rs"}

#[cfg(feature = "ocl")]
include!{"opencl.rs"}



/*****************************************/


#[derive(Clone)]
pub struct PoWorkConf {
    pub rpcaddr: String,
    pub supervene: u32, // cpu core
    pub noncemax: u32,
    pub noticewait: u64, // new block notice wait
    pub useopencl: bool, // use opencl miner
    pub workgroups: u32, // opencl work groups
    pub localsize: u32, // opencl work units per work group
    pub unitsize: u32, // opencl hashes per work unit
    pub opencldir: String, // opencl source dir
    pub debug: u32, // enable debug mode
    pub platformid: u32, // opencl platform id
    pub deviceid: u32, // opencl device id
}



impl PoWorkConf {

    pub fn new(ini: &IniObj) -> PoWorkConf {
        let sec = &ini_section(ini, "default"); // default = root
        let sec_gpu = &ini_section(ini, "gpu");
        let cnf = PoWorkConf{
            rpcaddr: ini_must(sec, "connect", "127.0.0.1:8081"),
            supervene: ini_must_u64(sec, "supervene", 2) as u32,
            noncemax: ini_must_u64(sec, "nonce_max", u32::MAX as u64) as u32,
            noticewait: ini_must_u64(sec, "notice_wait", 45),
            useopencl: ini_must_bool(sec_gpu, "use_opencl", false) as bool,
            workgroups: ini_must_u64(sec_gpu, "work_groups", 1024) as u32,
            localsize: ini_must_u64(sec_gpu, "local_size", 256) as u32,
            unitsize: ini_must_u64(sec_gpu, "unit_size", 128) as u32,
            opencldir: ini_must(sec_gpu, "opencl_dir", "opencl/"),
            debug: ini_must_u64(sec_gpu, "debug", 0) as u32,
            platformid: ini_must_u64(sec_gpu, "platform_id", 0) as u32,
            deviceid: ini_must_u64(sec_gpu, "device_id", 0) as u32,
        };
        cnf
    }

}





/*****************************************/

const HASH_WIDTH: usize = 32;
const MINING_INTERVAL: f64 = 3.0; // 3 secs
const TARGET_BLOCK_TIME: f64 = 300.0; // 5 mins
const ONEDAY_BLOCK_NUM: f64 = 288.0; // one day block

// current mining diamond number
static MINING_BLOCK_HEIGHT: AtomicU64 = AtomicU64::new(0);

use std::sync::LazyLock;
static MINING_BLOCK_STUFF: LazyLock<RwLock<Arc<BlockMiningStuff>>> = LazyLock::new(|| RwLock::default() );



#[derive(Clone, Default)]
struct BlockMiningStuff {
    height: u64,
    target_hash: Hash,
    block_intro: BlockIntro,
    coinbase_tx: TransactionCoinbase,
    mkrl_list: Vec<Hash>,
}


#[derive(Clone, Default)]
struct BlockMiningResult {
    height: u64,
    nonce_start: u32,
    nonce_space: u32,
    head_nonce: u32,
    coinbase_nonce: Vec<u8>,
    result_hash: Vec<u8>,
    target_hash: Vec<u8>,
    use_secs: f64,
}

impl BlockMiningResult {
    fn new() -> BlockMiningResult {
        let mut res = BlockMiningResult::default();
        res.result_hash = vec![255u8; 32];
        res
    }
}





pub fn poworker() {

    // config
    let cnfp = "./poworker.config.ini".to_string();
    let inicnf = sys::load_config(cnfp);
    let cnf = PoWorkConf::new(&inicnf);

    // test start
    // cnfobj.supervene = 1;
    // cnfobj.noncemax = u32::MAX / 200;
    // cnfobj.noticewait = 5;
    // test end

    let (res_tx, res_rx) = mpsc::channel();

    // deal results
    let cnf1 = cnf.clone();
    spawn(move || {
        let mut most_hash = vec![255u8; 32];
        let mut rstx = res_rx;
        loop {
            deal_block_mining_results(&cnf1, &mut most_hash, &mut rstx);
            delay_continue_ms!(123);
        }
    });

    if cnf.useopencl { // opencl is enabled
        // Initialize OpenCL
        #[cfg(feature = "ocl")]
        {
            let opencl_resources = Arc::new(initialize_opencl(&cnf.clone()));
            println!("\n[Start] Create GPU block miner worker.");
            let cnf2 = cnf.clone();
            let rstx: mpsc::Sender<Arc<BlockMiningResult>> = res_tx.clone();
            let opencl_clone = Arc::clone(&opencl_resources);
            spawn(move || {
                loop {
                    run_block_mining_item(&cnf2, 0, rstx.clone(), Some(opencl_clone.clone()));
                    delay_continue_ms!(9);
                }
            });
        }
    } else {
        // start worker thread
        let thrnum = cnf.supervene as usize;
        println!("\n[Start] Create #{} block miner worker thread.", thrnum);
        for thrid in 0 .. thrnum {
            let cnf2 = cnf.clone();
            let rstx = res_tx.clone();
            spawn(move || {
                loop {
                    run_block_mining_item(&cnf2, thrid, rstx.clone(), None);
                    delay_continue_ms!(9);
                }
            });
        }
    }

    // loop
    loop {
        pull_pending_block_stuff(&cnf);
        delay_continue_ms!(25);
    }
}


#[cfg(not(feature = "ocl"))]
struct OpenCLResources {}


fn run_block_mining_item(_cnf: &PoWorkConf, _thrid: usize,
    result_ch_tx: mpsc::Sender<Arc<BlockMiningResult>>,
    _opencl: Option<Arc<OpenCLResources>>,
) {

    let mining_hei = MINING_BLOCK_HEIGHT.load(Relaxed);
    if mining_hei == 0 {
        delay_return_ms!(111); // not yet
    }

    let mut coinbase_nonce = Hash::default();
    getrandom::fill(coinbase_nonce.as_mut()).unwrap();
    let mut nonce_start: u32 = 0;
    let mut nonce_space: u32 = if !_cnf.useopencl {
        100000
    } else {
        _cnf.workgroups * _cnf.localsize * _cnf.unitsize
    };
    // stuff data
    let stuff = { MINING_BLOCK_STUFF.read().unwrap().clone() };
    let height = stuff.height;
    let mut cbtx = stuff.coinbase_tx.clone();
    cbtx.set_nonce(coinbase_nonce);
    let mut block_intro = stuff.block_intro.clone();
    block_intro.set_mrklroot( calculate_mrkl_coinbase_update(cbtx.hash(), &stuff.mkrl_list) );
    // nonce total space = u32
    let mut nonce_finish = false;
    loop {
        let ctn = Instant::now();

        #[cfg(not(feature = "ocl"))]
        let (head_nonce, result_hash) = do_group_block_mining(height, block_intro.serialize(), nonce_start, nonce_space);

        #[cfg(feature = "ocl")]
        let (head_nonce, result_hash) = if _cnf.useopencl {
            let _opencl = _opencl
                .as_ref()
                .expect("OpenCL miner is disabled");
            do_group_block_mining_opencl(
                &_opencl,
                height,
                block_intro.serialize(),
                nonce_start,
                _cnf.workgroups,
                _cnf.localsize,
                _cnf.unitsize,
            )
        } else {
            do_group_block_mining(height, block_intro.serialize(), nonce_start, nonce_space)
        };
        
        let use_secs = Instant::now().duration_since(ctn).as_millis() as f64 / 1000.0;
        // record result
        let mlres = BlockMiningResult {
            height,
            nonce_start,
            nonce_space,
            head_nonce,
            coinbase_nonce: coinbase_nonce.to_vec(),
            result_hash: result_hash.to_vec(),
            target_hash: stuff.target_hash.to_vec(),
            use_secs,
        };
        result_ch_tx.send(mlres.into()).unwrap();
        if nonce_finish {
            return // end u32 nonce
        }
        if !_cnf.useopencl { // nonce space is always the same in opencl
            // update space
            nonce_space = (nonce_space as f64 * MINING_INTERVAL / use_secs) as u32;
        }
        let Some(nst) = nonce_start.checked_add(nonce_space) else {
            nonce_finish = true;
            nonce_space = u32::MAX - nonce_start - 1;
            continue // // u32 nonce space finish
        };
        nonce_start = nst;
        // check next height
        let check_hei = MINING_BLOCK_HEIGHT.load(Relaxed);
        if check_hei > mining_hei {
            return // turn to next height
        }
        // continue nonce space
    }

}

// return: nonce, hash
fn do_group_block_mining(height: u64, mut block_intro: Vec<u8>, 
    nonce_start: u32, nonce_space: u32,
) -> (u32, [u8; 32]) {
    let mut most_nonce = 0u32;
    let mut most_hash = [255u8; 32];
    for nonce in nonce_start .. nonce_start + nonce_space {
        // std::thread::sleep(std::time::Duration::from_millis(1)); // test
        block_intro[79..83].copy_from_slice(&nonce.to_be_bytes());
        let reshx = x16rs::block_hash(height, &block_intro);
        if hash_more_power(&reshx, &most_hash) {
            most_hash = reshx;
            most_nonce = nonce;
        }
    }
    // end
    (most_nonce, most_hash)
}

fn deal_block_mining_results(cnf: &PoWorkConf, most_hash: &mut Vec<u8>,
    result_ch_rx: &mut mpsc::Receiver<Arc<BlockMiningResult>>,
) {
    let vene = if cnf.useopencl {
        1 // Single thread with GPU
    } else {
        cnf.supervene
    };
    // deal
    let mut deal_hei = 0u64;
    let mut most = Arc::new(BlockMiningResult::new());
    let mut total_nonce_space = 0u64;
    let mut total_use_secs = 0.0;
    for _ in 0 .. vene as usize {
        let res = result_ch_rx.recv().unwrap();
        deal_hei = res.height;
        total_nonce_space += res.nonce_space as u64;
        total_use_secs += res.use_secs; // Accumulated total time
        if hash_more_power(&res.result_hash, &most.result_hash) {
            most = res.clone();
        }
    }
    // total most
    if hash_more_power(&most.result_hash, most_hash) {
        *most_hash = most.result_hash.clone();
    }
    // print hashrates
    let tarhx: [u8; HASH_WIDTH] = most.target_hash.clone().try_into().unwrap();
    let target_rates = hash_to_rates(&tarhx, TARGET_BLOCK_TIME);
    let nonce_rates = if cnf.useopencl {
        total_nonce_space as f64 / (total_use_secs / vene as f64)
    } else {
        total_nonce_space as f64 / MINING_INTERVAL
    };
    let mut mnper = nonce_rates / target_rates;
    if mnper > 1.0 {
        mnper = 1.0;
    }
    let hac1day = mnper * ONEDAY_BLOCK_NUM * block_reward_number(deal_hei) as f64;
    flush!("{} {}, {} {}, ≈{:.4}HAC/day {:.6}%, {}.        \r", 
        most.nonce_start, total_nonce_space,
        hex::encode(hash_left_zero_pad(&most.result_hash, 2)), 
        hex::encode(hash_left_zero_pad3(&most_hash)), 
        hac1day, mnper * 100.0, rates_to_show(nonce_rates)
    );
    // check success
    if cnf.debug == 1 || hash_more_power(&most.result_hash, &most.target_hash) {
        push_block_mining_success(cnf, &most);
    }
    // print next height
    may_print_turn_to_nex_block_mining(deal_hei, Some(most_hash));
}


fn may_print_turn_to_nex_block_mining(curr_hei: u64, most_hash: Option<&mut Vec<u8>>) {
    let mining_hei = MINING_BLOCK_HEIGHT.load(Relaxed);
    if curr_hei >= mining_hei {
        return // not turn
    }
    if let Some(most_hash) = most_hash {
        *most_hash = vec![255u8; 32]; // reset 
    }
    let stuff = MINING_BLOCK_STUFF.read().unwrap();
    let tarhx = hash_left_zero_pad3(&stuff.target_hash.as_bytes()).hex();

    println!("\n[{}] req height {} target {} to mining ... ", 
        &ctshow()[5..], mining_hei, tarhx
    );
}


fn set_pending_block_stuff(height: u64, res: serde_json::Value) {
    let jstr = |k: &str| { res[k].as_str().unwrap_or("") };
    let _jnum = |k: &str| { res[k].as_u64().unwrap_or(0) };
    // data
    // println!("{:?}", &res);
    let target_hash = Hash::from(hex::decode(jstr("target_hash")).unwrap().try_into().unwrap());
    let block_intro = BlockIntro::must(&hex::decode(jstr("block_intro")).unwrap());
    let coinbase_tx = TransactionCoinbase::must(&hex::decode(jstr("coinbase_body")).unwrap());
    let mut mkrl_list = Vec::new();
    if let JV::Array(ref lists) = res["mkrl_modify_list"] {
        for li in lists {
            mkrl_list.push(Hash::from(hex::decode(li.as_str().unwrap_or("")).unwrap().try_into().unwrap()));
        }
    }
    // set pending stuff
    let new_stuff = BlockMiningStuff{
        height,
        target_hash,
        block_intro,
        coinbase_tx,
        mkrl_list,
    };
    *MINING_BLOCK_STUFF.write().unwrap() = new_stuff.into();
    MINING_BLOCK_HEIGHT.store(height, Relaxed);
}



///////////////////////////////




fn pull_pending_block_stuff(cnf: &PoWorkConf) {

    let curr_hei = MINING_BLOCK_HEIGHT.load(Relaxed);

    // query pending
    let urlapi_pending = format!("http://{}/query/miner/pending?stuff=true&t={}", &cnf.rpcaddr, sys::curtimes());
    let res = HttpClient::new().get(&urlapi_pending).send();
    let Ok(repv) = res else {
        println!("Error: cannot get block data at {}\n", &urlapi_pending);
        delay_return!(30);
    };
    let res: JV = serde_json::from_str(&repv.text().unwrap()).unwrap();
    let jstr = |k| { res[k].as_str().unwrap_or("") };
    let jnum = |k| { res[k].as_u64().unwrap_or(0) };
    let JV::String(ref _blkhd) = res["block_intro"] else {
        println!("Error: get block stuff error: {}", jstr("err") );
        delay_return!(15);
    };
    let pending_height = jnum("height");

    // set pending block stuff
    if pending_height > curr_hei {
        set_pending_block_stuff(pending_height, res);
        if curr_hei == 0 {
            may_print_turn_to_nex_block_mining(curr_hei, None); // print first
        }
    }

    // with notice
    let mut rpid = vec![0].repeat(16);
    loop {

        getrandom::fill(&mut rpid).unwrap();
        let urlapi_notice = format!("http://{}/query/miner/notice?wait={}&height={}&rqid={}", 
            &cnf.rpcaddr, &cnf.noticewait, pending_height, &hex::encode(&rpid)
        );
        // println!("\n-------- {} -------- {}\n", &ctshow(), &urlapi_notice);
        let res = HttpClient::new().get(&urlapi_notice).timeout(Duration::from_secs(300)).send();
        let Ok(repv) = res else {
            println!("Error: cannot get miner notice at {}\n", &urlapi_notice);
            delay_return!(10);
        };
        let Ok(jsdata) = repv.text() else {
            println!("Error: cannot read miner notice at {}", &urlapi_notice);
            delay_return!(1);
        };
        let Ok(res2) = serde_json::from_str::<JV>(&jsdata) else {
            // println!("{}", &jsdata);
            panic!("miner notice error: {}", &jsdata);
        };
        let jnum = |k| { res2[k].as_u64().unwrap_or(0) };
        let res_hei = jnum("height");
        // println!("\n++++++++ {} {} {}\n", &jsdata, res_hei, current_height);
        if res_hei >= pending_height {
            // next block discover
            break 
        }
        // continue to wait

    }

}


fn push_block_mining_success(cnf: &PoWorkConf, success: &BlockMiningResult) {
    let urlapi_success = format!(
        "http://{}/submit/miner/success?height={}&block_nonce={}&coinbase_nonce={}&t={}", 
        &cnf.rpcaddr, success.height, success.head_nonce, success.coinbase_nonce.hex(), sys::curtimes()
    );
    let _ = HttpClient::new().get(&urlapi_success).send();
    println!("{} {}", &urlapi_success, HttpClient::new().get(&urlapi_success).send().unwrap().text().unwrap());
    // print
    println!("\n\n████████████████ [MINING SUCCESS] Find a block height {},\n██ hash {} to submit.",
        success.height, success.result_hash.hex()
    );
    println!("▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔")
}


