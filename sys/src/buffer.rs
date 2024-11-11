
#[macro_export]
macro_rules! bufcut {
    ( $buf:expr, $l:expr, $r:expr ) => { 
        $buf[$l..$r].try_into().unwrap()
    };
}


/*
*
*/
pub fn bufeat(buf: &[u8], n: usize) -> Ret<Vec<u8>> {
    let buflen = buf.len();
    match n > buflen {
        false => Ok(buf[..n].to_vec()), // ok clone
        true => {
            let n1 = n.to_string();
            let n2 = buflen.to_string();
            Err("buf length too short need ".to_owned()+&n1+" but got "+&n2)
        }
    }
}


/*
* 
*/
pub fn bufeatone(buf: &[u8],) -> Ret<u8> {
    match buf.len() >= 1 {
        true => Ok(buf[0]),
        false => Err(s!("buf length too short"))
    }
}

