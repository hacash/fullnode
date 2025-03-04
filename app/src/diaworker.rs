use std::sync::{RwLock, mpsc};
use std::sync::atomic::{AtomicU32, Ordering::*};

use std::time::*;
use std::thread::*;


use reqwest::blocking::Client as HttpClient;
use serde_json::Value as JV;



use sys::*;
use field::*;
use field::interface::*;
use protocol::action::*;
use protocol::difficulty::*;
use mint::genesis::*;



include!{"util.rs"}






/*************************************/





#[derive(Clone)]
pub struct DiaWorkConf {
    pub rpcaddr: String,
    pub supervene: u32, // cpu core
    pub bidaddr: Address,
    pub rewardaddr: Address,
}


impl DiaWorkConf {

    pub fn new(ini: &IniObj) -> DiaWorkConf {
        let sec = &ini_section(ini, "default"); // default = root
        let cnf = DiaWorkConf{
            rpcaddr: ini_must(sec, "connect", "127.0.0.1:8081"),
            supervene: ini_must_u64(sec, "supervene", 2) as u32,
            bidaddr: Address::default(),
            rewardaddr: Address::default(),
        };
        cnf
    }

}




/*************************************/





const HASH_WIDTH: usize = 32;
const MINING_INTERVAL: f64 = 3.0; // 3 secs


// current mining diamond number
static MINING_DIAMOND_NUM: AtomicU32 = AtomicU32::new(0);
lazy_static::lazy_static!{
    static ref MINING_DIAMOND_STUFF: RwLock<Hash> = RwLock::default();
}



#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct DiamondMiningResult {
    number: u32,
    nonce_start: u64,
    nonce_space: u64,
    u64_nonce: u64,
    msg_nonce: Vec<u8>,
    dia_str: [u8; 16],
    is_success: Option<DiamondMint>,
}



/*
* Diamond worker
*/
pub fn diaworker() {

    // config
    let cnfp = "./diaworker.config.ini".to_string();
    let inicnf = sys::load_config(cnfp);
    let mut cnf = DiaWorkConf::new(&inicnf);

    // test start
    // cnf.supervene = 1;
    // test end

    let (res_tx, res_rx) = mpsc::channel();

    // init
    load_init(&mut cnf);

    // deal results
    let cnf1 = cnf.clone();
    spawn(move || {
        let mut most_dia_str = [b'W'; 16];
        let mut rstx = res_rx;
        loop {
            deal_diamond_mining_results(&cnf1, &mut most_dia_str, &mut rstx);
            delay_continue_ms!(77);
        }
    });

    // start worker
    let thrnum = cnf.supervene as usize;
    println!("\n[Start] Create #{} diamond miner worker thread.", thrnum);
    for thrid in 0 .. thrnum {
        let cnf2 = cnf.clone();
        let rstx = res_tx.clone();
        spawn(move || {
            loop {
                run_diamond_worker_thread(&cnf2, thrid, rstx.clone());
                delay_continue_ms!(9);
            }
        });
    }

    // pull loop
    loop {
        pull_and_push_diamond(&cnf);
        delay_continue!(MINING_INTERVAL as u64);
    }
}




fn deal_diamond_mining_results(cnf: &DiaWorkConf, most_dia_str: &mut [u8; 16], 
    result_ch_rx: &mut mpsc::Receiver<DiamondMiningResult>,
) {
    let vene = cnf.supervene;
    let mut deal_number = 0u32;
    let mut most = DiamondMiningResult::default();
    most.dia_str = [b'w'; 16];
    let mut total_nonce_space = 0u64;
    for _ in 0 .. vene as usize {
        let res = result_ch_rx.recv().unwrap();
        deal_number = res.number;
        total_nonce_space += res.nonce_space as u64;
        if diamond_more_power(&res.dia_str, &most.dia_str) {
            most = res.clone();
        }
        // upload success
        if let Some(success) = &res.is_success {
            push_diamond_mining_success(cnf, success.clone());
        }
    }
    // total most
    if diamond_more_power(&most.dia_str, most_dia_str) {
        *most_dia_str = most.dia_str.clone();
    }
    // print hashrates
    let diastr = String::from_utf8(most.dia_str.to_vec()).unwrap();
    let most_diastr = String::from_utf8(most_dia_str.to_vec()).unwrap();
    let hsrts = rates_to_show(total_nonce_space as f64 / MINING_INTERVAL);
    flush!("{} {}, {} {}, {}.        \r", 
        most.nonce_start, total_nonce_space, 
        diastr, most_diastr, hsrts
    );

    // print next
    may_print_turn_to_nex_diamond_mining(deal_number, Some(most_dia_str));
}


fn may_print_turn_to_nex_diamond_mining(curr_number: u32, most_dia_str: Option<&mut [u8; 16]>) {
    let mining_number = MINING_DIAMOND_NUM.load(Relaxed);
    if mining_number <= curr_number {
        return // not turn
    }
    if let Some(most_dia_str) = most_dia_str {
        *most_dia_str = [b'W'; 16]; // reset 
    }

    println!("\n[{}] req next number {} to mining ... ", 
        &ctshow()[5..], mining_number
    );
}



// 
fn run_diamond_worker_thread(cnf: &DiaWorkConf, _thrid: usize,
    result_ch_tx: mpsc::Sender<DiamondMiningResult>,
) {
    let cmdn = MINING_DIAMOND_NUM.load(Relaxed);
    if cmdn == 0 {
        delay_return_ms!(99); // not yet
    }

    let rwd_addr = cnf.rewardaddr.clone();

    let mut nonce_space: u64 = 15000;
    let current_mining_number: u32 = cmdn;
    let current_mining_block_hash: Hash = {
        MINING_DIAMOND_STUFF.read().unwrap().clone()
    };

    // start mining
    let mut custom_nonce = Hash::default();
    getrandom::getrandom(custom_nonce.as_mut()).unwrap(); 
    let mut nonce_start = 0;

    loop {

        let ctn = Instant::now();
        // println!("- nonce_start: {}", nonce_start);
        let result = do_diamond_group_mining(current_mining_number, &current_mining_block_hash,
            &rwd_addr, &custom_nonce,
            nonce_start, nonce_space, 
        );
        // println!("do_diamond_group_mining: {:?}", &result);
        let use_secs = Instant::now().duration_since(ctn).as_millis() as f64 / 1000.0;
        result_ch_tx.send(result).unwrap(); // channel send
        let ns = nonce_start.checked_add(nonce_space);
        if let None = ns {
            break // u64 nonce end
        }
        nonce_start = ns.unwrap();
        nonce_space = ( nonce_space as f64 / use_secs * MINING_INTERVAL ) as u64;

        // check next
        if current_mining_number < MINING_DIAMOND_NUM.load(Relaxed) {
            return // turn to next number
        }

    }

}


fn do_diamond_group_mining(number: u32, prevblockhash: &Hash, 
    rwdaddr: &Address, custom_message: &Hash,
    nonce_start: u64, nonce_space: u64, 
) -> DiamondMiningResult {
    let empthbytes = [0u8; 0];
    let prevhash: &[u8; HASH_WIDTH] = prevblockhash;
    let address: &[u8; 21] = rwdaddr;
    let custom_nonce: &[u8] = match number > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        true => custom_message.as_bytes(),
        false => &empthbytes,
    };
    let mut most = DiamondMiningResult {
        number,
        nonce_start,
        nonce_space,
        u64_nonce: 0,
        msg_nonce: custom_nonce.to_vec(),
        dia_str: [b'W'; 16],
        is_success: None,
    };
    let mut most_firhx = [0u8; HASH_WIDTH];
    let mut most_resxh = [0u8; HASH_WIDTH];
    let mut most_diastr = [b'W'; 16];
    let mut most_noncebytes = [0u8; 8];

    // start mining
    for nonce in nonce_start .. nonce_start + nonce_space {
        // std::thread::sleep(std::time::Duration::from_micros(333)); // test
        let nonce_bytes = nonce.to_be_bytes();
        let (firhx, resxh, diastr) = x16rs::mine_diamond(number, prevhash, &nonce_bytes, address, custom_nonce);
        if diamond_more_power(&diastr, &most.dia_str) {
            most.u64_nonce = nonce;
            most.dia_str = diastr.clone();
            most_firhx = firhx;
            most_resxh = resxh;
            most_diastr = diastr;
            most_noncebytes = nonce_bytes;
        }
        // next
    }
    // check success
    if let Some(dia_name) = check_diamer_success(number, most_firhx, most_resxh, most_diastr) {
        let name = DiamondName::from(dia_name);
        let number = DiamondNumber::from(number);
        let mut diamint = DiamondMint::with(name, number);
        diamint.d.prev_hash = prevblockhash.clone();
        diamint.d.nonce = Fixed8::from(most_noncebytes);
        diamint.d.address = rwdaddr.clone();
        diamint.d.custom_message = custom_message.clone();
        most.is_success = Some(diamint); // mark success
    }
    // ok
    most
}


fn check_diamer_success(number: u32, firhx: [u8; HASH_WIDTH], resxh: [u8; HASH_WIDTH], diastr: [u8; 16]) -> Option<[u8; 6]> {
    if let None = x16rs::check_diamond_hash_result(&diastr) {
        return None
    }
    if ! x16rs::check_diamond_difficulty(number, &firhx, &resxh) {
        return None
    }
    // success find a diamond

    flush!("\n\n▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒\n");
    flush!("▒▒▒▒ MINING SUCCESS: {} ({})",  String::from_utf8(diastr.to_vec()).unwrap(), number);
    flush!("\n▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔\n");
    Some(diastr[10..].try_into().unwrap())
}



fn load_init(cnf: &mut DiaWorkConf) {

    let urlapi_pending = format!("http://{}/query/diamondminer/init", &cnf.rpcaddr);
    loop {
        let res = HttpClient::new().get(&urlapi_pending).send();
        let Ok(repv) = res else {
            println!("Error: cannot init diamond miner from {}", &urlapi_pending);
            delay_continue!(30);
        };
        let res: JV = serde_json::from_str(&repv.text().unwrap()).unwrap();
        let jstr = |k| { res[k].as_str().unwrap_or("") };
        let err = jstr("err");
        if err.len() > 0 {
            println!("{} Error: {}", &urlapi_pending, err);
            delay_continue!(30);
        }
        let adr1 = jstr("bid_address");
        let Ok(bid_addr) = Address::from_readable( &adr1 ) else {
            println!("Error: bid_address '{}' format error", &adr1);
            delay_continue!(30);
        };
        let adr2 = jstr("reward_address");
        let Ok(rwd_addr) = Address::from_readable( &adr2 ) else {
            println!("Error: reward_address '{}' format error", &adr2);
            delay_continue!(30);
        };
        println!("[Config] query diamond miner bid address: {}, reward address: {}", &adr1, &adr2);
        // ok
        cnf.bidaddr = bid_addr;
        cnf.rewardaddr = rwd_addr;
        break
    }
    // ok

}



fn pull_and_push_diamond(cnf: &DiaWorkConf) {

    let mining_num = MINING_DIAMOND_NUM.load(Relaxed);

    let urlapi_latest = format!("http://{}/query/latest", &cnf.rpcaddr);
    // get next number
    // println!("urlapi_latest: {}", &urlapi_latest);
    let res = HttpClient::new().get(&urlapi_latest).send();
    let Ok(repv) = res else {
        println!("Error: cannot get latest from {}", &urlapi_latest);
        delay_return!(30);
    };
    let res: JV = serde_json::from_str(&repv.text().unwrap()).unwrap();
    // println!("get latest: {:?}", &res);
    let jnum = |k| { res[k].as_u64().unwrap_or(0) };
    let next_num = jnum("diamond") as u32 + 1;
    // println!("mining next num: {} {}", &mining_num, &next_num);
    if next_num == 1 { 
        // println!("get latest: next_num == 1");
        *MINING_DIAMOND_STUFF.write().unwrap() = genesis_block_ptr().objc.hash();
        MINING_DIAMOND_NUM.store(next_num, Relaxed);
        return // first mining
    }
    if next_num <= mining_num {
        return // no change
    }
    // query next!
    let urlapi_diamond = format!("http://{}/query/diamond?number={}", &cnf.rpcaddr, next_num - 1);
    // println!("urlapi_diamond: {}", &urlapi_diamond);
    let res = HttpClient::new().get(&urlapi_diamond).send();
    let Ok(repv) = res else {
        println!("Error: cannot get diamond from {}", &urlapi_diamond);
        delay_return!(30);
    };
    let res: JV = serde_json::from_str(&repv.text().unwrap()).unwrap();
    // println!("query diamond: {:?}", &res);
    let prev_hash = res["born"]["hash"].as_str().unwrap_or("");
    let Ok(hx) = hex::decode(&prev_hash) else {
        println!("Error: cannot get born.hash from {}: {:?}", &urlapi_diamond, &res);
        delay_return!(30); // hash error
    };
    if hx.len() != HASH_WIDTH {
        delay_return!(30); // hash error
    }
    // change stuff
    *MINING_DIAMOND_STUFF.write().unwrap() = Hash::from(hx.try_into().unwrap());
    MINING_DIAMOND_NUM.store(next_num, Relaxed);
    // print first req msg
    if mining_num == 0 {
        may_print_turn_to_nex_diamond_mining(mining_num, None);
    }

}


fn push_diamond_mining_success(cnf: &DiaWorkConf, success: DiamondMint) {
    let urlapi_success = format!("http://{}/submit/diamondminer/success", &cnf.rpcaddr);
    spawn(move || {
        let actionbody = success.serialize();
        // println!("\n\ncurl {}?hexbody=true -X POST -d '{}'", &urlapi_success, &actionbody.hex());
        let res = HttpClient::new().post(&urlapi_success).body(actionbody).send();
        let Ok(repv) = res else {
            return // err
        };
        let res: JV = serde_json::from_str(&repv.text().unwrap()).unwrap();
        let jstr = |k: &str| { res[k].as_str().unwrap_or("") };
        let tx_err =  jstr("err");
        if tx_err.len() > 0 {
            println!("ㄨㄨㄨㄨ Failed submit tx diamond mint to mainnet\n     ERROR: {}\n", tx_err);
            return 
        }
        let tx_hash =  jstr("tx_hash");
        if tx_hash.len() != 64 {
            return // err
        }
        println!("Success submit tx diamond mint {} ({}) to mainnet, \n        get tx hash: {}\n", 
            success.d.diamond.to_readable(), *success.d.number, tx_hash
        );
    });
}
    




