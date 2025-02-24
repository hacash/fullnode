

impl HacashNode {

    // 
    pub fn start(this: Arc<HacashNode> ) {

        let p2p = this.p2p.clone();
        let hdl = this.msghdl.clone();

        // diamond auto bid
        start_diamond_auto_bidding(this.clone());

        // handle msg
        std::thread::spawn(move||{
            let rt = new_current_thread_tokio_rt();
            rt.block_on(async move {
                MsgHandler::start(hdl).await
            });
        });

        // start p2p loop, blocking
        
        let is_multi_thread = this.cnf.multi_thread;
        let mut imtip = ".";
        if is_multi_thread {
            imtip = " with multi thread."
        }
        println!("[P2P] Start and listening on {}{}", this.cnf.listen, imtip);
        let _ = new_tokio_rt(is_multi_thread).block_on(async{
            P2PManage::start(p2p).await
        });
    }

}










