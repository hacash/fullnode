


pub struct HacashNode {
    cnf: NodeConf,
    engine: Arc<dyn Engine>,
    txpool: Arc<MemTxPool>,
    p2p: Arc<P2PManage>,
    msghdl: Arc<MsgHandler>,
}


impl HacashNode {

    pub fn open(ini: &IniObj, engine: Arc<dyn Engine>) -> HacashNode {
        let cnf = NodeConf::new(ini);
        // tx pool
        let mut tpmaxs = vec![5000, 200];
        cover(&mut tpmaxs, &cnf.txpool_maxs);
        let txpool = Arc::new(MemTxPool::new(tpmaxs));
        let msghdl = Arc::new(MsgHandler::new(engine.clone(), txpool.clone()));
        let p2p = Arc::new(P2PManage::new(&cnf, msghdl.clone()));
        msghdl.set_p2p_mng(Box::new(PeerMngInst::new(p2p.clone())));

        HacashNode{
            cnf: cnf,
            engine: engine,
            txpool: txpool.clone(),
            p2p: p2p,
            msghdl: msghdl,
        }
    }

    pub fn close(&self) {
        self.p2p.close();
        self.msghdl.close();
    }

}