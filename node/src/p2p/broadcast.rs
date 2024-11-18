
/**
* broadcast
*/


impl P2PManage {

    pub async fn broadcast_unaware(&self, key: &KnowKey, ty: u16, body: Vec<u8>) {
        let mut resps = vec![];
        let peers = vec![ self.backbones(), self.offshoots() ].concat();
        for peer in peers {
            if !peer.knows.check(key) {
                peer.knows.add(key.clone());
                resps.push(peer);
            }
        }
        // build msg
        let msgbody = vec![ty.to_be_bytes().to_vec(), body].concat();
        let msgbuf = tcp_create_msg(MSG_CUSTOMER, msgbody);
        // send each
        for peer in resps {
            // println!("broadcast_unaware msg={} to peer={}", ty, peer.nick());
            let _ = peer.send(&msgbuf).await;
        }
    }

}