

impl P2PManage {

    pub async fn start(this: Arc<P2PManage>) -> RetErr {

        // start p2p listen
        // let p2p = this.clone();
        // tokio::spawn(async move{
        //     P2PManage::server_listen(p2p).await
        // });
        
        // connect boot nodes
        let p2p = this.clone();
        tokio::spawn(async move{
            p2p.connect_boot_nodes().await
        });

        // do once find nodes
        if this.cnf.findnodes {
            let p2p = this.clone();
            tokio::spawn(async move{
                asleep(15).await;
                p2p.find_nodes().await
            });
        }

        // event loop
        // this.event_loop().await;
        P2PManage::event_loop(this).await;
        Ok(())
    }


}
