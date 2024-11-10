
#[macro_export]
macro_rules! s {
    ($v:expr) => { ($v).to_string() };
}



pub fn bytes_to_readable_string(bts: &[u8]) -> String {
    let ss: Vec<u8> = bts.iter().map(|x|match x {
        32..=126 => *x,
        _ => ' ' as u8,
    }).collect();
    let resstr = String::from_utf8(ss).ok().unwrap();
    resstr.trim_end().to_string()
}


pub fn bytes_from_readable_string(stuff: &[u8], len: usize) -> Ret<Vec<u8>> {
    let mut bts = Vec::with_capacity(len);
    let rs = stuff.to_vec();
    for i in 0..stuff.len() {
        if i >= len {
            break
        }
        let a = rs[i];
        bts.push(match a {
            32..=126 => a,
            _ => return Err("string cannot readable".to_string()),
        });
    }
    Ok(bts)
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

