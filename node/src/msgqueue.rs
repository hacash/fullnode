//! Inbound tx/block queue (decouples P2P read loop from admission).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::thread::ThreadId;

use sys::Rerr;
use tokio::sync::mpsc;

pub enum InboundMsg {
    Tx {
        peer: Option<String>,
        body: Vec<u8>,
        ack: Option<SyncSender<Rerr>>,
    },
    Block {
        peer: Option<String>,
        body: Vec<u8>,
        ack: Option<SyncSender<Rerr>>,
    },
}

pub struct InboundHub {
    tx: mpsc::Sender<InboundMsg>,
    rx: Mutex<Option<mpsc::Receiver<InboundMsg>>>,
    started: AtomicBool,
    handler_thread: Mutex<Option<ThreadId>>,
}

impl InboundHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity.max(64));
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
            started: AtomicBool::new(false),
            handler_thread: Mutex::new(None),
        }
    }

    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    pub fn take_rx(&self) -> Option<mpsc::Receiver<InboundMsg>> {
        self.rx.lock().unwrap().take()
    }

    pub fn mark_started(&self) {
        self.started.store(true, Ordering::Release);
    }

    pub fn enter_handler_thread(&self) {
        *self.handler_thread.lock().unwrap() = Some(std::thread::current().id());
    }

    pub fn leave_handler_thread(&self) {
        *self.handler_thread.lock().unwrap() = None;
    }

    fn is_handler_thread(&self) -> bool {
        let cur = std::thread::current().id();
        self.handler_thread
            .lock()
            .unwrap()
            .map(|id| id == cur)
            .unwrap_or(false)
    }

    pub fn try_send(&self, msg: InboundMsg) -> Rerr {
        self.tx
            .try_send(msg)
            .map_err(|e| sys::Error::fault(format!("node inbound queue full/closed: {}", e)))
    }

    /// Backpressured enqueue for the P2P reader.  Network messages must not
    /// be discarded merely because the admission worker is temporarily busy.
    pub async fn send(&self, msg: InboundMsg) -> Rerr {
        self.tx
            .send(msg)
            .await
            .map_err(|e| sys::Error::fault(format!("node inbound queue closed: {}", e)))
    }

    pub fn submit_and_wait(&self, mut msg: InboundMsg) -> Rerr {
        if self.is_handler_thread() {
            return sys::errf!("cannot synchronously submit from node message handler thread");
        }
        if !self.is_started() {
            return sys::errf!("node message handler not started");
        }
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        match &mut msg {
            InboundMsg::Tx { ack, .. } | InboundMsg::Block { ack, .. } => {
                *ack = Some(ack_tx);
            }
        }
        self.try_send(msg)?;
        ack_rx.recv().map_err(|e| {
            sys::Error::fault(format!("node message handler response failed: {}", e))
        })?
    }

    pub fn enqueue_tx(&self, peer: Option<String>, body: Vec<u8>) -> Rerr {
        self.try_send(InboundMsg::Tx {
            peer,
            body,
            ack: None,
        })
    }

    pub fn enqueue_block(&self, peer: Option<String>, body: Vec<u8>) -> Rerr {
        self.try_send(InboundMsg::Block {
            peer,
            body,
            ack: None,
        })
    }
}
