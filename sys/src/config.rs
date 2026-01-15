

// pub type IniObj = HashMap<String, HashMap<String, Option<String>>>;


pub fn get_current_exe_absolute_dir(dir: &str) -> PathBuf {
    let mut ddrp = PathBuf::from(&dir);
    // println!("{:?} {}", ddrp, ddrp.is_absolute());
    if ! ddrp.is_absolute() {
        ddrp = std::env::current_exe().unwrap().parent().unwrap().to_path_buf().join(&dir);
    }
    ddrp
}

/*
* get data path
*/
pub fn get_mainnet_data_dir(ini: &IniObj) -> PathBuf {

    let sec = ini_section(ini, "default"); // default = root
    let data_dir = ini_must(&sec, "data_dir", "hacash_mainnet_data");

    get_current_exe_absolute_dir(&data_dir)
}


/*
* load config
*/
pub fn load_config(mut cnfilestr: String) -> IniObj {

    // let mut cnfilestr = "./hacash.config.ini".to_string();

    // exec dir
    let execdir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let mut cnf_file = execdir.join(&cnfilestr);

    // cmd args
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        cnfilestr = args[1].clone();
        cnf_file = PathBuf::from(&cnfilestr);
    }

    // check exists
    if ! cnf_file.exists() {
        // error
        println!("[Config Error] Cannot find config file {}", cnfilestr);
        return IniObj::new()
    }

    cnfilestr = cnf_file.canonicalize().unwrap().to_str().unwrap().to_string();
    // println!("{:?} {:?}", args, exedir);
    println!("[Config] Load: {} {}.", cnfilestr, ctshow());
    
    // load file
    ini::ini!(&cnfilestr)

}
