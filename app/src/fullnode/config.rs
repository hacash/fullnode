use std::path::PathBuf;

use field::{Address, Amount, AmtCpr, Fixed16};
use sys::{
    IniSec, ini_bool, ini_deny_unknown, ini_seq, ini_str, ini_str_or, ini_u16, ini_u64, ini_usize,
};

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

#[derive(Clone)]
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

#[derive(Clone)]
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

#[derive(Clone, Default)]
struct MinerFileConfig {
    enable: bool,
    reward: String,
    message: String,
}

#[derive(Clone, Default)]
struct DiamondMinerFileConfig {
    enable: bool,
    reward: String,
    bid_password: String,
    bid_min: String,
    bid_max: String,
    bid_step: String,
}

// ================================ INI section decoders ================================
//
// Hand-written replacements for the former serde `deserialize_ini` layer.
// Semantics preserved per section: `key =` (empty value) applies the default,
// `key = ""` is an explicit empty string, bools accept exactly
// "true"/"1"/"false"/"0", integers parse strictly, comma-separated values
// decode as trimmed sequences, and `ini_deny_unknown` reproduces
// `deny_unknown_fields` (bare empty keys stay tolerated).

fn decode_engine(sec: &IniSec) -> sys::Ret<base::EngineConfig> {
    ini_deny_unknown(
        sec,
        "engine",
        &[
            "data_dir",
            "fast_sync",
            "unstable_block",
            "side_tree_capacity",
            "recent_blocks",
            "average_fee_purity",
            "show_miner_name",
        ],
    )?;
    let mut cfg = base::EngineConfig::default();
    cfg.data_dir = ini_str_or(sec, "data_dir", &cfg.data_dir);
    cfg.fast_sync = ini_bool(sec, "fast_sync", cfg.fast_sync)?;
    cfg.unstable_block = ini_u64(sec, "unstable_block", cfg.unstable_block)?;
    cfg.side_tree_capacity = ini_usize(sec, "side_tree_capacity", cfg.side_tree_capacity)?;
    cfg.recent_blocks = ini_bool(sec, "recent_blocks", cfg.recent_blocks)?;
    cfg.average_fee_purity = ini_bool(sec, "average_fee_purity", cfg.average_fee_purity)?;
    cfg.show_miner_name = ini_bool(sec, "show_miner_name", cfg.show_miner_name)?;
    Ok(cfg)
}

fn decode_vm(sec: &IniSec) -> sys::Ret<base::VmConfig> {
    ini_deny_unknown(
        sec,
        "vm",
        &["log_enable", "log_open_height", "log_delete_auth_hash"],
    )?;
    let mut cfg = base::VmConfig::default();
    cfg.log_enable = ini_bool(sec, "log_enable", cfg.log_enable)?;
    cfg.log_open_height = ini_u64(sec, "log_open_height", cfg.log_open_height)?;
    cfg.log_delete_auth_hash = ini_str_or(sec, "log_delete_auth_hash", &cfg.log_delete_auth_hash);
    Ok(cfg)
}

fn decode_p2p(sec: &IniSec) -> sys::Ret<base::P2PConfig> {
    ini_deny_unknown(
        sec,
        "p2p",
        &[
            "listen_ip",
            "block_queue_cap",
            "boot_nodes",
            "node_name",
            "listen_port",
            "find_nodes",
            "accept_nodes",
            "use_stable_nodes",
            "backbone_peers",
            "offshoot_peers",
        ],
    )?;
    let mut cfg = base::P2PConfig::default();
    if let Some(v) = ini_str(sec, "listen_ip") {
        cfg.listen_ip = v.parse().map_err(|_| {
            sys::Error::fault(format!("config key listen_ip has invalid IP value {v:?}"))
        })?;
    }
    cfg.block_queue_cap = ini_usize(sec, "block_queue_cap", cfg.block_queue_cap)?;
    if let Some(nodes) = ini_seq(sec, "boot_nodes") {
        cfg.boot_nodes = nodes;
    }
    cfg.node_name = ini_str_or(sec, "node_name", &cfg.node_name);
    cfg.listen_port = ini_u16(sec, "listen_port", cfg.listen_port)?;
    cfg.find_nodes = ini_bool(sec, "find_nodes", cfg.find_nodes)?;
    cfg.accept_nodes = ini_bool(sec, "accept_nodes", cfg.accept_nodes)?;
    cfg.use_stable_nodes = ini_bool(sec, "use_stable_nodes", cfg.use_stable_nodes)?;
    cfg.backbone_peers = ini_usize(sec, "backbone_peers", cfg.backbone_peers)?;
    cfg.offshoot_peers = ini_usize(sec, "offshoot_peers", cfg.offshoot_peers)?;
    Ok(cfg)
}

fn decode_server(sec: &IniSec) -> sys::Ret<base::ServerConfig> {
    ini_deny_unknown(
        sec,
        "server",
        &["enable", "listen_ip", "listen_port", "debug_routes"],
    )?;
    let mut cfg = base::ServerConfig::default();
    cfg.enable = ini_bool(sec, "enable", cfg.enable)?;
    if let Some(v) = ini_str(sec, "listen_ip") {
        cfg.listen_ip = v.parse().map_err(|_| {
            sys::Error::fault(format!("config key listen_ip has invalid IP value {v:?}"))
        })?;
    }
    cfg.listen_port = ini_u16(sec, "listen_port", cfg.listen_port)?;
    cfg.debug_routes = ini_bool(sec, "debug_routes", cfg.debug_routes)?;
    Ok(cfg)
}

fn decode_runtime(sec: &IniSec) -> sys::Ret<TokioRuntimeConfig> {
    ini_deny_unknown(sec, "runtime", &["worker_threads", "max_blocking_threads"])?;
    let mut cfg = TokioRuntimeConfig::default();
    cfg.worker_threads = ini_usize(sec, "worker_threads", cfg.worker_threads)?;
    cfg.max_blocking_threads = ini_usize(sec, "max_blocking_threads", cfg.max_blocking_threads)?;
    Ok(cfg)
}

fn decode_txpool(sec: &IniSec) -> sys::Ret<TxPoolConfig> {
    ini_deny_unknown(sec, "txpool", &["maxs", "min_fee_purity"])?;
    let mut cfg = TxPoolConfig::default();
    if let Some(maxs) = ini_seq(sec, "maxs") {
        let mut parsed = Vec::with_capacity(maxs.len());
        for v in maxs {
            parsed.push(v.parse().map_err(|_| {
                sys::Error::fault(format!("config key maxs has invalid usize value {v:?}"))
            })?);
        }
        cfg.maxs = parsed;
    }
    cfg.min_fee_purity = ini_u64(sec, "min_fee_purity", cfg.min_fee_purity)?;
    Ok(cfg)
}

fn decode_miner(sec: &IniSec) -> sys::Ret<MinerFileConfig> {
    ini_deny_unknown(sec, "miner", &["enable", "reward", "message"])?;
    let mut cfg = MinerFileConfig::default();
    cfg.enable = ini_bool(sec, "enable", cfg.enable)?;
    cfg.reward = ini_str_or(sec, "reward", &cfg.reward);
    cfg.message = ini_str_or(sec, "message", &cfg.message);
    Ok(cfg)
}

fn decode_diamond_miner(sec: &IniSec) -> sys::Ret<DiamondMinerFileConfig> {
    ini_deny_unknown(
        sec,
        "diamond_miner",
        &[
            "enable",
            "reward",
            "bid_password",
            "bid_min",
            "bid_max",
            "bid_step",
        ],
    )?;
    let mut cfg = DiamondMinerFileConfig::default();
    cfg.enable = ini_bool(sec, "enable", cfg.enable)?;
    cfg.reward = ini_str_or(sec, "reward", &cfg.reward);
    cfg.bid_password = ini_str_or(sec, "bid_password", &cfg.bid_password);
    cfg.bid_min = ini_str_or(sec, "bid_min", &cfg.bid_min);
    cfg.bid_max = ini_str_or(sec, "bid_max", &cfg.bid_max);
    cfg.bid_step = ini_str_or(sec, "bid_step", &cfg.bid_step);
    Ok(cfg)
}

pub(super) fn load(path: &std::path::Path) -> sys::Ret<RuntimeConfig> {
    let mut ini = sys::load_config(path.to_string_lossy().as_ref())?;
    // These options were exposed but never consumed: accept and discard them during
    // upgrades so removing them from EngineConfig does not break old configurations.
    if let Some(engine) = ini.get_mut("engine") {
        engine.remove("flush_every_blocks");
        engine.remove("flush_every_bytes");
    }
    let engine = decode_engine(sys::ini_section(&ini, "engine"))?;
    let p2p = decode_p2p(sys::ini_section(&ini, "p2p"))?;
    let server = decode_server(sys::ini_section(&ini, "server"))?;
    let runtime = decode_runtime(sys::ini_section(&ini, "runtime"))?;
    let vm = decode_vm(sys::ini_section(&ini, "vm"))?;
    let mint = mint::MintConf::from_ini(sys::ini_section(&ini, "mint"))?;
    let txpool = decode_txpool(sys::ini_section(&ini, "txpool"))?;
    let miner = decode_miner(sys::ini_section(&ini, "miner"))?;
    let diamond_miner = decode_diamond_miner(sys::ini_section(&ini, "diamond_miner"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use sys::IniObj;

    fn parse(text: &str) -> IniObj {
        sys::load_ini_text(text).expect("parse ini")
    }

    /// The sample config decodes with the same values as the old serde layer:
    /// bare empty keys fall back to defaults, explicit values win.
    #[test]
    fn sample_config_decodes() {
        let ini = parse(
            "[engine]\ndata_dir = ./hacash_mainnet_data\nfast_sync = false\nshow_miner_name = true\n\
             [p2p]\nlisten_ip = 0.0.0.0\nlisten_port = 3337\nnode_name =\nboot_nodes = 182.92.163.225:3337,54.193.49.59:3337\n\
             [server]\nlisten_ip = 127.0.0.1\nlisten_port = 8082\ndebug_routes = false\n\
             [mint]\nchain_id = 0\ndiamond_form = true\n\
             [txpool]\nmaxs =\nmin_fee_purity = 6024\n\
             [miner]\nenable = false\n\
             [diamond_miner]\nenable = false\n\
             [vm]\nlog_enable = false\nlog_open_height = 0\nlog_delete_auth_hash =",
        );
        let engine = decode_engine(sys::ini_section(&ini, "engine")).unwrap();
        assert_eq!(engine.data_dir, "./hacash_mainnet_data");
        assert!(!engine.fast_sync);
        assert!(engine.show_miner_name);
        assert_eq!(engine.unstable_block, 4); // default
        let p2p = decode_p2p(sys::ini_section(&ini, "p2p")).unwrap();
        assert_eq!(p2p.listen_port, 3337);
        assert_eq!(p2p.node_name, ""); // bare `node_name =` keeps the default (empty)
        assert_eq!(
            p2p.boot_nodes,
            vec![
                "182.92.163.225:3337".to_owned(),
                "54.193.49.59:3337".to_owned()
            ]
        );
        assert_eq!(p2p.backbone_peers, 4); // default
        let server = decode_server(sys::ini_section(&ini, "server")).unwrap();
        assert!(!server.enable); // default (key absent)
        assert_eq!(server.listen_port, 8082);
        let vm = decode_vm(sys::ini_section(&ini, "vm")).unwrap();
        assert_eq!(vm.log_delete_auth_hash, ""); // bare empty -> explicit default
        let txpool = decode_txpool(sys::ini_section(&ini, "txpool")).unwrap();
        assert_eq!(txpool.maxs, Vec::<usize>::new()); // bare `maxs =` -> default
        assert_eq!(txpool.min_fee_purity, 6024);
    }

    #[test]
    fn unknown_sections_are_tolerated_unknown_keys_rejected() {
        let ini = parse("[hascan]\nsome_key = 1\n[engine]\nfast_sync = false\nbogus = 1\n");
        assert!(decode_engine(sys::ini_section(&ini, "engine")).is_err());
        let ini = parse("[hascan]\nsome_key = 1\n[engine]\nfast_sync = false\nbogus =\n");
        // bare empty unknown keys stay tolerated (old section iterator skipped them)
        assert!(decode_engine(sys::ini_section(&ini, "engine")).is_ok());
    }
}
