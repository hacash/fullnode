
#[derive(Clone, Copy)]
pub struct ServerConf {
    pub enable: bool,
    pub listen: u16,
    pub multi_thread: bool,
}



impl  ServerConf {
    
    pub fn new(ini: &IniObj) -> ServerConf {
        let sec = ini_section(ini, "server");
        let cnf = ServerConf{
            enable:       ini_must_bool(&sec, "enable", false),
            listen:   ini_must_u64(&sec, "listen", 8083) as u16,
            multi_thread: ini_must_bool(&sec, "multi_thread", false),
        };

        cnf
    }


}
