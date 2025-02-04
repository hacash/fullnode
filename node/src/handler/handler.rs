
pub struct MsgHandler {
    engine: Arc<dyn Engine>,
    txpool: Arc<MemTxPool>,
    p2pmng: StdMutex<Option<Box<dyn PeerManage>>>,

    blktx: Sender<BlockTxArrive>,
    blktxch: StdMutex<Option<Receiver<BlockTxArrive>>>,

    doing_sync: AtomicU64,
    knows: Knowledge,
    exiter: Exiter,

    inserting: StdMutex<bool>, // is exited
}

impl MsgHandler {

    pub fn new(engine: Arc<dyn Engine>, txpool: Arc<MemTxPool>) -> MsgHandler {
        let (tx, rx): (Sender<BlockTxArrive>, Receiver<BlockTxArrive>) = mpsc::channel(4000);
        MsgHandler{
            engine: engine,
            txpool: txpool,
            p2pmng: None.into(),
            blktx: tx,
            blktxch: Some(rx).into(),
            doing_sync: AtomicU64::new(0),
            knows: Knowledge::new(200),
            exiter: Exiter::new(),
            inserting: StdMutex::new(false),
        }
    }

    pub fn switch_peer(&self, p: Arc<Peer>) -> Arc<Peer> {
        self.p2pmng.lock().unwrap().as_ref().unwrap().switch_peer(p)
    }

    pub fn set_p2p_mng(&self, mng: Box<dyn PeerManage>) {
        let mut mymng = self.p2pmng.lock().unwrap();
        *mymng = Some(mng);
    }

    pub async fn submit_transaction(&self, body: Vec<u8>) {
        let _ = self.blktx.send(BlockTxArrive::Tx(None, body)).await;
    }

    pub async fn submit_block(&self, body: Vec<u8>) {
        let _ = self.blktx.send(BlockTxArrive::Block(None, body)).await;
    }

    pub fn exit(&self) {
        // wait block inserting finish
        let lk = self.inserting.lock().unwrap();
        self.exiter.exit();
        drop(lk)
    }

}


/**
* handle message
*/
impl MsgHandler {

    pub async fn on_connect(&self, peer: Arc<Peer>) {
        // println!("on_connect peer={}", peer.nick());
        // println!("&&&& peer.send_msg(MSG_REQ_STATUS, vec![]) ...");
        let _ = peer.send_msg(MSG_REQ_STATUS, vec![]).await;
        // println!("&&&& peer.send_msg(MSG_REQ_STATUS, vec![]) ok.");
    }
    
    pub async fn on_disconnect(&self, _peer: Arc<Peer>) {
        // println!("- on disconnect peer = {}", peer.nick());
        // do nothing
    }
    
    pub async fn on_message(&self, peer: Arc<Peer>, ty: u16, body: Vec<u8>) {
        // println!("&&&& on_message peer={} ty={} len={}", peer.nick(), ty, body.len());

        match ty {
            MSG_TX_SUBMIT =>      { let _ = self.blktx.send(BlockTxArrive::Tx(Some(peer.clone()), body)).await; },
            MSG_BLOCK_DISCOVER => { let _ = self.blktx.send(BlockTxArrive::Block(Some(peer.clone()), body)).await; },
            MSG_BLOCK_HASH =>     { self.receive_hashs(peer, body).await; },
            MSG_REQ_BLOCK_HASH => { self.send_hashs(peer, body).await; },
            MSG_BLOCK =>          { self.receive_blocks(peer, body).await; },
            MSG_REQ_BLOCK =>      { self.send_blocks(peer, body).await; },
            MSG_REQ_STATUS =>     { self.send_status(peer).await; },
            MSG_STATUS =>         { self.receive_status(peer, body).await; },
            _ => /* not find msg type and ignore */ (),
        };

    }


}