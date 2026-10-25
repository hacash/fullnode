use ripemd::Ripemd160;
use sha2::Sha256;
use sha3::{Digest, Sha3_256};

pub mod pure;

pub const H32S: usize = 32;

/// Compute the X16RS hash (pure Rust; bit-compatible with the historical C
/// implementation — verified against `tests/data/x16rs_vectors.bin`).
///
/// `loopnum <= 0` copies the input through, matching the C loop
/// `for (n = 0; n < loopnum; n++)`.
pub fn x16rs_hash(loopnum: i32, indata: &[u8; H32S]) -> [u8; H32S] {
    pure::x16rs_hash(loopnum, indata)
}

/// The original C implementation, for cross-verification tests and
/// performance benchmarks only. Requires the `c-hash` feature.
#[cfg(feature = "c-hash")]
pub use x16rs_sys::x16rs_hash as x16rs_hash_c;

pub const DIAMOND_HASH_BASE_CHAR_NUM: usize = 17;
pub const DIAMOND_HASH_BASE_STRING: &str = "0WTYUIAHXVMEKBSZN";
pub const DIAMOND_HASH_BASE_CHARS: [u8; 17] = *b"0WTYUIAHXVMEKBSZN";
pub const DIAMOND_NAME_VALID_CHARS: [u8; 16] = *b"WTYUIAHXVMEKBSZN";

const DMD_L: usize = 10;
const DMD_M: usize = 16;
const DMD_N: usize = DMD_M - DMD_L;

pub fn sha3(data: impl AsRef<[u8]>) -> [u8; H32S] {
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    hasher.finalize()[..].try_into().unwrap()
}

pub fn sha2(data: impl AsRef<[u8]>) -> [u8; H32S] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()[..].try_into().unwrap()
}

pub fn ripemd160(data: impl AsRef<[u8]>) -> [u8; 20] {
    let mut hasher = Ripemd160::new();
    hasher.update(data);
    hasher.finalize()[..].try_into().unwrap()
}

pub fn calculate_hash(data: impl AsRef<[u8]>) -> [u8; H32S] {
    sha3(data)
}

pub fn hash(height: u64, data: &[u8]) -> [u8; H32S] {
    block_hash(height, data)
}

pub fn block_hash_repeat(height: u64) -> i32 {
    (height / 50_000 + 1).min(16) as i32
}

pub fn block_hash(height: u64, stuff: &[u8]) -> [u8; H32S] {
    let repeat = block_hash_repeat(height);
    let reshash = calculate_hash(stuff);
    x16rs_hash(repeat, &reshash)
}

/// Hash entry points for the *mining* paths (poworker CPU workers). With the
/// `c-hash` feature these route to the original C (sphlib) implementation;
/// without it they fall back to the pure Rust one. Outputs are bit-identical
/// (cross-verified by `tests/cross_c.rs`), so only speed differs.
///
/// Block/diamond *verification* must keep using the top-level functions
/// (`block_hash`, `mine_diamond`), which are always pure Rust.
pub mod mining {
    use super::*;

    pub fn block_hash(height: u64, stuff: &[u8]) -> [u8; H32S] {
        #[cfg(feature = "c-hash")]
        {
            let repeat = block_hash_repeat(height);
            let reshash = calculate_hash(stuff);
            x16rs_hash_c(repeat, &reshash)
        }
        #[cfg(not(feature = "c-hash"))]
        {
            super::block_hash(height, stuff)
        }
    }

    pub fn mine_diamond(
        number: u32,
        prevblockhash: &[u8; H32S],
        nonce: &[u8; 8],
        address: &[u8; 21],
        custom_message: impl AsRef<[u8]>,
    ) -> ([u8; H32S], [u8; H32S], [u8; DMD_M]) {
        #[cfg(feature = "c-hash")]
        {
            let custom = custom_message.as_ref();
            let mut stuff = Vec::with_capacity(
                prevblockhash.len() + nonce.len() + address.len() + custom.len(),
            );
            stuff.extend_from_slice(prevblockhash);
            stuff.extend_from_slice(nonce);
            stuff.extend_from_slice(address);
            stuff.extend_from_slice(custom);
            let seed_hash = calculate_hash(stuff);
            let repeat = mine_diamond_hash_repeat(number);
            let result_hash = x16rs_hash_c(repeat, &seed_hash);
            let diamond_str = diamond_hash(&result_hash);
            (seed_hash, result_hash, diamond_str)
        }
        #[cfg(not(feature = "c-hash"))]
        {
            super::mine_diamond(number, prevblockhash, nonce, address, custom_message)
        }
    }
}

/// OpenCL (GPU) mining host code; the kernels live in `opencl/*.cl`. The
/// submodules require the `ocl` feature (native builds only).
pub mod opencl;

/// Diamond "more power" comparison: `src` beats `dst` when it replaces a
/// leading '0' with a non-'0' character. Shared by the CPU and OpenCL miners.
pub fn diamond_more_power(dst: &[u8], src: &[u8]) -> bool {
    for i in 0..16 {
        let (l, r) = (dst[i], src[i]);
        if l == b'0' && r != b'0' {
            return true;
        }
        if l != b'0' && r == b'0' {
            return false;
        }
        if l != b'0' && r != b'0' {
            return false;
        }
    }
    false
}

pub fn is_valid_diamond_name(v: &[u8]) -> bool {
    if v.len() != DMD_N {
        return false;
    }
    v.iter()
        .all(|a| DIAMOND_NAME_VALID_CHARS.iter().any(|x| x == a))
}

pub fn check_diamond_hash_result(stuff: impl AsRef<[u8]>) -> Option<[u8; DMD_N]> {
    let hxval = stuff.as_ref();
    if hxval.len() != DMD_M {
        return None;
    }
    for a in &hxval[..DMD_L] {
        if *a != b'0' {
            return None;
        }
    }
    for a in &hxval[DMD_L..DMD_M] {
        if *a == b'0' || !DIAMOND_HASH_BASE_CHARS.contains(a) {
            return None;
        }
    }
    Some(hxval[DMD_L..DMD_M].try_into().unwrap())
}

pub fn check_diamond_difficulty(number: u32, sha3hx: &[u8; H32S], x16rshx: &[u8; H32S]) -> bool {
    const MODIFFBITS: [u8; H32S] = [
        128, 132, 136, 140, 144, 148, 152, 156, 160, 164, 168, 172, 176, 180, 184, 188, 192, 196,
        200, 204, 208, 212, 216, 220, 224, 228, 232, 236, 240, 244, 248, 252,
    ];

    let sha3_required_leading = number as usize / 42_000;
    let sha3_max_byte = 255 - (number / 65_536).min(255) as u8;
    for i in 0..H32S {
        if i < sha3_required_leading && sha3hx[i] >= MODIFFBITS[i] {
            return false;
        }
        if sha3hx[i] > sha3_max_byte {
            return false;
        }
    }

    let mut remaining_diff = number as usize / 3277;
    for a in x16rshx {
        if remaining_diff < 255 {
            if (*a as usize) + remaining_diff > 255 {
                return false;
            }
            return true;
        } else if *a != 0 {
            return false;
        } else {
            remaining_diff -= 255;
        }
    }
    false
}

pub fn mine_diamond_hash_repeat(number: u32) -> i32 {
    (number / 8192 + 1) as i32
}

pub fn diamond_hash(bshash: &[u8; H32S]) -> [u8; DMD_M] {
    let mut result_hash = [0u8; DMD_M];
    let mut base_idx: u32 = 13;
    for i in 0..DMD_M {
        let product = base_idx * (bshash[i * 2] as u32) * (bshash[i * 2 + 1] as u32);
        base_idx = product % DIAMOND_HASH_BASE_CHAR_NUM as u32;
        result_hash[i] = DIAMOND_HASH_BASE_CHARS[base_idx as usize];
        if base_idx == 0 {
            base_idx = 13;
        }
    }
    result_hash
}

pub fn mine_diamond(
    number: u32,
    prevblockhash: &[u8; H32S],
    nonce: &[u8; 8],
    address: &[u8; 21],
    custom_message: impl AsRef<[u8]>,
) -> ([u8; H32S], [u8; H32S], [u8; DMD_M]) {
    let custom = custom_message.as_ref();
    let mut stuff =
        Vec::with_capacity(prevblockhash.len() + nonce.len() + address.len() + custom.len());
    stuff.extend_from_slice(prevblockhash);
    stuff.extend_from_slice(nonce);
    stuff.extend_from_slice(address);
    stuff.extend_from_slice(custom);
    let seed_hash = calculate_hash(stuff);
    let repeat = mine_diamond_hash_repeat(number);
    let result_hash = x16rs_hash(repeat, &seed_hash);
    let diamond_str = diamond_hash(&result_hash);
    (seed_hash, result_hash, diamond_str)
}
