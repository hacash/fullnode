


impl MsgHandler {

    pub async fn start(this: Arc<MsgHandler>, mut worker: Worker) {
        // let mut closech = this.exiter.signal();
        let mut blktxch = { 
            this.blktxch.lock().unwrap().take().unwrap()
        };
        loop {
            tokio::select! {
                // close signal
                _ = worker.wait() => {
                    break
                },
                // block tx arrived
                msg = blktxch.recv() => {
                    match msg.unwrap() {
                        BlockTxArrive::Tx(peer, tx) => handle_new_tx(this.clone(), peer, tx).await,
                        BlockTxArrive::Block(peer, blk) => handle_new_block(this.clone(), peer, blk).await,
                    }
                }
            }
        }
        println!("[MsgHandler] loop end.");
        worker.end();
    }
}



