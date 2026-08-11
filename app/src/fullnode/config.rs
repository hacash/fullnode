use std::path::PathBuf;

use field::{Address, Amount, AmtCpr, Fixed16};
use serde::Deserialize;

#[derive(Clone, Default)]
pub(super) struct RuntimeConfig {
    pub engine: base::EngineConfig,
    pub p2p: base::P2PConfig,
    pub server: base::ServerConfig,
    pub runtime: TokioRuntimeConfig,
    pub txpool_maxs: Vec<usize>,
    pub txpool_min_fee_purity: u64,
    pub miner: mint::MinerConf,
    pub mint: mint::MintConf,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    engine: base::EngineConfig,
    p2p: base::P2PConfig,
    server: base::ServerConfig,
    runtime: TokioRuntimeConfig,
    vm: base::VmConfig,
    mint: mint::MintConf,
    txpool: TxPoolConfig,
    miner: MinerFileConfig,
    diamond_miner: DiamondMinerFileConfig,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct TokioRuntimeConfig {
    pub worker_threads: usize,
    pub max_blocking_threads: usize,
}

impl Default for TokioRuntimeConfig {
    fn default() -> Self {
        Self {
            worker_threads: 4,
            max_blocking_threads: 8,
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TxPoolConfig {
    maxs: Vec<usize>,
    min_fee_purity: u64,
}

impl Default for TxPoolConfig {
    fn default() -> Self {
        Self {
            maxs: Vec::new(),
            min_fee_purity: 1_000_000 / 166,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MinerFileConfig {
    enable: bool,
    reward: String,
    message: String,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DiamondMinerFileConfig {
    enable: bool,
    reward: String,
    bid_password: String,
    bid_min: String,
    bid_max: String,
    bid_step: String,
}

pub(super) fn load(path: &std::path::Path) -> sys::Ret<RuntimeConfig> {
    let mut ini = sys::load_config(path.to_string_lossy().as_ref())?;
    // These options were exposed but never consumed. Accept and discard them
    // during upgrades so removing them from EngineConfig does not make an old
    // node configuration fail to start.
    if let Some(engine) = ini.get_mut("engine") {
        engine.remove("flush_every_blocks");
        engine.remove("flush_every_bytes");
    }
    let file: FileConfig = sys::deserialize_ini(&ini)?;
    let FileConfig {
        engine,
        p2p,
        server,
        runtime,
        vm,
        mint,
        txpool,
        miner,
        diamond_miner,
    } = file;
    let mut cfg = RuntimeConfig {
        engine,
        p2p,
        server,
        runtime,
        txpool_maxs: txpool.maxs,
        txpool_min_fee_purity: txpool.min_fee_purity,
        mint,
        ..Default::default()
    };
    cfg.engine.vm = vm;
    if cfg.engine.fast_sync && cfg.engine.unstable_block == 0 {
        return sys::errf!(
            "config [engine].unstable_block must be at least 1 while fast_sync is enabled \
             (linear sync advances the durable root in unstable_block steps)"
        );
    }
    if let Ok(dir) = std::env::var("HACASH_DATA_DIR") {
        cfg.engine.data_dir = dir;
    }
    cfg.p2p.node_key = load_or_create_node_key(&cfg.engine.data_dir, &cfg.p2p.node_name)?;
    if cfg.p2p.node_name.is_empty() {
        cfg.p2p.node_name = format!("hn{}", &hex::encode(cfg.p2p.node_key)[..8]);
    }
    normalize_runtime_config(&mut cfg);
    cfg.miner = miner_config(miner, diamond_miner)?;
    Ok(cfg)
}

fn miner_config(
    miner: MinerFileConfig,
    diamond: DiamondMinerFileConfig,
) -> sys::Ret<mint::MinerConf> {
    let mut cfg = mint::MinerConf {
        enable: miner.enable,
        diamond_enable: diamond.enable,
        ..Default::default()
    };
    if cfg.enable {
        cfg.reward = Address::from_readable(&miner.reward)
            .map_err(|e| sys::Error::fault(format!("config [miner].reward invalid: {e}")))?;
        cfg.message = fixed16_message(&miner.message);
    }
    if !cfg.diamond_enable {
        return Ok(cfg);
    }
    cfg.diamond_reward = Address::from_readable(&diamond.reward)
        .map_err(|e| sys::Error::fault(format!("config [diamond_miner].reward invalid: {e}")))?;
    if !cfg.diamond_reward.is_privkey() {
        return sys::errf!(
            "config [diamond_miner].reward must be PRIVAKEY type: {}",
            cfg.diamond_reward.to_readable()
        );
    }
    cfg.diamond_bid_account = sys::Account::create_by(&diamond.bid_password).map_err(|e| {
        sys::Error::fault(format!("config [diamond_miner].bid_password invalid: {e}"))
    })?;
    cfg.diamond_bid_min = parse_bid_amount("bid_min", &diamond.bid_min)?;
    cfg.diamond_bid_max = parse_bid_amount("bid_max", &diamond.bid_max)?;
    cfg.diamond_bid_step = parse_bid_amount("bid_step", &diamond.bid_step)?;
    Ok(cfg)
}

fn parse_bid_amount(name: &str, value: &str) -> sys::Ret<Amount> {
    Amount::from(value)
        .map_err(|e| sys::Error::fault(format!("config [diamond_miner].{name} invalid: {e}")))?
        .compress(2, AmtCpr::Grow)
}

/// Resolve runtime values derived from node-local configuration.
fn normalize_runtime_config(cfg: &mut RuntimeConfig) {
    cfg.p2p.data_dir = cfg.engine.data_dir.clone();
}

fn fixed16_message(msg: &str) -> Fixed16 {
    let mut out = [b' '; 16];
    let bytes = msg.as_bytes();
    let n = bytes.len().min(16);
    out[..n].copy_from_slice(&bytes[..n]);
    Fixed16::from(out)
}

fn load_or_create_node_key(data_dir: &str, node_name: &str) -> sys::Ret<[u8; 16]> {
    use std::io::{Read, Seek, Write};

    let data_dir = PathBuf::from(data_dir);
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| sys::Error::fault(format!("create data dir failed: {e}")))?;
    let abs = std::fs::canonicalize(&data_dir).unwrap_or_else(|_| data_dir.clone());
    let nid_path = data_dir.join("node.id");
    let mut node_key = [0u8; 16];
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&nid_path)
        .map_err(|e| sys::Error::fault(format!("open node.id failed: {e}")))?;
    let mut snid = String::new();
    file.read_to_string(&mut snid)
        .map_err(|e| sys::Error::fault(format!("read node.id failed: {e}")))?;
    if let Ok(nid) = hex::decode(snid.trim()) {
        if nid.len() == 16 {
            node_key.copy_from_slice(&nid);
        }
    }
    if node_key[0] != 0 || node_key[15] != 0 {
        return Ok(node_key);
    }
    let name = if node_name.is_empty() {
        "hx8888"
    } else {
        node_name
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let hash = sys::sha2_256(format!("{}-{name}-{nanos}", abs.display()).as_bytes());
    node_key.copy_from_slice(&hash[..16]);
    file.set_len(0)
        .map_err(|e| sys::Error::fault(format!("truncate node.id failed: {e}")))?;
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| sys::Error::fault(format!("seek node.id failed: {e}")))?;
    file.write_all(hex::encode(node_key).as_bytes())
        .map_err(|e| sys::Error::fault(format!("write node.id failed: {e}")))?;
    Ok(node_key)
}
