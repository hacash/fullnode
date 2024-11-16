

impl MsgHandler {

    async fn send_blocks(&self, peer: Arc<Peer>, mut buf: Vec<u8>) {
        if buf.len() != 8 {
            return // error len
        }
        let starthei = u64::from_be_bytes( bufcut!(buf, 0, 8) );
        let latest = self.engine.latest_block();
        let lathei = latest.objc().height().uint();
	    let maxsendsize = 1024 * 1024 * 20usize; // max 20MB
	    let maxsendnum = 10000usize; // max 10000
        let mut totalsize = 0;
        let mut totalnum = 0;
        let mut endhei = 0;
        // load data
        let stoptr = self.engine.store();
        let store = CoreStoreDisk::wrap(stoptr.as_ref());
        let mut blkdtsary = vec![];
        for hei in starthei ..= lathei {
            let blkdts = store.blockdatabyptr(&BlockHeight::from(hei));
            if blkdts.is_none() {
                return // not find block hash by height
            }
            let dts = blkdts.unwrap().into_vec();
            totalsize += dts.len();
            totalnum += 1;
            endhei = hei;
            blkdtsary.push( dts );
            if totalnum >= maxsendnum || totalsize >= maxsendsize {
                break // chunk finish
            }
        }
        let resblkdts = blkdtsary.concat();
        // ret
        let msgbody = vec![
            lathei.to_be_bytes().to_vec(),
            starthei.to_be_bytes().to_vec(),
            endhei.to_be_bytes().to_vec(),
            resblkdts,
        ].concat();
        // return blocks to peer
        peer.send_msg(MSG_BLOCK, msgbody).await;
    }
    
    async fn receive_blocks(&self, peer: Arc<Peer>, mut buf: Vec<u8>) {
        if buf.len() < 3 * 8 {
            println!("check data failed.");
            return
        }
        let blocks = buf.split_off(3*8);
        let latest_hei = u64::from_be_bytes( bufcut!(buf, 0, 8) );
        let start_hei = u64::from_be_bytes( bufcut!(buf, 8, 16) );
        let end_hei = u64::from_be_bytes( bufcut!(buf, 16, 24) );
        let persent =  end_hei as f64 / latest_hei as f64 * 100.0;
        flush!("{}({:.2}%) inserting...", end_hei, persent);
        // try insert
        let engine = self.engine.clone();
        let res = tokio::task::spawn_blocking(move || {
            engine.insert_sync(start_hei, blocks)
        }).await.unwrap();
        if let Err(e) = res {
            println!("{}", e); // show err
            return
        }
        println!("ok.");
        if end_hei >= latest_hei {
            println!("all blocks sync finished.");
            return
        }
        // sync more blocks
        let peer = self.switch_peer(peer);
        send_req_block_msg(self, peer, end_hei+1).await
    }


}
