

#[allow(dead_code)]
pub struct HacashNode {
    cnf: NodeConf,
    engine: Arc<dyn Engine>,
    txpool: Arc<dyn TxPool>,
    p2p: Arc<P2PManage>,
    msghdl: Arc<MsgHandler>,
}


impl HacashNode {

    pub fn open(ini: &IniObj, txpool: Arc<dyn TxPool>, engine: Arc<dyn Engine>) -> Self {
        let cnf = NodeConf::new(ini);
        let msghdl = Arc::new(MsgHandler::new(engine.clone(), txpool.clone()));
        let p2p = Arc::new(P2PManage::new(&cnf, msghdl.clone()));
        msghdl.set_p2p_mng(Box::new(PeerMngInst::new(p2p.clone())));
        Self{
            cnf,
            engine,
            txpool,
            p2p,
            msghdl,
        }
    }

}