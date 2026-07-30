#[cfg(feature = "ocl")]
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use field::{Address, DiamondName, DiamondNumber, Encode, Fixed8, Hash};
use mint::action_diamond::{DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE, DiamondMint};
use mint::diamond_mining::{
    DiamondMiningResult, HASH_WIDTH, check_diamond_success, diamond_more_power,
};
use reqwest::blocking::Client as HttpClient;
use serde_json::Value;
use sys::Ret;

#[cfg(feature = "ocl")]
use mint::opencl::common;
#[cfg(feature = "ocl")]
use mint::opencl::dia;

const MINING_INTERVAL: f64 = 3.0;

static HTTP_CLIENT: LazyLock<HttpClient> = LazyLock::new(|| {
    HttpClient::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client build")
});

#[derive(Clone)]
struct DiaWorkConf {
    rpcaddr: String,
    threads: usize,
    bid_address: Address,
    reward_address: Address,
    use_opencl: bool,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    workgroups: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    localsize: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    unitsize: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    opencldir: String,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    platformid: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    deviceids: String,
}

#[derive(Clone)]
enum MinerBackend {
    Cpu,
    #[cfg(feature = "ocl")]
    Opencl {
        resource: Arc<common::OpenCLResources>,
        workgroups: u32,
        localsize: u32,
        unitsize: u32,
    },
}

static MINING_DIAMOND_NUM: AtomicU32 = AtomicU32::new(0);
static MINING_PREV_HASH: LazyLock<RwLock<Hash>> = LazyLock::new(|| RwLock::new(Hash::default()));

impl DiaWorkConf {
    fn load() -> Ret<Self> {
        let ini = sys::load_config("diaworker.config.ini")?;
        let sec = sys::ini_section(&ini, "default");
        let gpu = sys::ini_section(&ini, "gpu");
        Ok(Self {
            rpcaddr: sys::ini_must(sec, "connect", "127.0.0.1:8082"),
            threads: sys::ini_must_u64(sec, "supervene", 2).max(1) as usize,
            bid_address: Address::default(),
            reward_address: Address::default(),
            use_opencl: sys::ini_must_bool(gpu, "use_opencl", false),
            workgroups: sys::ini_must_u64(gpu, "work_groups", 1024) as u32,
            localsize: sys::ini_must_u64(gpu, "local_size", 256) as u32,
            unitsize: sys::ini_must_u64(gpu, "unit_size", 128) as u32,
            opencldir: sys::ini_must(gpu, "opencl_dir", "opencl/"),
            platformid: sys::ini_must_u64(gpu, "platform_id", 0) as u32,
            deviceids: sys::ini_must(gpu, "device_ids", ""),
        })
    }
}

pub fn run() -> Ret<()> {
    let mut conf = DiaWorkConf::load()?;
    load_init(&mut conf)?;
    let backends = build_miner_backends(&conf);
    let worker_count = backends.len().max(1);
    println!(
        "[diaworker] connect={} workers={} opencl={}",
        conf.rpcaddr, worker_count, conf.use_opencl
    );

    let (tx, rx) = mpsc::channel();
    let printer_conf = conf.clone();
    thread::spawn(move || result_loop(printer_conf, rx, worker_count));

    for (idx, backend) in backends.into_iter().enumerate() {
        let worker_conf = conf.clone();
        let worker_tx = tx.clone();
        thread::spawn(move || worker_loop(worker_conf, idx, backend, worker_tx));
    }

    loop {
        pull_next_diamond(&conf);
        thread::sleep(Duration::from_secs(MINING_INTERVAL as u64));
    }
}

fn build_miner_backends(conf: &DiaWorkConf) -> Vec<MinerBackend> {
    let mut backends = Vec::new();
    if conf.use_opencl {
        #[cfg(feature = "ocl")]
        {
            let resources = common::initialize_opencl(
                true,
                &conf.opencldir,
                conf.platformid,
                &conf.deviceids,
                conf.workgroups,
                conf.localsize,
                conf.unitsize,
            );
            backends.extend(resources.into_iter().map(|resource| MinerBackend::Opencl {
                resource: Arc::new(resource),
                workgroups: conf.workgroups,
                localsize: conf.localsize,
                unitsize: conf.unitsize,
            }));
        }
        #[cfg(not(feature = "ocl"))]
        {
            println!(
                "[diaworker] use_opencl=true but app was built without `ocl`; fallback to CPU"
            );
        }
    }
    if backends.is_empty() {
        backends.extend((0..conf.threads).map(|_| MinerBackend::Cpu));
    }
    backends
}

fn result_loop(conf: DiaWorkConf, rx: mpsc::Receiver<DiamondMiningResult>, worker_count: usize) {
    let mut best_seen = [b'W'; 16];
    loop {
        let mut count = 0usize;
        let mut total_nonce_space = 0u64;
        let mut total_elapsed = 0.0;
        let mut best_batch: Option<DiamondMiningResult> = None;
        while let Ok(result) = rx.try_recv() {
            total_nonce_space = total_nonce_space.saturating_add(result.nonce_space);
            total_elapsed += result.elapsed_secs;
            if best_batch.as_ref().map_or(true, |best| {
                diamond_more_power(&result.diamond_string, &best.diamond_string)
            }) {
                best_batch = Some(result.clone());
            }
            if let Some(success) = result.success {
                push_diamond_mining_success(&conf, success);
            }
            count += 1;
            if count >= worker_count * 4 {
                break;
            }
        }

        if let Some(best) = best_batch {
            if diamond_more_power(&best.diamond_string, &best_seen) {
                best_seen = best.diamond_string;
            }
            let secs = (total_elapsed / count.max(1) as f64).max(0.001);
            let rate = total_nonce_space as f64 / secs;
            println!(
                "[diaworker] number={} start={} scanned={} best={} global_best={} rate={}/s",
                best.number,
                best.nonce_start,
                total_nonce_space,
                String::from_utf8_lossy(&best.diamond_string),
                String::from_utf8_lossy(&best_seen),
                mint::difficulty::rates_to_show(rate)
            );
        }
        thread::sleep(Duration::from_millis(77));
    }
}

fn worker_loop(
    conf: DiaWorkConf,
    _worker_idx: usize,
    backend: MinerBackend,
    tx: mpsc::Sender<DiamondMiningResult>,
) {
    loop {
        let number = MINING_DIAMOND_NUM.load(Ordering::Relaxed);
        if number == 0 {
            thread::sleep(Duration::from_millis(99));
            continue;
        }
        let prev_hash = *MINING_PREV_HASH.read().unwrap();
        let custom_message = random_hash();
        let mut nonce_start = 0u64;
        let mut nonce_space = backend_nonce_space(&backend).max(1);

        loop {
            let started = Instant::now();
            let mut result = match &backend {
                MinerBackend::Cpu => do_diamond_group_mining(
                    number,
                    &prev_hash,
                    &conf.reward_address,
                    &custom_message,
                    nonce_start,
                    nonce_space,
                ),
                #[cfg(feature = "ocl")]
                MinerBackend::Opencl {
                    resource,
                    workgroups,
                    localsize,
                    unitsize,
                } => dia::do_diamond_group_mining_opencl(
                    resource,
                    number,
                    &prev_hash,
                    &conf.reward_address,
                    &custom_message,
                    nonce_start,
                    nonce_space,
                    *workgroups,
                    *localsize,
                    *unitsize,
                ),
            };
            result.elapsed_secs = started.elapsed().as_secs_f64();
            if tx.send(result).is_err() {
                return;
            }

            let Some(next_start) = nonce_start.checked_add(nonce_space) else {
                break;
            };
            nonce_start = next_start;
            if matches!(backend, MinerBackend::Cpu) {
                let secs = started.elapsed().as_secs_f64();
                if secs.is_finite() && secs > 0.0 {
                    nonce_space = ((nonce_space as f64 / secs) * MINING_INTERVAL) as u64;
                }
                nonce_space = nonce_space.max(1);
            }
            if number < MINING_DIAMOND_NUM.load(Ordering::Relaxed) {
                break;
            }
        }
    }
}

fn backend_nonce_space(backend: &MinerBackend) -> u64 {
    match backend {
        MinerBackend::Cpu => 15_000,
        #[cfg(feature = "ocl")]
        MinerBackend::Opencl {
            workgroups,
            localsize,
            unitsize,
            ..
        } => (*workgroups as u64)
            .saturating_mul(*localsize as u64)
            .saturating_mul(*unitsize as u64),
    }
}

fn random_hash() -> Hash {
    let mut bytes = [0u8; HASH_WIDTH];
    if let Err(e) = getrandom::fill(&mut bytes) {
        panic!("random nonce failed: {}", e);
    }
    Hash::from(bytes)
}

fn do_diamond_group_mining(
    number: u32,
    prev_hash: &Hash,
    reward_address: &Address,
    custom_message: &Hash,
    nonce_start: u64,
    nonce_space: u64,
) -> DiamondMiningResult {
    let empty = [0u8; 0];
    let custom_nonce = if number > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        custom_message.as_ref()
    } else {
        &empty
    };
    let mut best = DiamondMiningResult {
        number,
        nonce_start,
        nonce_space,
        nonce: 0,
        diamond_string: [b'W'; 16],
        success: None,
        elapsed_secs: 0.0,
    };
    let mut best_first_hash = [0u8; HASH_WIDTH];
    let mut best_result_hash = [0u8; HASH_WIDTH];
    let mut best_nonce_bytes = [0u8; 8];

    let nonce_end = nonce_start.saturating_add(nonce_space);
    for nonce in nonce_start..nonce_end {
        let nonce_bytes = nonce.to_be_bytes();
        let prev_hash_array = prev_hash.into_array();
        let (first_hash, result_hash, diamond_string) = x16rs::mine_diamond(
            number,
            &prev_hash_array,
            &nonce_bytes,
            reward_address.as_array(),
            custom_nonce,
        );
        if diamond_more_power(&diamond_string, &best.diamond_string) {
            best.nonce = nonce;
            best.diamond_string = diamond_string;
            best_first_hash = first_hash;
            best_result_hash = result_hash;
            best_nonce_bytes = nonce_bytes;
        }
    }

    if let Some(diamond_name) = check_diamond_success(
        number,
        best_first_hash,
        best_result_hash,
        best.diamond_string,
    ) {
        let mut act =
            DiamondMint::with(DiamondName::from(diamond_name), DiamondNumber::from(number));
        act.d.prev_hash = *prev_hash;
        act.d.nonce = Fixed8::from(best_nonce_bytes);
        act.d.address = *reward_address;
        act.d.custom_message = *custom_message;
        best.success = Some(act);
    }
    best
}

fn load_init(conf: &mut DiaWorkConf) -> Ret<()> {
    loop {
        let json = match get_json(&api_url(conf, "/query/diamondminer/init")) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[diaworker] init failed: {}", e);
                thread::sleep(Duration::from_secs(30));
                continue;
            }
        };
        if json["ret"].as_i64() != Some(0) {
            eprintln!(
                "[diaworker] init error: {}",
                json["err"].as_str().unwrap_or("unknown")
            );
            thread::sleep(Duration::from_secs(30));
            continue;
        }
        conf.bid_address = parse_address(json_str(&json, "bid_address")?)?;
        conf.reward_address = parse_address(json_str(&json, "reward_address")?)?;
        println!(
            "[diaworker] bid={} reward={}",
            conf.bid_address.to_readable(),
            conf.reward_address.to_readable()
        );
        return Ok(());
    }
}

fn pull_next_diamond(conf: &DiaWorkConf) {
    let mining_num = MINING_DIAMOND_NUM.load(Ordering::Relaxed);
    let latest = match get_json(&api_url(conf, "/query/latest")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[diaworker] latest failed: {}", e);
            return;
        }
    };
    let next_number = latest["diamond"].as_u64().unwrap_or(0) as u32 + 1;
    if next_number == 1 {
        *MINING_PREV_HASH.write().unwrap() = mint::genesis::genesis_block_hash();
        MINING_DIAMOND_NUM.store(next_number, Ordering::Relaxed);
        return;
    }
    if next_number <= mining_num {
        return;
    }

    let path = format!("/query/diamond?number={}", next_number - 1);
    let diamond = match get_json(&api_url(conf, &path)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[diaworker] diamond query failed: {}", e);
            return;
        }
    };
    let Some(prev_hash) = diamond["born"]["hash"].as_str().and_then(parse_hash_hex) else {
        eprintln!(
            "[diaworker] diamond born hash missing for {}",
            next_number - 1
        );
        return;
    };
    *MINING_PREV_HASH.write().unwrap() = prev_hash;
    MINING_DIAMOND_NUM.store(next_number, Ordering::Relaxed);
    println!("[diaworker] mining diamond number {}", next_number);
}

fn push_diamond_mining_success(conf: &DiaWorkConf, success: DiamondMint) {
    let url = api_url(conf, "/submit/diamondminer/success");
    let resp = HTTP_CLIENT
        .post(&url)
        .body(success.encode())
        .send()
        .and_then(|resp| resp.text());
    let Ok(text) = resp else {
        eprintln!("[diaworker] submit diamond failed");
        return;
    };
    let json = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    if json["ret"].as_i64() != Some(0) {
        eprintln!(
            "[diaworker] submit diamond error: {}",
            json["err"].as_str().unwrap_or("unknown")
        );
        return;
    }
    println!(
        "[diaworker] submitted diamond {} ({}) tx={}",
        success.d.diamond.to_readable(),
        success.d.number.uint(),
        json["tx_hash"].as_str().unwrap_or("")
    );
}

fn api_url(conf: &DiaWorkConf, path: &str) -> String {
    format!("http://{}{}", conf.rpcaddr, path)
}

fn get_json(url: &str) -> Ret<Value> {
    let text = HTTP_CLIENT
        .get(url)
        .send()
        .map_err(|e| sys::Error::fault(format!("request {} failed: {}", url, e)))?
        .text()
        .map_err(|e| sys::Error::fault(format!("read {} failed: {}", url, e)))?;
    serde_json::from_str::<Value>(&text)
        .map_err(|e| sys::Error::fault(format!("invalid json from {}: {}; body={}", url, e, text)))
}

fn json_str<'a>(json: &'a Value, key: &str) -> Ret<&'a str> {
    json[key]
        .as_str()
        .ok_or_else(|| sys::Error::fault(format!("missing json string field {}", key)))
}

fn parse_address(v: &str) -> Ret<Address> {
    Address::from_readable(v).map_err(|e| sys::Error::fault(format!("address invalid: {}", e)))
}

fn parse_hash_hex(v: &str) -> Option<Hash> {
    let bytes = hex::decode(v).ok()?;
    let arr: [u8; HASH_WIDTH] = bytes.as_slice().try_into().ok()?;
    Some(Hash::from(arr))
}
