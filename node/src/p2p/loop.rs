

impl P2PManage {

    pub async fn event_loop(this: Arc<P2PManage>) -> Rerr {
        let mut printpeer_tkr = new_ticker(60*97).await; // 97mins print peers
        let mut reconnect_tkr = new_ticker(51*33).await; // 30mins check reconnect
        let mut findnodes_tkr = new_ticker(52*60*4).await; // 4hour find nodes or boot
        let mut checkpeer_tkr = new_ticker(53*3).await; // 3mins ping all no active nodes
        let mut boostndes_tkr = new_ticker(54*5).await; // 5mins boost public nodes form offshoots table

        let server_listener = this.server().await;
        let mut closech = this.exiter.signal();

        loop {
            tokio::select! {
                _ = closech.recv() => {
                    break
                }
                _ = printpeer_tkr.tick() => {
                    this.print_conn_peers();
                },
                _ = reconnect_tkr.tick() => {
                    let no_nodes = this.backbones().len() < 2;
                    if no_nodes && this.cnf.findnodes {
                        let _ = this.connect_boot_nodes().await; // connect boots
                    }
                },
                _ = findnodes_tkr.tick() => {
                    if this.cnf.findnodes {
                        this.find_nodes().await; // do find nodes
                    }
                },
                _ = checkpeer_tkr.tick() => {
                    this.check_active_nodes().await; // do check no active
                    this.ping_nodes().await; // do ping all nodes
                },
                _ = boostndes_tkr.tick() => {
                    this.boost_public().await;
                    if this.backbones().len() == 0 {
                        let _ = this.connect_boot_nodes().await; // connect boots
                    }
                },
                client = server_listener.accept() => {
                    let Ok((client, _)) = errunbox!( client ) else {
                        continue
                    };
                    let tobj = this.clone();
                    tokio::spawn(async move {
                        tobj.handle_conn(client, false).await // not report me
                    });
                },
                else => break
            }
        }
        // println!("[P2P] event loop end.");
        // close all peer
        this.disconnect_all_peers().await;
        Ok(())
    }

}

