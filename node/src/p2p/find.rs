

/**
 * Find and Connect new public node
 */
impl P2PManage {

    pub async fn find_nodes(&self) {
        do_find_nodes(self).await;
    }
    
}



async fn do_insert_new_nodes(this: &P2PManage, mearest_addrs: Vec<SocketAddr>, first: &PeerKey) {
    let nodelen = mearest_addrs.len();
    if nodelen == 0 {
        let publen = this.backbones.lock().unwrap().len();
        println!("connected {} public nodes, not find any nearest.", publen);
        return
    }
    println!("find {} nearest nodes, try connect...", nodelen);
    // try connect for each
    let mut cncount = 0;
    for addr in &mearest_addrs {
        if let Err(e) = this.connect_node(addr).await {
            println!("failed connect to {}, {}.", addr, e);
            continue
        }
        cncount += 1;
        if cncount >= 16 {
            break // end
        }
        if let None = find_peer_from_dht_list(this.backbones.clone(), first) {
            // println!("--------------------let None = find_peer_from_dht_list(this.backbones.clone(), first = {}", hex::encode(first));
            break // replace all old nodes
        }
    }
    // finish
}


async fn do_find_nodes(this: &P2PManage) {
    print!("[P2P] Searching nodes...");
    let mut allfindnodes = HashMap::<PeerKey, SocketAddr>::new();
    // search form backbone nodes
    let mut willdropeds = vec![ this.cnf.node_key.clone() ];
    let peers = this.backbones();
    for p in peers {
        willdropeds.push(p.key.clone());
        if let Err(e) = request_public_nodes(p.addr, &mut allfindnodes).await {
            println!("\n\ndo_find_nodes request_public_nodes Error: {}\n", e.to_string());
        }
    }
    if willdropeds.len() <= 1 {
        println!("not connected any nodes.");
        return
    }
    // drop myself and current connected
    for rmp in &willdropeds {
        allfindnodes.remove(rmp);
    }
    let newndcount = allfindnodes.len();
    if newndcount == 0 {
        println!("not find any new nodes.");
        return
    }
    // check nearest
    let compare = &willdropeds[0]; // my node key
    let first = &willdropeds[1]; // first
    let least = &willdropeds[willdropeds.len() - 1]; // tail
    let mut nearest_list: Vec<PeerKey> = Vec::new();
    let mut mearest_addrs: Vec<SocketAddr> = Vec::new();
    for nd in &allfindnodes {
        if insert_nearest_to_dht_list(&mut nearest_list, compare, least, nd.0) {
            mearest_addrs.push(allfindnodes[nd.0].clone());
        }
    }
    // do insert
    do_insert_new_nodes(this, mearest_addrs, first).await
}



async fn request_public_nodes(addr: SocketAddr, datas: &mut HashMap<PeerKey, SocketAddr>) -> Rerr {
    let adrbts = tcp_dial_handshake_send_msg_and_read_all(
        addr, MSG_REQUEST_NEAREST_PUBLIC_NODES, 5).await?;
    if adrbts.len() < 1 {
        return errf!("data size error");
    }
        // println!("msg MSG_REQUEST_NEAREST_PUBLIC_NODES get {}", adrbts.hex());
    let sn = 6+16; // ip port + key
    let num = adrbts[0] as usize;
    if num < 1 {
        return Ok(()) // not find any
    }
    if num*sn != adrbts.len()-1 {
        return errf!("data size error");
    }
    let addrs = parse_public_nodes(&adrbts[1..]);
    for p in addrs {
        datas.insert(p.0, p.1);
    }
    // ok fnish
    Ok(())
}


