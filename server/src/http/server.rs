
#[derive(Clone)]
pub struct HttpServer {
    cnf: ServerConf,
    engine: Arc<dyn Engine>,
    hcshnd: Arc<dyn HNoder>,
}


impl HttpServer {
    pub fn open(iniobj: &IniObj, hnd: Arc<dyn HNoder>) -> Self {
        let cnf = ServerConf::new(iniobj);
        Self{
            cnf: cnf,
            engine: hnd.engine(),
            hcshnd: hnd,
        }
    }

}


impl Server for HttpServer {
    fn start(&self, worker: Worker) {
        self.do_start(worker)
    }
}

