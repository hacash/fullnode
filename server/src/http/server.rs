
#[derive(Clone)]
pub struct RPCServer {
    cnf: ServerConf,
    engine: ChainEngine,
    hcshnd: ChainNode,
}


impl RPCServer {
    pub fn open(iniobj: &IniObj, eng: ChainEngine, nd: ChainNode) -> RPCServer {
        let cnf = ServerConf::new(iniobj);
        RPCServer{
            cnf: cnf,
            engine: eng,
            hcshnd: nd,
        }
    }

}

