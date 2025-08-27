pub const USIZE_WIDTH: u32 = usize::BITS / 8;


pub const UINT_MAX_W1: u64 = 256 - 1;
pub const UINT_MAX_W2: u64 = 256 * 256 - 1;
pub const UINT_MAX_W3: u64 = 256 * 256 * 256 - 1;
pub const UINT_MAX_W4: u64 = u32::MAX as u64;
pub const UINT_MAX_W5: u64 = 256 * 256 * 256 * 256 * 256 - 1;
pub const UINT_MAX_W6: u64 = 256 * 256 * 256 * 256 * 256 * 256 - 1;
pub const UINT_MAX_W7: u64 = 256 * 256 * 256 * 256 * 256 * 256 * 256 - 1;
pub const UINT_MAX_W8: u64 = u64::MAX;
pub const UINT_MAX_DEFS: [u64; 9] = [
    0u64,
    UINT_MAX_W1,
    UINT_MAX_W2,
    UINT_MAX_W3,
    UINT_MAX_W4,
    UINT_MAX_W5,
    UINT_MAX_W6,
    UINT_MAX_W7,
    UINT_MAX_W8,
];


// bytes <=> uint common fn

pub fn bytes_to_uint(buf: &[u8], msz: usize, len: usize) -> Ret<u64> {
    if len > msz || len > 8 {        
        return Err("size cannot over ".to_owned() + &msz.to_string())
    }
    let mut vbts = [0u8; 8];
    let left = 8 - len;
    let mut i = 0;
    for k in left..8 {
        vbts[k] = buf[i];
        i += 1;
    }
    // println!("{} {} {} {}", msz, len, hex::encode(buf), hex::encode(vbts));
    Ok(u64::from_be_bytes(vbts))
}

pub fn bytes_from_uint(val: u64, msz: usize, len: usize) -> Ret<Vec<u8>> {
    if len > msz {
        return Err("size cannot over ".to_owned() + &msz.to_string())
    }
    let rlbt = val.to_be_bytes();
    let left = 8 - len;
    Ok(rlbt[left..8].to_vec())
}


/**
* number set in range
*/
#[macro_export]
macro_rules! set_in_range { ($n: expr, $a: expr, $b: expr) => {
    $n = match $n {
        0..=$a => $a, 
        $a..=$b => $n,
        _ => $b,
    };
} }