

impl P2PManage {

    pub async fn connect_boot_nodes(&self) -> Rerr {

        print!("[P2P] Connect {} boot nodes", self.cnf.boot_nodes.len());
        for ndip in &self.cnf.boot_nodes {
            print!(", {}", &ndip);
        }
        if !self.cnf.findnodes {
            print!(", don't search nodes");
        }
        println!(".");
        for addr in &self.cnf.boot_nodes {
            // println!("&&&& connect_boot_nodes addr {} ...", addr);
            if let Err(e) = self.connect_node(*addr).await {
                println!("[P2P Error] Connect to {}, {}", &addr, e);
            }
            // println!("&&&& connect_boot_nodes addr {} ok.", addr);
        }
        Ok(())
    }

    pub async fn connect_node(&self, addr: SocketAddr) -> Ret<Arc<Peer>> {
        let conn = tcp_dial_connect(addr, 6).await?;
        let report_me = true;
        self.handle_conn(conn, report_me).await
    }

    pub async fn handle_conn(&self, mut conn: TcpStream, report_me: bool) -> Ret<Arc<Peer>> {
        tcp_check_handshake(&mut conn, 5).await?;
        let mynodeinfo = self.pick_my_node_info();
        let mndf = mynodeinfo.clone();
        if report_me {
            // report my node info: mark+port+id+name
            // println!("&&&& tcp_send_msg(&mut conn, MSG_REPORT_PEER, mndf) ...");
            tcp_send_msg(&mut conn, MSG_REPORT_PEER, mndf).await?;
            // println!("&&&& tcp_send_msg(&mut conn, MSG_REPORT_PEER, mndf) ok.");
        }
        // deal conn
        // println!("&&&& insert_peer(conn, mynodeinfo) ...");
        self.insert_peer(conn, mynodeinfo).await
    }

    pub async fn insert_peer(&self, conn: TcpStream, mynodeinfo: Vec<u8>) -> Ret<Arc<Peer>> {
        // println!("&&&& try_create_peer(peer.clone()) ...");
        let (peer, conn_read) = self.try_create_peer(conn, mynodeinfo).await?;
        // println!("&&&& try_create_peer(peer.clone()) ok.");
        // loop read peer msg
        // println!("&&&& handle_peer_message(peer.clone(), conn_read) ...");
        self.handle_peer_message(peer.clone(), conn_read).await?;
        // println!("&&&& handle_peer_message(peer.clone(), conn_read) ok.");
        // insert to node list
        // println!("&&&& insert(peer.clone()) ...");
        let droped = self.insert(peer.clone());
        self.delay_close_peer(droped, 15).await; // delay 15 secs to close
        // println!("&&&& insert(peer.clone()) ok.");
        Ok(peer)
    }


    async fn try_create_peer(&self, mut stream: TcpStream, mynodeinfo: Vec<u8>) -> Ret<(Arc<Peer>, OwnedReadHalf)> {
        let conn = &mut stream;
        // read first msg
        let (ty, body) = tcp_read_msg(conn, 5).await?;
        // println!("&&&& try_create_peer.tcp_read_msg() ty = {} .", ty);
        if MSG_REMIND_ME_IS_PUBLIC == ty {
            return errf!("ok") // finish close

        }else if MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK == ty {
            let buf = self.cnf.node_key.clone();
            let _ = AsyncWriteExt::write_all(conn, &buf).await;
            return errf!("ok") // finish close

        }else if MSG_REQUEST_NEAREST_PUBLIC_NODES == ty {
            let peerlist = self.publics();
            let (count, adrbts) = serialize_public_nodes(&peerlist, 100); // max 100
            let retbts = vec![vec![count as u8], adrbts].concat(); // + len
            let _ = AsyncWriteExt::write_all(conn, &retbts).await;
            return errf!("ok") // finish close
        }
        // other msg
        // println!("&&&& try_create_peer.create_with_msg() ty = {}.", ty);
        Peer::create_with_msg(stream, ty, body, mynodeinfo).await
    }
    

    fn pick_my_node_info(&self) -> Vec<u8> {
        let mut nodeinfo = vec![0u8; 2+2+PEER_KEY_SIZE*2];
        // port
        nodeinfo.splice(2..4, self.cnf.listen.to_be_bytes());
        // id
        nodeinfo.splice(4..20, self.cnf.node_key);
        //name
        let mut namebt = self.cnf.node_name.clone();
        namebt += "                ";
        namebt.truncate(PEER_KEY_SIZE); // node name max 16
        nodeinfo.splice(20..20+PEER_KEY_SIZE, namebt.into_bytes());
        // ok
        nodeinfo
    }


    async fn delay_close_peer(&self, peer: Option<Arc<Peer>>, delay: u64) {
        if peer.is_none() {
            return
        }
        let peer = peer.unwrap();
        // disconnect and drop peer
        if delay == 0 {
            peer.disconnect().await;
            return // close immediately
        }
        // set delay to close
        tokio::spawn(async move{
            asleep(delay as f32).await;
            peer.disconnect().await;
        });
    }

}


