use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::{Duration, Instant};

use base::{BlockBuild, TransactionBuild, TransactionSign};
use field::Hash;
use mint::diamond_mining::HASH_WIDTH;
use mint::difficulty::{hash_to_rates, rates_to_show};
use mint::minter::block_reward_number;
use mint::tx_coinbase::CoinbaseTx;
use protocol::block_std::{StdBlock, calculate_mrkl_prelude_update};
use reqwest::blocking::Client as HttpClient;
use serde_json::Value;
use sys::{Ret, ToHex};

#[cfg(feature = "ocl")]
use mint::opencl::common;
#[cfg(feature = "ocl")]
use mint::opencl::pow;

const MINING_INTERVAL: f64 = 3.0;
const TARGET_BLOCK_TIME: f64 = 300.0;
const ONEDAY_BLOCK_NUM: f64 = 288.0;

static HTTP_CLIENT: LazyLock<HttpClient> = LazyLock::new(|| {
    HttpClient::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client build")
});

static HTTP_CLIENT_NOTICE: LazyLock<HttpClient> = LazyLock::new(|| {
    HttpClient::builder()
        .no_proxy()
        .timeout(Duration::from_secs(300))
        .build()
        .expect("http notice client build")
});

#[derive(Clone)]
pub struct PoWorkConf {
    pub rpcaddr: String,
    pub threads: usize,
    pub nonce_max: u32,
    pub nonce_chunk: u32,
    pub notice_wait: u64,
    pub debug: bool,
    pub use_opencl: bool,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    pub workgroups: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    pub localsize: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    pub unitsize: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    pub opencldir: String,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    pub platformid: u32,
    #[cfg_attr(not(feature = "ocl"), allow(dead_code))]
    pub deviceids: String,
}

impl PoWorkConf {
    pub fn load() -> Ret<Self> {
        let ini = sys::load_config("poworker.config.ini")?;
        let sec = sys::ini_section(&ini, "default");
        let gpu = sys::ini_section(&ini, "gpu");
        // Prefer [default] debug=bool; fall back to legacy [gpu] debug=1.
        let debug =
            sys::ini_must_bool(sec, "debug", false) || sys::ini_must_u64(gpu, "debug", 0) == 1;
        Ok(Self {
            rpcaddr: sys::ini_must(sec, "connect", "127.0.0.1:8082"),
            threads: sys::ini_must_u64(sec, "supervene", 2).max(1) as usize,
            nonce_max: sys::ini_must_u64(sec, "nonce_max", u32::MAX as u64) as u32,
            nonce_chunk: sys::ini_must_u64(sec, "nonce_chunk", 100_000).max(1) as u32,
            notice_wait: sys::ini_must_u64(sec, "notice_wait", 45).clamp(1, 300),
            debug,
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

#[derive(Clone)]
struct PendingWork {
    height: u64,
    target_hash: Hash,
    block_intro: StdBlock,
    coinbase_nonce: Hash,
    coinbase_hash: Hash,
    mkrl_modify_list: Vec<Hash>,
}

#[derive(Clone)]
struct MiningResult {
    height: u64,
    block_nonce: u32,
    coinbase_nonce: Hash,
    hash: Hash,
    scanned: u32,
    elapsed: Duration,
}

static MINING_HEIGHT: AtomicU64 = AtomicU64::new(0);

pub fn run() -> Ret<()> {
    let conf = PoWorkConf::load()?;
    run_with_conf(conf)
}

pub fn run_with_conf(conf: PoWorkConf) -> Ret<()> {
    run_with_stop(conf, None)
}

pub fn run_with_stop(conf: PoWorkConf, stop_flag: Option<Arc<AtomicBool>>) -> Ret<()> {
    println!(
        "[poworker] connect={} threads={} nonce_max={} chunk={} opencl={}",
        conf.rpcaddr, conf.threads, conf.nonce_max, conf.nonce_chunk, conf.use_opencl
    );
    let backends = build_miner_backends(&conf);
    loop {
        if should_stop(&stop_flag) {
            return Ok(());
        }
        let work = match fetch_pending(&conf) {
            Ok(work) => work,
            Err(e) => {
                eprintln!("[poworker] pending failed: {}", e);
                thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        MINING_HEIGHT.store(work.height, Ordering::Relaxed);
        println!(
            "[poworker] mining height={} target={}",
            work.height, work.target_hash
        );
        mine_height(&conf, &backends, work, &stop_flag)?;
    }
}

fn should_stop(stop_flag: &Option<Arc<AtomicBool>>) -> bool {
    stop_flag
        .as_ref()
        .map(|f| f.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn build_miner_backends(conf: &PoWorkConf) -> Vec<MinerBackend> {
    let mut backends = Vec::new();

    if conf.use_opencl {
        #[cfg(feature = "ocl")]
        {
            let opencl_resources = common::initialize_opencl(
                false,
                &conf.opencldir,
                conf.platformid,
                &conf.deviceids,
                conf.workgroups,
                conf.localsize,
                conf.unitsize,
            );
            if !opencl_resources.is_empty() {
                println!(
                    "[poworker] create {} OpenCL block miner worker(s)",
                    opencl_resources.len()
                );
                backends.extend(opencl_resources.into_iter().map(|resource| {
                    MinerBackend::Opencl {
                        resource: Arc::new(resource),
                        workgroups: conf.workgroups,
                        localsize: conf.localsize,
                        unitsize: conf.unitsize,
                    }
                }));
            }
        }

        #[cfg(not(feature = "ocl"))]
        {
            println!(
                "[poworker] use_opencl=true but app was built without `ocl` feature; fallback to CPU miner"
            );
        }
    }

    if backends.is_empty() {
        backends.extend((0..conf.threads).map(|_| MinerBackend::Cpu));
    }

    backends
}

fn api_url(conf: &PoWorkConf, path: &str) -> String {
    format!("http://{}{}", conf.rpcaddr, path)
}

fn get_json(url: &str) -> Ret<Value> {
    get_json_with(&HTTP_CLIENT, url)
}

fn get_json_notice(url: &str) -> Ret<Value> {
    get_json_with(&HTTP_CLIENT_NOTICE, url)
}

fn get_json_with(client: &HttpClient, url: &str) -> Ret<Value> {
    let text = client
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

fn json_u64(json: &Value, key: &str) -> Ret<u64> {
    json[key]
        .as_u64()
        .ok_or_else(|| sys::Error::fault(format!("missing json number field {}", key)))
}

fn parse_hash_hex(s: &str) -> Ret<Hash> {
    let bytes = hex::decode(s).map_err(|_| sys::Error::fault("hash hex invalid"))?;
    let arr: [u8; HASH_WIDTH] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| sys::Error::fault("hash hex length invalid"))?;
    Ok(Hash::from(arr))
}

fn fetch_pending(conf: &PoWorkConf) -> Ret<PendingWork> {
    let url = api_url(conf, "/query/miner/pending?stuff=true");
    let json = get_json(&url)?;
    if json["ret"].as_i64() != Some(0) {
        return sys::errf!(
            "miner pending error: {}",
            json["err"].as_str().unwrap_or("unknown")
        );
    }
    let block_intro_bytes = hex::decode(json_str(&json, "block_intro")?)
        .map_err(|_| sys::Error::fault("block_intro hex invalid"))?;
    let block_intro = StdBlock::decode_intro(mint::block_hasher, &block_intro_bytes)?;
    let coinbase_body = hex::decode(json_str(&json, "coinbase_body")?)
        .map_err(|_| sys::Error::fault("coinbase_body hex invalid"))?;
    let coinbase_nonce = parse_hash_hex(json_str(&json, "coinbase_nonce")?)?;
    // Standard registry (protocol + mint + vm) for codec consistency with hacash.
    let reg = crate::standard_registry()?;
    let (coinbase_ref, _) = mint::tx_coinbase::create_coinbase(&reg, &coinbase_body)?;
    let mut coinbase_tx = coinbase_ref
        .as_any()
        .downcast_ref::<CoinbaseTx>()
        .ok_or_else(|| sys::Error::fault("coinbase tx type invalid"))?
        .clone();
    coinbase_tx.set_mining_nonce(coinbase_nonce);
    let coinbase_hash = coinbase_tx.hash();
    let mkrl_modify_list = json["mkrl_modify_list"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| parse_hash_hex(v.as_str().unwrap_or_default()))
                .collect::<Ret<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(PendingWork {
        height: json_u64(&json, "height")?,
        target_hash: parse_hash_hex(json_str(&json, "target_hash")?)?,
        block_intro,
        coinbase_nonce,
        coinbase_hash,
        mkrl_modify_list,
    })
}

fn mine_height(
    conf: &PoWorkConf,
    backends: &[MinerBackend],
    work: PendingWork,
    stop_flag: &Option<Arc<AtomicBool>>,
) -> Ret<()> {
    let mut next_start = 0u32;
    let mut best_hash = Hash::from([255u8; HASH_WIDTH]);
    let mut cpu_chunk = conf.nonce_chunk.max(1);
    let worker_count = backends.len().max(1);
    loop {
        if should_stop(stop_flag) {
            return Ok(());
        }
        if next_start >= conf.nonce_max {
            return Ok(());
        }
        let mut handles = Vec::new();
        let mut offset = 0u32;
        let mut round_scanned = 0u32;
        let round_started = Instant::now();
        for backend in backends.iter().take(worker_count) {
            let chunk = match backend {
                MinerBackend::Cpu => cpu_chunk,
                #[cfg(feature = "ocl")]
                MinerBackend::Opencl {
                    workgroups,
                    localsize,
                    unitsize,
                    ..
                } => workgroups
                    .saturating_mul(*localsize)
                    .saturating_mul(*unitsize)
                    .max(1),
            };
            let start = next_start.saturating_add(offset);
            offset = offset.saturating_add(chunk);
            if start >= conf.nonce_max {
                break;
            }
            let space = chunk.min(conf.nonce_max.saturating_sub(start));
            round_scanned = round_scanned.saturating_add(space);
            let item = work.clone();
            let backend = backend.clone();
            handles.push(thread::spawn(move || {
                mine_chunk(item, start, space, backend)
            }));
        }
        next_start = next_start.saturating_add(offset);
        for handle in handles {
            let result = handle
                .join()
                .map_err(|_| sys::Error::fault("poworker mining thread panicked"))?;
            if result.hash.as_ref() < best_hash.as_ref() {
                best_hash = result.hash.clone();
                print_result(&result, &work.target_hash, &best_hash);
            }
            // Match server check: success when hash <= target.
            if conf.debug || result.hash.as_ref() <= work.target_hash.as_ref() {
                submit_success(conf, &result)?;
                return Ok(());
            }
        }
        // CPU adaptive batch size (align with diaworker / fullnodedev).
        if backends.iter().any(|b| matches!(b, MinerBackend::Cpu)) {
            let secs = round_started.elapsed().as_secs_f64();
            if secs.is_finite() && secs > 0.0 && round_scanned > 0 {
                let next = ((round_scanned as f64 / secs) * MINING_INTERVAL) as u32;
                cpu_chunk = next.max(1);
            }
        }
        let height = miner_notice(conf, work.height)?;
        if height >= work.height {
            return Ok(());
        }
    }
}

fn mine_chunk(
    work: PendingWork,
    nonce_start: u32,
    nonce_space: u32,
    backend: MinerBackend,
) -> MiningResult {
    let mut intro = work.block_intro.clone();
    intro.set_mrklroot(calculate_mrkl_prelude_update(
        work.coinbase_hash,
        &work.mkrl_modify_list,
    ));
    let intro_bytes = intro.encode_intro();
    let started = Instant::now();

    let (best_nonce, best_hash) = match backend {
        MinerBackend::Cpu => mine_chunk_cpu(work.height, intro_bytes, nonce_start, nonce_space),
        #[cfg(feature = "ocl")]
        MinerBackend::Opencl {
            resource,
            workgroups,
            localsize,
            unitsize,
        } => mine_chunk_opencl(
            &resource,
            work.height,
            intro_bytes,
            nonce_start,
            nonce_space,
            workgroups,
            localsize,
            unitsize,
        ),
    };

    MiningResult {
        height: work.height,
        block_nonce: best_nonce,
        coinbase_nonce: work.coinbase_nonce,
        hash: best_hash,
        scanned: nonce_space,
        elapsed: started.elapsed(),
    }
}

fn mine_chunk_cpu(
    height: u64,
    mut intro_bytes: Vec<u8>,
    nonce_start: u32,
    nonce_space: u32,
) -> (u32, Hash) {
    let mut best_nonce = nonce_start;
    let mut best_hash = Hash::from([255u8; HASH_WIDTH]);
    let end = nonce_start.saturating_add(nonce_space);
    for nonce in nonce_start..end {
        intro_bytes[79..83].copy_from_slice(&nonce.to_be_bytes());
        let hash = Hash::from(x16rs::block_hash(height, &intro_bytes));
        if hash.as_ref() < best_hash.as_ref() {
            best_hash = hash;
            best_nonce = nonce;
        }
    }
    (best_nonce, best_hash)
}

#[cfg(feature = "ocl")]
fn mine_chunk_opencl(
    opencl: &common::OpenCLResources,
    height: u64,
    intro_bytes: Vec<u8>,
    nonce_start: u32,
    nonce_space: u32,
    workgroups: u32,
    localsize: u32,
    unitsize: u32,
) -> (u32, Hash) {
    let unit_batch = (localsize as u64).saturating_mul(unitsize as u64);
    let workgroups_eff = if unit_batch == 0 {
        0
    } else {
        ((nonce_space as u64) / unit_batch).min(workgroups as u64) as u32
    };
    let gpu_nonce_space = workgroups_eff
        .saturating_mul(localsize)
        .saturating_mul(unitsize);

    let mut best = if workgroups_eff > 0 {
        let (nonce, hash) = pow::do_group_block_mining_opencl(
            opencl,
            height,
            intro_bytes.clone(),
            nonce_start,
            workgroups_eff,
            localsize,
            unitsize,
        );
        (nonce, Hash::from(hash))
    } else {
        (nonce_start, Hash::from([255u8; HASH_WIDTH]))
    };

    let tail_space = nonce_space.saturating_sub(gpu_nonce_space);
    if tail_space > 0 {
        let tail_start = nonce_start.saturating_add(gpu_nonce_space);
        let tail = mine_chunk_cpu(height, intro_bytes, tail_start, tail_space);
        if tail.1.as_ref() < best.1.as_ref() {
            best = tail;
        }
    }

    best
}

fn print_result(result: &MiningResult, target: &Hash, best_seen: &Hash) {
    let secs = result.elapsed.as_secs_f64().max(0.001);
    let rate = result.scanned as f64 / secs;
    let target_arr: [u8; HASH_WIDTH] = {
        let b = target.as_ref();
        let mut arr = [0u8; HASH_WIDTH];
        arr.copy_from_slice(b);
        arr
    };
    let target_rates = hash_to_rates(&target_arr, TARGET_BLOCK_TIME);
    let mut mnper = if target_rates.is_finite() && target_rates > 0.0 {
        rate / target_rates
    } else {
        0.0
    };
    if !mnper.is_finite() || mnper < 0.0 {
        mnper = 0.0;
    } else if mnper > 1.0 {
        mnper = 1.0;
    }
    let hac1day = mnper * ONEDAY_BLOCK_NUM * block_reward_number(result.height) as f64;
    println!(
        "[poworker] height={} nonce={} hash={} best={} rate={} ≈{:.4}HAC/day {:.4}%",
        result.height,
        result.block_nonce,
        result.hash,
        best_seen,
        rates_to_show(rate),
        hac1day,
        mnper * 100.0
    );
}

fn miner_notice(conf: &PoWorkConf, height: u64) -> Ret<u64> {
    let rqid = sys::curtimes();
    let url = api_url(
        conf,
        &format!(
            "/query/miner/notice?wait={}&height={}&rqid={}",
            conf.notice_wait, height, rqid
        ),
    );
    let json = get_json_notice(&url)?;
    if json["ret"].as_i64() != Some(0) {
        return Ok(0);
    }
    Ok(json["height"].as_u64().unwrap_or(0))
}

fn submit_success(conf: &PoWorkConf, result: &MiningResult) -> Ret<()> {
    let url = api_url(
        conf,
        &format!(
            "/submit/miner/success?height={}&block_nonce={}&coinbase_nonce={}",
            result.height,
            result.block_nonce,
            result.coinbase_nonce.as_ref().to_hex()
        ),
    );
    let json = get_json(&url)?;
    println!("[poworker] submit {}", json);
    Ok(())
}
