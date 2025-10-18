
#[macro_export]
macro_rules! s {
    ($v:expr) => { ($v).to_string() };
}


pub fn start_with_char(s: &str, c: char) -> bool {
    match s.len() > 0 {
        true => s.as_bytes()[0] == c as u8,
        _ => false
    }
}

pub fn bytes_to_readable_string(bts: &[u8]) -> String {
    let ss: Vec<u8> = bts.iter().map(|x|match x {
        32..=126 => *x,
        _ => ' ' as u8,
    }).collect();
    let resstr = String::from_utf8(ss).ok().unwrap();
    resstr.trim_end().to_string()
}


pub fn bytes_from_readable_string(stuff: &[u8], len: usize) -> Vec<u8> {
    let ept = ' ' as u8;
    let mut bts = vec![ept; len];
    for i in 0..stuff.len() {
        if i >= len {
            break
        }
        bts[i] = match stuff[i] {
            a @ 32..=126 => a,
            _ => ept,
        };
    }
    bts
}

pub fn bytes_try_to_readable_string(bts: &[u8]) -> Option<String> {
    if false == check_readable_string(bts) {
        return None
    }
    let resstr = String::from_utf8(bts.to_vec()).ok().unwrap();
    Some(resstr.to_string())
}


pub fn bytes_to_readable_string_or_hex(bts: &[u8]) -> String {
    match check_readable_string(bts) {
        true => String::from_utf8(bts.to_vec()).ok().unwrap().to_string(),
        false => hex::encode(bts),
    }
}


pub fn check_readable_string(bts: &[u8]) -> bool {
    for a in bts {
        if *a<32 || *a>126 {
            return false // cannot read
        }
    }
    return true
}


pub fn left_readable_string(bts: &[u8]) -> String {
    let mut ss: Vec<u8> = vec![];
    for a in bts {
        if *a<32 || *a>126 {
            break // end
        }
        ss.push(*a);
    }
    String::from_utf8(ss).ok().unwrap().trim_end().to_string()
}

