
static PEER_AUTO_ID_INCREASE: AtomicU64 = AtomicU64::new(0);


#[derive(Debug)]
pub struct Peer {
    pub id: u64,
    pub key: PeerKey,
    pub name: String,
    pub is_public: bool, // is public IP
    pub is_cntome: bool, // is connect to me
    pub addr: SocketAddr,
    // will change
    pub active: StdMutex<SystemTime>,
    pub conn_write: StdMutex<Option<OwnedWriteHalf>>,
    // data
    pub knows: Knowledge,
}

impl Peer {

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn nick(&self) -> String {
        let mut nick = self.name.clone();
        let kpx: [u8; 4] = self.key.clone()[0..4].try_into().unwrap();
        // nick += format!("【 {} 】", kpx[0]).as_str(); return nick; // debug
        if self.is_public {
            nick += format!("<{}>", self.addr).as_str();
        }
        nick
    }

    pub fn update_active(&self) {
        *self.active.lock().unwrap() = SystemTime::now();
    }

    fn take_conn_write(&self) -> Option<OwnedWriteHalf> {
        self.conn_write.lock().unwrap().take()
    }

    pub async fn disconnect(&self) {
        // println!("----- call fn disconnect peer: {}", self.nick());
        let mayconn = self.take_conn_write();
        if let None = mayconn {
            return // already closed, do nothing
        }
        let mut w = mayconn.unwrap();
        // drop conn obj to close
        // do close first
        let close_msg = vec![0u8,0,0,1,MSG_REQUEST_NEAREST_PUBLIC_NODES];
        tcp_send(&mut w, &close_msg).await; // send close mark
        // do close two
        let close_msg = vec![0u8,0,0,1,MSG_CLOSE]; // close
        tcp_send(&mut w, &close_msg).await; // send close mark
        w.forget();
    }

    pub async fn create_with_msg(mut stream: TcpStream, ty: u8, msg: Vec<u8>, mynodeinfo: Vec<u8>) -> Ret<(Arc<Peer>, OwnedReadHalf)> {
        let mut mykeyname = mynodeinfo;
        if mykeyname.len() > PEER_KEY_SIZE*2 {
            mykeyname = mykeyname[4..].to_vec(); // drop port
        }
        let conn  = &mut stream;
        let mut addr = conn.peer_addr().unwrap();
        let mut is_public = false;
        let mut is_cntome = false;
        let mut idnamebts: &[u8];
        let mut oginport: u16 = 0;
        if msg.len() < 4 {
            return errf!("msg length too short")
        }
        if MSG_REPORT_PEER == ty {
            is_cntome = true;
            oginport = u16::from_be_bytes( bufcut!(msg, 2, 4) );
            idnamebts = &msg[4..];
        }else if MSG_ANSWER_PEER == ty {
            is_public = true; // connect to public
            idnamebts = &msg[..];
        }else{
            // unsupport msg ty
            return errf!("unsupport msg ty {}", ty)
        }
        if idnamebts.len() < 32 {
            return errf!("msg length too short")
        }
        let peerkey = bufcut!(idnamebts, 0, PEER_KEY_SIZE);
        let name = Fixed16::cons( bufcut!(idnamebts, PEER_KEY_SIZE, PEER_KEY_SIZE*2) ).readable().replace(" ", "");
        if peerkey == mykeyname[0..PEER_KEY_SIZE] {
            return  errf!("cannot connect to self")
        }
        // dial to check is public ip
        if !is_public && MSG_REPORT_PEER == ty {
            // to answer mys node info
            // report my node info: mark+port+id+name
            tcp_send_msg(conn, MSG_ANSWER_PEER, mykeyname.clone()).await?;
            // check is public
            let mut pubaddr = addr.clone();
            pubaddr.set_port(oginport);
            if let Ok(pb) = tcp_dial_to_check_is_public_id(pubaddr, &peerkey, 4).await {
                if pb {
                    is_public = true; // public connect to me
                    addr.set_port(oginport);
                    // println!("public connect to me!!!")
                }
            }
        }
        
        // conn split
        let (mut read_half, mut write_half) = stream.into_split();

        // cid
        PEER_AUTO_ID_INCREASE.fetch_add(1, Ordering::Relaxed);
        let atid = PEER_AUTO_ID_INCREASE.load(Ordering::Relaxed);

        // create
        let mut peer = Peer {
            id: atid,
            key: peerkey,
            name: name,
            is_cntome: is_cntome,
            is_public: is_public,
            addr: addr,
            active: SystemTime::now().into(),
            conn_write: Some(write_half).into(),
            knows: Knowledge::new(50),
        };
        let pptr = Arc::new(peer);

        // println!("create peer {} successfully: {:?}", peer.desc(), peer);
        Ok((pptr, read_half))
    }


}