//! P2PNode struct + `Node` trait router.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base::{BlkPkg, ChainListener, Engine, Node, P2PConfig, TxPkg, TxPool, TxSubmitResult};
use field::Hash;
use sys::{Rerr, Waiter};

use crate::knowledge::Knowledge;
use crate::msgqueue::{InboundHub, InboundMsg};
use crate::peertable::PeerTable;
use crate::sync_pipeline::SyncSlot;
use crate::sync_tracker::SyncTracker;

/// Handler for a negotiated/custom message type (101..=255).
pub trait CustomMessageHandler: Send + Sync {
    fn on_connect(&self, _peer: Arc<dyn base::Peer>) -> Rerr {
        Ok(())
    }

    fn on_disconnect(&self, _peer: Arc<dyn base::Peer>) {}

    fn handle(&self, peer: Arc<dyn base::Peer>, ty: u8, body: Vec<u8>) -> Rerr;
}

pub struct P2PNode {
    pub(crate) txpool: Arc<dyn TxPool>,
    pub(crate) engine: Arc<dyn Engine>,
    pub(crate) config: P2PConfig,
    pub(crate) peertable: Arc<PeerTable>,
    pub(crate) listeners: Mutex<Vec<Arc<dyn ChainListener>>>,
    /// Global broadcast knowledge (capacity 2000).
    pub(crate) knows: Knowledge,
    pub(crate) sync_tracker: Arc<SyncTracker>,
    pub(crate) doing_sync: Arc<AtomicU64>,
    pub(crate) inserting: Arc<Mutex<()>>,
    /// Unified window sync downloader -> one BlockStream apply thread.
    pub(crate) sync_session: Arc<SyncSlot>,
    pub(crate) sync_generation: AtomicU64,
    pub(crate) orphan_blocks: Mutex<HashMap<Hash, Vec<BlkPkg>>>,
    pub(crate) inbound: Arc<InboundHub>,
    pub(crate) stopping: AtomicBool,
    pub(crate) held_replay_started: AtomicBool,
    pub(crate) custom_handlers: Mutex<HashMap<u8, Arc<dyn CustomMessageHandler>>>,
}

impl P2PNode {
    pub fn open(txpool: Arc<dyn TxPool>, engine: Arc<dyn Engine>, config: P2PConfig) -> Self {
        let peertable = Arc::new(PeerTable::new(
            config.node_key,
            config.backbone_peers,
            config.offshoot_peers,
        ));
        Self {
            txpool,
            engine,
            config,
            peertable,
            listeners: Mutex::new(Vec::new()),
            knows: Knowledge::new(2000),
            sync_tracker: Arc::new(SyncTracker::new()),
            doing_sync: Arc::new(AtomicU64::new(0)),
            inserting: Arc::new(Mutex::new(())),
            sync_session: Arc::new(Mutex::new(None)),
            sync_generation: AtomicU64::new(0),
            orphan_blocks: Mutex::new(HashMap::new()),
            inbound: Arc::new(InboundHub::new(4000)),
            stopping: AtomicBool::new(false),
            held_replay_started: AtomicBool::new(false),
            custom_handlers: Mutex::new(HashMap::new()),
        }
    }

    /// Stop accepting network work and wake the active sync pipeline. Engine
    /// shutdown remains responsible for draining in-flight work and stopping
    /// consensus hooks afterwards.
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let sync_session = self
            .sync_session
            .lock()
            .ok()
            .and_then(|mut session| session.take());
        if let Some(session) = sync_session {
            session.cancel();
        }
        self.doing_sync.store(0, Ordering::Release);
        for peer in self.peertable.values_snapshot() {
            peer.disconnect();
        }
    }

    /// Register a custom message handler. System types and the permanently
    /// invalid type 100 are rejected at registration time.
    pub fn register_custom_message_handler(
        &self,
        ty: u8,
        handler: Arc<dyn CustomMessageHandler>,
    ) -> Rerr {
        if ty <= 100 {
            return sys::errf!("custom message type must be in 101..=255, got {}", ty);
        }
        self.custom_handlers.lock().unwrap().insert(ty, handler);
        Ok(())
    }

    pub(crate) fn custom_message_handler(&self, ty: u8) -> Option<Arc<dyn CustomMessageHandler>> {
        self.custom_handlers.lock().ok()?.get(&ty).cloned()
    }

    pub(crate) fn custom_message_handlers(&self) -> Vec<Arc<dyn CustomMessageHandler>> {
        let Ok(handlers) = self.custom_handlers.lock() else {
            return Vec::new();
        };
        let mut unique: Vec<Arc<dyn CustomMessageHandler>> = Vec::new();
        for handler in handlers.values() {
            if !unique.iter().any(|current| Arc::ptr_eq(current, handler)) {
                unique.push(handler.clone());
            }
        }
        unique
    }

    pub(crate) fn registered_custom_message_types(&self) -> Vec<u8> {
        let mut types = self
            .custom_handlers
            .lock()
            .map(|handlers| handlers.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        types.sort_unstable();
        types
    }

    pub(crate) fn start_held_replay_worker(self: &Arc<Self>, waiter: Waiter) {
        if self.held_replay_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let node = self.clone();
        let _ = std::thread::Builder::new()
            .name("held-block-replay".into())
            .spawn(move || {
                while !waiter.sleep_or_quit(std::time::Duration::from_secs(5)) {
                    let Some(_hold) = waiter.try_hold() else {
                        break;
                    };
                    node.drain_deferred_blocks();
                }
            });
    }

    /// Start inbound tx/block worker (needs `Arc` — called from `start_p2p_on`).
    pub(crate) fn ensure_msg_handler(self: &Arc<Self>, waiter: Waiter) {
        let Some(mut rx) = self.inbound.take_rx() else {
            return;
        };
        let this = self.clone();
        let _ = std::thread::Builder::new()
            .name("node-msg-handler".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("node-msg-handler runtime");
                rt.block_on(async move {
                    this.inbound.enter_handler_thread();
                    this.inbound.mark_started();
                    loop {
                        tokio::select! {
                            _ = waiter.cancelled() => break,
                            msg = rx.recv() => {
                                let Some(msg) = msg else { break };
                                let Some(_hold) = waiter.try_hold() else { break };
                                this.dispatch_inbound(msg).await;
                            }
                        }
                    }
                    this.inbound.leave_handler_thread();
                    println!("[MsgHandler] loop end.");
                });
            });
    }

    async fn dispatch_inbound(self: &Arc<Self>, msg: InboundMsg) {
        let (ack, res, is_tx) = match msg {
            InboundMsg::Tx { peer, body, ack } => {
                let r = self.handle_transaction_bytes(body, peer, false);
                (ack, r, true)
            }
            InboundMsg::Block { peer, body, ack } => {
                let r = self.handle_block_bytes(body, peer);
                (ack, r, false)
            }
        };
        if let Some(ack) = ack {
            let _ = ack.send(res);
        } else if let Err(e) = res {
            // A syncing node routinely rejects live mempool transactions
            // whose dependencies have not been reached locally yet.
            if !is_tx || !e.to_string().starts_with("tx rejected:") {
                eprintln!("[MsgHandler] inbound failed: {}", e);
            }
        }
    }

    pub(crate) fn cache_orphan_block(&self, parent: Hash, block: BlkPkg) {
        const MAX_ORPHAN_BLOCKS: usize = 4096;
        let mut orphans = self.orphan_blocks.lock().unwrap();
        if orphans
            .values()
            .any(|blocks| blocks.iter().any(|item| item.hash() == block.hash()))
        {
            return;
        }
        while orphans.values().map(Vec::len).sum::<usize>() >= MAX_ORPHAN_BLOCKS {
            let Some(key) = orphans.keys().next().copied() else {
                break;
            };
            let remove_key = if let Some(blocks) = orphans.get_mut(&key) {
                blocks.pop();
                blocks.is_empty()
            } else {
                true
            };
            if remove_key {
                orphans.remove(&key);
            }
        }
        orphans.entry(parent).or_default().push(block);
    }

    pub(crate) fn take_orphan_blocks(&self, parent: &Hash) -> Vec<BlkPkg> {
        self.orphan_blocks
            .lock()
            .ok()
            .and_then(|mut orphans| orphans.remove(parent))
            .unwrap_or_default()
    }

    pub(crate) fn take_all_orphan_blocks(&self) -> Vec<BlkPkg> {
        self.orphan_blocks
            .lock()
            .ok()
            .map(|mut orphans| {
                let mut blocks = Vec::new();
                for pending in orphans.drain().map(|(_, blocks)| blocks) {
                    blocks.extend(pending);
                }
                blocks
            })
            .unwrap_or_default()
    }
}

impl Node for P2PNode {
    fn start(&self, waiter: Waiter) -> Rerr {
        self.stopping.store(false, Ordering::Release);
        self.engine.node_hooks().start(waiter)
    }

    fn admit_transaction(
        &self,
        tx: &TxPkg,
        _is_async: bool,
        only_pool: bool,
    ) -> sys::Ret<TxSubmitResult> {
        self.admit_transaction_inner(tx, only_pool)
    }

    fn submit_transaction(&self, tx: &TxPkg, is_async: bool, only_pool: bool) -> Rerr {
        self.submit_transaction_pkg(tx, is_async, only_pool, None)
    }

    fn submit_block(&self, blk: &BlkPkg, is_async: bool) -> Rerr {
        if is_async {
            if self.inbound.is_started() {
                return self
                    .inbound
                    .enqueue_block(None, blk.data().as_ref().to_vec());
            }
            // Handler not up yet - process inline.
            return self.submit_block_pkg(blk, None);
        }
        if self.inbound.is_started() {
            return self.inbound.submit_and_wait(InboundMsg::Block {
                peer: None,
                body: blk.data().as_ref().to_vec(),
                ack: None,
            });
        }
        self.submit_block_pkg(blk, None)
    }

    fn engine(&self) -> Arc<dyn Engine> {
        self.engine.clone()
    }
    fn txpool(&self) -> Arc<dyn TxPool> {
        self.txpool.clone()
    }

    fn add_chain_listener(&self, listener: Arc<dyn ChainListener>) -> Rerr {
        self.engine.add_chain_listener(listener.clone())?;
        self.listeners.lock().unwrap().push(listener);
        Ok(())
    }

    fn all_peer_prints(&self) -> Vec<String> {
        self.peertable.try_all_prints()
    }

    fn stop(&self) {
        self.begin_shutdown();
        self.engine.node_hooks().exit();
    }
}
