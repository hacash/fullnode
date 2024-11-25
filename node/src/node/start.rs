

impl HacashNode {

    // 
    pub fn start(this: Arc<HacashNode> ) {

        let rt = new_current_thread_tokio_rt();
        let p2p = this.p2p.clone();
        let hdl = this.msghdl.clone();

        // diamond auto bid
        // start_diamond_auto_bidding(this.clone());

        // handle msg
        std::thread::spawn(move||{
            let rt = new_current_thread_tokio_rt();
            rt.block_on(async move {
                MsgHandler::start(hdl).await
            });
        });

        // start p2p loop, blocking
        println!("[P2P] Start and listening on {}.", this.cnf.listen);
        let _ = rt.block_on(async{
            P2PManage::start(p2p).await
        });
    }

}











/*


        // test
        for n in 0..256u8 {
            let b = 100;
            println!("{}<->{}: {}", b, n, super::p2p::calculate_one_byte_topology_distance(b, n));
        }
        panic!("{}", "test end");



*/