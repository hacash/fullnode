

impl MsgHandler {

    async fn send_hashs(&self, peer: Arc<Peer>, buf: Vec<u8>) {
        if buf.len() != 1+8 {
            return // error len
        }
        let hnum = buf[0] as u64;
        if hnum > 80 {
            return // max 80
        }
        let endhei = u64::from_be_bytes( bufcut!(buf, 1, 9) );
        // req
        let latest = self.engine.latest_block();
        let lathei = latest.height().uint();
        if endhei > lathei {
            return
        }
        let mut starthei = endhei - hnum;
        if hnum >= endhei {
            starthei = 1;
        }
        let stoptr = self.engine.disk();
        let store = BlockStore::wrap(stoptr);
        // load
        let mut reshxs = Vec::with_capacity((hnum + 8) as usize);
        reshxs.push( buf[1..9].to_vec() ); // endhei
        for hei in (starthei..=endhei).rev() {
            let curhx = store.block_hash(&BlockHeight::from(hei));
            if curhx.is_none() {
                return // not find block hash by height
            }
            reshxs.push( curhx.unwrap().to_vec() );
        }
        // return hashs to peer
        let _ = peer.send_msg(MSG_BLOCK_HASH, reshxs.concat()).await;
    }

    async fn receive_hashs(&self, peer: Arc<Peer>, mut buf: Vec<u8>) {
        // println!("&&&& receive_hashs = {}", hex::encode(&buf));
        if buf.len() < 8 {
            println!("check hash failed.");
            return
        }
        let hashs = buf.split_off(8);
        let end_hei = u64::from_be_bytes( bufcut!(buf, 0, 8) );
        let hash_len = hashs.len();
        if hash_len == 0 || hash_len % 32 != 0 {
            return // error len
        }
        let mut hash_num = hash_len as u64 / 32;
        // check
        let latest = self.engine.latest_block();
        let lathei = latest.height().uint();
        if end_hei > lathei {
            return // not find target height block
        }
        let dfhmax = self.engine.config().unstable_block as u64 + 1; 
        if hash_num > dfhmax {
            hash_num = dfhmax;
        }
        let mut start_hei = end_hei - hash_num;
        if end_hei <= hash_num {
            start_hei = 0; // first block
        }
        // println!("&&&& hash_len = {}, start_hei={}, end_hei={}, hash_num={}", hash_len, start_hei, end_hei, hash_num);
        // diff each blk hash
        let duskptr = self.engine.disk();
        let store = BlockStore::wrap(duskptr);
        let mut hi = 0;
        for hei in ((start_hei+1)..=end_hei).rev() {
            // println!("store.block_hash height = {}", hei);
            let myhx = store.block_hash(&BlockHeight::from(hei));
            if myhx.is_none() {
                println!("not find block hash by height = {}", hei);
                return // not find block hash by height
            }
            let myhx = myhx.unwrap();
            let hx = Fixed32::from( bufcut!(hashs, hi, hi+32) );
            // debug_println!("hei={}, hx={}, myhx={}", hei, hx, myhx);
            if hx == myhx {
                // sync blocks from next height
                // println!("&&&& get_status_try_sync_blocks receive_hashs myhx = {}, hei={} ...", myhx.hex(), hei+1);
                get_status_try_sync_blocks(self, peer, hei + 1).await;
                return // to sync new blocks
            }
            // next
            hi += 32;
        }
        // cannot sync fork!!!
    }

}




