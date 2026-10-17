//! Process entry points. Apps are registered here, then [`Fullnode`] runs them.

use std::sync::Arc;

use base::Scaner;
use sys::Rerr;

use crate::apps;
use crate::fullnode::Fullnode;

/// Run the standard node without an external indexer.
pub fn run() -> Rerr {
    launch(Fullnode::open(&config_path_from_args(), None)?)
}

/// Run with an external block indexer, using the normal command-line config
/// path. Indexer-specific sections remain owned and parsed by the indexer.
pub fn run_with_scaner(scaner: Arc<dyn Scaner>) -> Rerr {
    launch(Fullnode::open(&config_path_from_args(), Some(scaner))?)
}

/// Run a node assembled with an injected registry and consensus
/// (`consensus` must also implement `mint::ConsensusApi` for the miner
/// HTTP surface). Used by side/test-chain nodes that register their own
/// parameter profile and consensus implementation.
pub fn run_with<C>(
    scaner: Option<Arc<dyn Scaner>>,
    registry: Arc<dyn base::ExecutionServices>,
    consensus: Arc<C>,
) -> Rerr
where
    C: base::ConsensusRuntime + mint::ConsensusApi + 'static,
{
    launch(Fullnode::open_with(
        &config_path_from_args(),
        scaner,
        registry,
        consensus,
    )?)
}

fn launch(mut node: Fullnode) -> Rerr {
    apps::register(&mut node)?;
    node.run()
}

fn config_path_from_args() -> std::path::PathBuf {
    std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(|| "hacash.config.ini".into())
}
