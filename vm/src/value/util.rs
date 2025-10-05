
// use std::*;
// use num_traits::FromBytes;

fn buf_not_zero(buf: &[u8]) -> bool {
    buf.iter().any(|a|*a>0)
}

#[allow(dead_code)]
fn buf_is_zero(buf: &[u8]) -> bool {
    ! buf_not_zero(buf)
}

pub fn buf_drop_left_zero(buf: &[u8], minl: usize) -> Vec<u8> {
    let n = buf.len();
    if n == 0 {
        return vec![]
    }
    let mut l = 0;
    let mut m = n;
    for i in 0..n {
        l = i;
        if buf[i] != 0 || m <= minl {
            break
        }
        m -= 1;
    }
    // ok
    buf[l..].into()
}

pub fn buf_fill_left_zero(buf: &[u8], zn: usize) -> Vec<u8> {
    let sz = buf.len();
    if sz >= zn {
        return buf[0..zn].into()
    }
    let res = buf[..].into();
    let pdn = zn - sz;
    [vec![0].repeat(pdn), res].concat()
}

