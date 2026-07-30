//! Standard full-node process assembly and lifecycle.
//!
//! This module deliberately uses direct concrete assembly instead of a generic
//! builder: `app` is the one composition root that knows the standard Hacash
//! stack.  The split is only between construction (`Fullnode::open`) and
//! process ownership (`Fullnode::run`).

mod assemble;
mod bider;
mod config;
mod indexer;
mod storage;

use std::path::Path;
use std::sync::Arc;

use base::{ChainId, ChainView, Node, Scaner};
use sys::{Rerr, Waiter};

use indexer::AttachedIndexer;

/// Fully assembled standard node. Nothing accepts network traffic until
/// [`Self::run`] starts its services.
pub struct Fullnode {
    engine: Arc<chain::ChainEngine>,
    node: Arc<node::P2PNode>,
    server: server::HttpServer,
    runtime: config::TokioRuntimeConfig,
    indexer: Option<AttachedIndexer>,
    waiter: Waiter,
    miner_config: mint::MinerConf,
    chain_id: ChainId,
    diamond_bider: Option<std::thread::JoinHandle<()>>,
}

/// Run the standard node without an external indexer.
pub fn run() -> Rerr {
    Fullnode::open(&config_path_from_args(), None)?.run()
}

/// Run with an external block indexer, using the normal command-line config
/// path. Indexer-specific sections remain owned and parsed by the indexer.
pub fn run_with_scaner(scaner: Arc<dyn Scaner>) -> Rerr {
    Fullnode::open(&config_path_from_args(), Some(scaner))?.run()
}

/// Default config path for the standard full-node command.
fn config_path_from_args() -> std::path::PathBuf {
    std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(|| "hacash.config.ini".into())
}

impl Fullnode {
    /// Construct the node and complete engine recovery/rebuild, but do not
    /// start P2P, HTTP, miner hooks, or indexer work yet.
    pub fn open(path: &Path, scaner: Option<Arc<dyn Scaner>>) -> sys::Ret<Self> {
        assemble::open(path, scaner)
    }

    /// Start services in dependency order and own the complete shutdown path.
    pub fn run(mut self) -> Rerr {
        install_ctrlc(self.waiter.clone())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(self.runtime.worker_threads.clamp(1, 32))
            .max_blocking_threads(self.runtime.max_blocking_threads.clamp(1, 32))
            .enable_all()
            .build()
            .map_err(|e| sys::Error::fault(format!("tokio runtime build failed: {e}")))?;

        if let Err(e) = self.start(runtime.handle()) {
            let _ = self.stop();
            return Err(e);
        }

        println!(
            "[hacash] ready: height={} consensus={} scaner={} services={:?}",
            self.engine.latest_height(),
            self.engine.consensus().name(),
            self.indexer.as_ref().map_or("off", AttachedIndexer::name),
            self.server.service_names(),
        );

        runtime.block_on(async { self.waiter.cancelled().await });
        self.stop()
    }

    fn start(&mut self, handle: &tokio::runtime::Handle) -> Rerr {
        let _runtime = handle.enter();
        // Consensus hooks must be ready before local workers can submit work.
        self.node.start(self.waiter.clone())?;

        // The listener was attached during `open`; complete checkpoint catch-up
        // before opening P2P so an indexer cannot miss a newly stable block.
        if let Some(indexer) = &self.indexer {
            indexer.start(self.waiter.clone())?;
        }

        self.diamond_bider = bider::start(
            self.miner_config.clone(),
            self.node.clone(),
            self.chain_id,
            self.waiter.clone(),
        );
        self.node
            .clone()
            .start_p2p_on(handle, self.waiter.clone())?;
        self.server.start_on(handle, self.waiter.clone())?;
        Ok(())
    }

    fn stop(&mut self) -> Rerr {
        self.waiter.trigger();
        self.node.begin_shutdown();
        let bider_panicked = self
            .diamond_bider
            .take()
            .is_some_and(|bider| bider.join().is_err());
        self.engine.shutdown()?;
        if bider_panicked {
            return Err(sys::Error::fault("diamond auto-bid worker panicked"));
        }
        Ok(())
    }
}

fn install_ctrlc(waiter: Waiter) -> Rerr {
    ctrlc::set_handler(move || waiter.trigger())
        .map_err(|e| sys::Error::fault(format!("install ctrl-c handler failed: {e}")))
}
