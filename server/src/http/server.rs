
#[derive(Clone)]
pub struct HttpServer {
    cnf: ServerConf,
    engine: ChainEngine,
    hcshnd: ChainNode,
}


impl HttpServer {
    pub fn open(iniobj: &IniObj, eng: ChainEngine, nd: ChainNode) -> HttpServer {
        let cnf = ServerConf::new(iniobj);
        HttpServer{
            cnf: cnf,
            engine: eng,
            hcshnd: nd,
        }
    }

}

