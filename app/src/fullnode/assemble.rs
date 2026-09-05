use std::path::Path;
use std::sync::Arc;

use base::{ApiService, ConsensusRuntime, Engine, Node, Scaner, Store, TxPool};

use super::Fullnode;
use super::config::{self, RuntimeConfig};

pub(super) fn open(path: &Path, scaner: Option<Arc<dyn Scaner>>) -> sys::Ret<Fullnode> {
    let config = config::load(path)?;
    let registry = Arc::new(crate::standard_registry()?);
    let consensus = Arc::new(mint::HacashConsensus::with_config(
        registry.as_ref(),
        config.mint.clone(),
        config.miner.clone(),
    )?);
    open_with(path, scaner, registry, consensus)
}

/// Assembly with an injected registry and consensus (used by side/test-chain
/// nodes that register their own parameter profile and consensus).
pub(super) fn open_with<C>(
    path: &Path,
    scaner: Option<Arc<dyn Scaner>>,
    registry: Arc<dyn base::ExecutionServices>,
    consensus: Arc<C>,
) -> sys::Ret<Fullnode>
where
    C: base::ConsensusRuntime + mint::ConsensusApi + 'static,
{
    let config = config::load(path)?;
    let miner_enabled = config.miner.enable;
    let consensus_runtime: Arc<dyn ConsensusRuntime> = consensus.clone();
    let api_consensus: Arc<dyn mint::ConsensusApi> = consensus;

    let waiter = sys::Waiter::new();
    let engine = open_engine(registry, &config, consensus_runtime, waiter.clone())?;
    let node = open_node(engine.clone(), &config, miner_enabled)?;
    engine.add_chain_listener(Arc::new(node::TxPoolMaintainer::new(
        engine.clone(),
        node.txpool(),
    )))?;

    let mut services = standard_api_services(api_consensus, &config.engine.vm);
    let scaner = scaner
        .map(|scaner| super::indexer::attach(engine.clone(), scaner, &mut services))
        .transpose()?;
    let server = server::HttpServer::open(node.clone(), services, config.server, unix_seconds());

    Ok(Fullnode {
        engine,
        node,
        server,
        runtime: config.runtime,
        indexer: scaner,
        waiter,
        miner_config: config.miner,
        chain_id: config.mint.chain_id,
        diamond_bider: None,
    })
}

fn open_engine(
    registry: Arc<dyn base::ExecutionServices>,
    config: &RuntimeConfig,
    consensus: Arc<dyn ConsensusRuntime>,
    waiter: sys::Waiter,
) -> sys::Ret<Arc<chain::ChainEngine>> {
    let store = open_store(&config.engine)?;
    chain::ChainEngine::open(
        registry,
        config.engine.clone(),
        consensus,
        store,
        waiter,
        config.txpool_min_fee_purity,
    )
}

fn open_node(
    engine: Arc<chain::ChainEngine>,
    config: &RuntimeConfig,
    miner_enabled: bool,
) -> sys::Ret<Arc<node::P2PNode>> {
    let mut groups = engine.tx_policy().tx_pool_groups();
    apply_txpool_caps(&mut groups, miner_enabled, &config.txpool_maxs);
    let txpool: Arc<dyn TxPool> = Arc::new(node::MemTxPool::with_groups(
        config.txpool_min_fee_purity,
        groups,
    ));
    Ok(Arc::new(node::P2PNode::open(
        txpool,
        engine,
        config.p2p.clone(),
    )))
}

fn standard_api_services(
    consensus: Arc<dyn mint::ConsensusApi>,
    vm: &base::VmConfig,
) -> Vec<Arc<dyn ApiService>> {
    let mut services: Vec<Arc<dyn ApiService>> = vec![
        Arc::new(api::StatusApi),
        Arc::new(api::ChainApi),
        Arc::new(api::ContractStorageApi),
        Arc::new(api::PoolApi),
        Arc::new(api::AccountApi),
    ];
    services.extend(mint::api::api_services(consensus.clone()));
    services.push(Arc::new(super::miner_notice::MinerNoticeApi { consensus }));
    let tx_creator: Arc<dyn base::TransactionCreator> =
        Arc::new(protocol::tx_std::create_standard_transaction);
    services.extend(vm::api::api_services(
        tx_creator,
        vm.log_delete_auth_hash.clone(),
    ));
    services
}

fn apply_txpool_caps(
    groups: &mut [base::TxPoolGroupSpec],
    miner_enabled: bool,
    configured: &[usize],
) {
    for (idx, spec) in groups.iter_mut().enumerate() {
        if !miner_enabled {
            spec.default_capacity = 10;
        }
        if let Some(capacity) = configured.get(idx) {
            spec.default_capacity = *capacity;
        }
    }
}

fn open_store(config: &base::EngineConfig) -> sys::Ret<Arc<dyn Store>> {
    super::storage::open(&config.data_dir)
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
