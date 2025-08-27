

impl HacashNode {

    // 
    pub fn do_start(&self, worker: Worker) {

        let p2p = self.p2p.clone();
        let hdl = self.msghdl.clone();

        // diamond auto bid on mainnet
        // if this.engine.config().is_mainnet() {
        //     start_diamond_auto_bidding(this.clone());
        // }

        // handle msg
        let nwkr =  worker.fork();
        std::thread::spawn(move||{
            let rt = new_current_thread_tokio_rt();
            rt.block_on(async move {
                MsgHandler::start(hdl, nwkr).await
            });
        });

        // start p2p loop, blocking
        
        let is_multi_thread = self.cnf.multi_thread;
        let mut imtip = ".";
        if is_multi_thread {
            imtip = " with multi thread."
        }
        println!("[P2P] Start and listening on {}{}", self.cnf.listen, imtip);
        let _ = new_tokio_rt(is_multi_thread).block_on(async move {
            P2PManage::start(p2p, worker).await
        });
    }

}










