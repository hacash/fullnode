use crate::{Ret, decodef};

pub fn start_with_char(s: &str, c: char) -> bool {
    !s.is_empty() && s.as_bytes()[0] == c as u8
}

pub fn bytes_to_readable_string(bts: &[u8]) -> String {
    let mut s = String::with_capacity(bts.len());
    for &b in bts {
        s.push(if (32..=126).contains(&b) {
            b as char
        } else {
            ' '
        });
    }
    s.trim_end().to_owned()
}

pub fn bytes_from_readable_string(stuff: &[u8], len: usize) -> Ret<Vec<u8>> {
    if stuff.len() != len {
        return decodef!(
            "readable string length mismatch: expected {} but got {}",
            len,
            stuff.len()
        );
    }
    let mut bts = vec![b' '; len];
    for (dst, &src) in bts.iter_mut().zip(stuff.iter()) {
        *dst = if (32..=126).contains(&src) { src } else { b' ' };
    }
    Ok(bts)
}

pub fn bytes_try_to_readable_string(bts: &[u8]) -> Option<String> {
    if !check_readable_string(bts) {
        return None;
    }
    Some(std::str::from_utf8(bts).ok()?.to_owned())
}

pub fn bytes_to_readable_string_or_hex(bts: &[u8]) -> String {
    match bytes_try_to_readable_string(bts) {
        Some(s) => s,
        None => hex::encode(bts),
    }
}

pub fn check_readable_string(bts: &[u8]) -> bool {
    bts.iter().all(|a| (32..=126).contains(a))
}

pub fn left_readable_string(bts: &[u8]) -> String {
    let end = bts
        .iter()
        .position(|a| !(32..=126).contains(a))
        .unwrap_or(bts.len());
    std::str::from_utf8(&bts[..end])
        .ok()
        .unwrap_or("")
        .trim_end()
        .to_owned()
}
