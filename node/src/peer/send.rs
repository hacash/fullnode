

impl Peer {
    
    pub async fn send_msg(&self, ty: u16, body: Vec<u8>) -> Rerr {
        let msg = vec![ty.to_be_bytes().to_vec(), body].concat();
        self.send_p2p_msg(MSG_CUSTOMER, msg).await
    }

    pub async fn send_p2p_msg(&self, ty: u8, body: Vec<u8>) -> Rerr {
        let msgbuf = tcp_create_msg(ty, body);
        self.send(&msgbuf).await
    }

    pub async fn send(&self, buf: &Vec<u8>) -> Rerr {
        let mut w;
        {
            w = match self.conn_write.lock().unwrap().take() {
                None => return errf!("peer may be busy or closed"),
                Some(w) => w,
            };
        }
        let _ = tcp_send(&mut w, buf).await;
        {
            *self.conn_write.lock().unwrap() = Some(w);
        }
        Ok(())
    }


}