//! Standard full-node process assembly and lifecycle. Construction is
//! [`Fullnode::open`]; process entry points live in [`crate::launch`] and register
//! apps before [`Fullnode::run`].

mod assemble;
/// INI config parsing (public for side/test-chain nodes that assemble a
/// custom consensus from `config.mint` / `config.miner`).
pub mod config;
mod host;
mod indexer;
mod miner_notice;
mod storage;

pub use crate::launch::{run, run_with, run_with_scaner};
pub use host::{AppHost, NodeApp};

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
    apps: Vec<Box<dyn NodeApp>>,
    apps_sealed: bool,
}

impl Fullnode {
    /// Construct the node and complete engine recovery/rebuild, but do not
    /// start P2P, HTTP, miner hooks, or indexer work yet.
    pub fn open(path: &Path, scaner: Option<Arc<dyn Scaner>>) -> sys::Ret<Self> {
        assemble::open(path, scaner)
    }

    /// Assemble with an injected registry and consensus (mainnet defaults in
    /// `open`; side/test chains inject their own profile + consensus).
    pub fn open_with<C>(
        path: &Path,
        scaner: Option<Arc<dyn Scaner>>,
        registry: Arc<dyn base::ExecutionServices>,
        consensus: Arc<C>,
    ) -> sys::Ret<Self>
    where
        C: base::ConsensusRuntime + mint::ConsensusApi + 'static,
    {
        assemble::open_with(path, scaner, registry, consensus)
    }

    pub(crate) fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    pub(crate) fn miner_config(&self) -> &mint::MinerConf {
        &self.miner_config
    }

    /// Append a process app. Registration closes when [`Self::run`] starts.
    pub fn add_app(&mut self, app: Box<dyn NodeApp>) -> Rerr {
        if self.apps_sealed {
            return sys::errf!("add_app after node start");
        }
        self.apps.push(app);
        Ok(())
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
            "[Hacash] height={} consensus={} scaner={} services={:?}",
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
        self.apps_sealed = true;
        // Consensus hooks must be ready before local workers can submit work.
        self.node.start(self.waiter.clone())?;

        // The listener was attached during `open`; complete checkpoint catch-up
        // before opening P2P so an indexer cannot miss a newly stable block.
        if let Some(indexer) = &self.indexer {
            indexer.start(self.waiter.clone())?;
        }

        self.attach_apps()?;
        self.node
            .clone()
            .start_p2p_on(handle, self.waiter.clone())?;
        self.server.start_on(handle, self.waiter.clone())?;
        Ok(())
    }

    fn attach_apps(&mut self) -> Rerr {
        let engine = Arc::clone(&self.engine);
        let node: Arc<dyn Node> = self.node.clone();
        let waiter = self.waiter.clone();
        let chain_id = self.chain_id;
        let Fullnode { apps, server, .. } = self;
        let mut host = AppHost::new(engine.as_ref(), node, server, chain_id, waiter);
        for app in apps.iter_mut() {
            app.attach(&mut host)?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Rerr {
        self.waiter.trigger();
        self.node.begin_shutdown();
        let mut worker_err = None;
        for app in &mut self.apps {
            if let Err(e) = app.join() {
                if worker_err.is_none() {
                    worker_err = Some(e);
                }
            }
        }
        self.engine.shutdown()?;
        match worker_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

fn install_ctrlc(waiter: Waiter) -> Rerr {
    ctrlc::set_handler(move || waiter.trigger())
        .map_err(|e| sys::Error::fault(format!("install ctrl-c handler failed: {e}")))
}
