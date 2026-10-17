//! Process extensions registered outside [`super::Fullnode`]. The container calls
//! [`NodeApp::attach`] in registration order and [`NodeApp::join`] in that same order.

use std::sync::Arc;

use base::{BlockSubmitGate, ChainId, ChainListener, Node};
use sys::{Rerr, Waiter};

pub trait NodeApp: Send {
    fn name(&self) -> &'static str;

    /// Register listeners and gates, and start background work.
    /// A skipped app returns `Ok` and leaves [`NodeApp::join`] as a no-op.
    fn attach(&mut self, host: &mut AppHost) -> Rerr;

    fn join(&mut self) -> Rerr {
        Ok(())
    }
}

/// Capabilities an app may use while attaching. The HTTP route table is already fixed;
/// block-submit gates can still be appended.
pub struct AppHost<'a> {
    engine: &'a chain::ChainEngine,
    node: Arc<dyn Node>,
    server: &'a mut server::HttpServer,
    chain_id: ChainId,
    waiter: Waiter,
}

impl<'a> AppHost<'a> {
    pub(super) fn new(
        engine: &'a chain::ChainEngine,
        node: Arc<dyn Node>,
        server: &'a mut server::HttpServer,
        chain_id: ChainId,
        waiter: Waiter,
    ) -> Self {
        Self {
            engine,
            node,
            server,
            chain_id,
            waiter,
        }
    }

    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    pub fn node(&self) -> Arc<dyn Node> {
        Arc::clone(&self.node)
    }

    pub fn waiter(&self) -> Waiter {
        self.waiter.clone()
    }

    pub fn engine(&self) -> &'a chain::ChainEngine {
        self.engine
    }

    pub fn server_mut(&mut self) -> &mut server::HttpServer {
        self.server
    }

    pub fn add_chain_listener(&self, listener: Arc<dyn ChainListener>) -> Rerr {
        self.engine.add_chain_listener(listener)
    }

    pub fn add_block_submit_gate(&mut self, gate: BlockSubmitGate) {
        self.server.add_block_submit_gate(gate);
    }
}
