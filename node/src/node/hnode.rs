

impl HNode for HacashNode {

    fn submit_transaction(&self, txpkg: &TxPkg, in_async: bool) -> Rerr {
        // check signature
        let txread = txpkg.objc.as_read();
        // txread.verify_signature()?;
        // try execute tx
        self.engine.try_execute_tx(txread)?;
        // add to pool
        let msghdl = self.msghdl.clone();
        let txbody = txpkg.data.clone();
        let runobj = async move {
            msghdl.submit_transaction(txbody).await;
        };
        if in_async {
            tokio::spawn(runobj);
        }else{
            new_current_thread_tokio_rt().block_on(runobj);
        }
        Ok(())
    }


    fn submit_block(&self, blkpkg: &BlockPkg, in_async: bool) -> Rerr {
        // NOT do any check
        // insert
        let msghdl = self.msghdl.clone();
        let blkbody = blkpkg.data.clone();
        let runobj = async move {
            msghdl.submit_block(blkbody).await;
        };
        if in_async {
            tokio::spawn(runobj);
        }else{
            new_current_thread_tokio_rt().block_on(runobj);
        }
        Ok(())
    }

    fn engine(&self) -> Arc<dyn Engine> {
        self.engine.clone()
    }

    fn txpool(&self) -> Arc<dyn TxPool> {
        self.txpool.clone()
    }

    fn all_peer_prints(&self) -> Vec<String> { 
        self.p2p.all_peer_prints()
    }

    fn exit(&self) {
        self.p2p.exit();
        self.msghdl.exit();
        // wait something to finish
        // std::thread::sleep(std::time::Duration::from_secs_f32(0.5));
    }

}
