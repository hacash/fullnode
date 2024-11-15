
pub type IniObj = HashMap<String, HashMap<String, Option<String>>>;


pub fn join_path(a: &PathBuf, b: &str) -> PathBuf {
    let mut a = a.clone();
    a.push(b);
    a
}

pub fn ini_section(ini: &IniObj, key: &str) -> HashMap<String, Option<String>> {
    match ini.get(key) {
        Some(sec) => sec.clone(),
        None => HashMap::new(),
    }
}

pub fn ini_must(sec: &HashMap<String, Option<String>>, key: &str, def: &str) -> String {
    ini_must_maxlen(sec, key, def, 0)
}

pub fn ini_must_maxlen(sec: &HashMap<String, Option<String>>, key: &str, def: &str, ml: usize) -> String {
    let mut val = match sec.get(key) {
        Some(Some(val)) => val.to_string(),
        Some(None) => def.to_string(),
        None => def.to_string(),
    };
    if ml > 0 {
        val.truncate(ml);
    }
    val
}

pub fn ini_must_u64(sec: &HashMap<String, Option<String>>, key: &str, dv: u64) -> u64 {
    let val = ini_must(sec, key, &dv.to_string());
    match val.parse::<u64>() {
        Ok(n) => n,
        Err(_) => dv,
    }
}

pub fn ini_must_f64(sec: &HashMap<String, Option<String>>, key: &str, dv: f64) -> f64 {
    let val = ini_must(sec, key, &dv.to_string());
    match val.parse::<f64>() {
        Ok(n) => n,
        Err(_) => dv,
    }
}

pub fn ini_must_bool(sec: &HashMap<String, Option<String>>, key: &str, dv: bool) -> bool {
    let mut dfv = "false";
    if dv {
        dfv = "true";
    }
    let val = ini_must(sec, key, dfv);
    match val.as_str() {
        "_" => false,
        "false" => false,
        "False" => false,
        "FALSE" => false,
        "0" => false,
        _ => true,
    }
}


pub fn ini_must_account(sec: &HashMap<String, Option<String>>, key: &str) -> Account {
    let pass = ini_must(sec, key, "123456");
    let Ok(acc) = Account::create_by(&pass) else {
        panic!("[Config Error] account password {} error.", &pass)
    };
    acc
}
