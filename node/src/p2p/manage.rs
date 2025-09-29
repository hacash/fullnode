
// #[derive(Clone)]
pub struct PeerMngInst {
    p2p: Arc<P2PManage>,
}

impl PeerMngInst {
    pub fn new(p2p: Arc<P2PManage>) -> PeerMngInst {
        PeerMngInst{
            p2p: p2p,
        }
    }
}



impl PeerManage for PeerMngInst {

    fn switch_peer(&self, p: Arc<Peer>) -> Arc<Peer> {
        self.p2p.switch_peer(p)
    }

    fn broadcast_message(&self, delay: u64, key: KnowKey, ty: u16, body: Vec<u8>) {
        P2PManage::broadcast_message(self.p2p.clone(), delay, key, ty, body)
    }
}


impl P2PManage {

    fn switch_peer(&self, p: Arc<Peer>) -> Arc<Peer> {
        let pubps = self.publics();
        if pubps.len() == 0 {
            return p
        }
        for pr in &pubps {
            if pr.id == p.id {
                return p
            }
        }
        // the first one
        pubps[0].clone()
    }

    fn broadcast_message(p2p: Arc<P2PManage>, delay: u64, key: KnowKey, ty: u16, body: Vec<u8>) {
        tokio::spawn(async move {
            if delay > 0 {
                // may sleep to avoid duplicate broadcasts
                asleep(delay as f32).await; 
            }
            p2p.broadcast_unaware(&key, ty, body).await;
        });
    }
}