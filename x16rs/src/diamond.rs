/*
* "0WTYUIAHXVMEKBSZN"
*/
pub const DIAMOND_HASH_BASE_CHAR_NUM: usize = 17;
pub const DIAMOND_HASH_BASE_STRING: &str =       "0WTYUIAHXVMEKBSZN";
pub const DIAMOND_HASH_BASE_CHARS:  [u8; 17] = *b"0WTYUIAHXVMEKBSZN";
pub const DIAMOND_NAME_VALID_CHARS: [u8; 16] =  *b"WTYUIAHXVMEKBSZN";

const DMD_L: usize = 10;
const DMD_M: usize = 16;
const DMD_N: usize = DMD_M - DMD_L; // 6

pub fn _is_valid_diamond_name(v: &[u8]) -> bool {
    if v.len() != DMD_N {
        return false
    }
    // check in array
    for a in v {
        if ! DIAMOND_NAME_VALID_CHARS.iter().any(|x|x==a) {
            return false // invalid char
        }
    }
    // ok
    true
}


/*
*
*/
pub fn check_diamond_hash_result(stuff: impl AsRef<[u8]>) -> Option<[u8; DMD_N]> {
    let hxval = stuff.as_ref().to_vec();
    if hxval.len() != DMD_M {
        return None; // invalid legnth
    }
    for i in 0..DMD_L {
        if hxval[i] != b'0' {
            return None; // left 10 char must be '0'
        }
    }
    for i in DMD_L..DMD_M {
        if hxval[i] == b'0' {
            return None; // right 6 char must NOT be '0'
        }
        if ! DIAMOND_HASH_BASE_CHARS.iter().any(|x|*x==hxval[i]) {
            return None; // invalid char
        }
    }
    // ok
    Some(hxval[DMD_L..DMD_M].try_into().unwrap())
}

/*
*
*/
pub fn check_diamond_difficulty(number: u32, sha3hx: &[u8; H32S], x16rshx: &[u8; H32S]) -> bool {
    const MODIFFBITS: [u8; H32S] = [
        // difficulty requirements
        128, 132, 136, 140, 144, 148, 152, 156, // step +4
        160, 164, 168, 172, 176, 180, 184, 188, 192, 196, 200, 204, 208, 212, 216, 220, 224, 228,
        232, 236, 240, 244, 248, 252,
    ];
    // check step 1
    // referring to Moore's law, the excavation difficulty of every 42000 diamonds will double in about 2 years,
    // and the difficulty increment will tend to decrease to zero in 64 years
    // and increase the difficulty by 32-bit hash every 65536 diamonds
    let shnumlp = number as usize / 42000; // 32step max to 64 years
    let shmaxit = 255 - ((number / 65536) as u8);
    for i in 0..H32S {
        if i < shnumlp && sha3hx[i] >= MODIFFBITS[i] {
            return false; // fail
        }
        if sha3hx[i] > shmaxit {
            return false; // fail
        }
    }
    // check step 2
    // every 3277 diamonds is about 56 days. Adjust the difficulty 3277 = 16 ^ 6 / 256 / 20
    // when the difficulty is the highest, the first 20 bits of the hash are 0, not all 32 bits are 0.
    let mut diffnum = number as usize / 3277;
    for a in x16rshx {
        if diffnum < 255 {
            if (*a as usize) + diffnum > 255 {
                return false; // difficulty check failed
            } else {
                return true; // check success
            }
        } else if diffnum >= 255 {
            if *a != 0 {
                return false;
            }
            // to do next round check
            diffnum -= 255;
        }
    }
    // over loop
    false
}

/*
*
*/  
pub fn mine_diamond_hash_repeat(number: u32) -> i32 {
    // adjust the hashing times every 8192 diamonds (about 140 days and half a year)
    let repeat = number / 8192 + 1;
    return repeat as i32; // max 2048
}

/*
*
*/
pub fn diamond_hash(bshash: &[u8; H32S]) -> [u8; DMD_M] {
    let mut reshx = [0u8; DMD_M];
    let mut mgcidx: u32 = 13; // index number and magic num
    for i in 0..DMD_M {
        let num = mgcidx * (bshash[i * 2] as u32) * (bshash[i * 2 + 1] as u32);
        mgcidx = num % DIAMOND_HASH_BASE_CHAR_NUM as u32;
        reshx[i] = DIAMOND_HASH_BASE_CHARS[mgcidx as usize];
        if mgcidx == 0 {
            mgcidx = 13;
        }
    }
    // ok
    reshx
}

/*
*
*/
pub fn mine_diamond(
    number: u32,
    prevblockhash: &[u8; H32S],
    nonce: &[u8; 8],
    address: &[u8; 21],
    custom_message: impl AsRef<[u8]>,
) -> ([u8; H32S], [u8; H32S], [u8; DMD_M]) {
    // hash stuff
    let stuff = [
        prevblockhash.to_vec(),
        nonce.to_vec(),
        address.to_vec(),
        custom_message.as_ref().to_vec(),
    ].concat();
    // get ssshash by sha3 algrotithm
    let ssshash = calculate_hash(stuff); // SHA3
    // get diamond hash value by HashX16RS algorithm
    let repeat = mine_diamond_hash_repeat(number);
    let reshash = x16rs_hash(repeat, &ssshash);
    // get diamond name by DiamondHash function
    let diastr = diamond_hash(&reshash);
    // ok
    (ssshash, reshash, diastr)
}
