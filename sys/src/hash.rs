use sha2::Sha256;
use sha3::{Digest, Sha3_256};
use ripemd::Ripemd160;

pub const H32S: usize = 32;


// sha3-256
pub fn sha3(data: impl AsRef<[u8]>) -> [u8; H32S] {
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let result: [u8; H32S] = result[..].try_into().unwrap();
    result
}


// sha2-256
pub fn sha2(data: impl AsRef<[u8]>) -> [u8; H32S] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let result: [u8; H32S] = result[..].try_into().unwrap();
    result
}


pub fn ripemd160(data: impl AsRef<[u8]>) -> [u8; 20] {
    let mut hasher = Ripemd160::new();
    hasher.update(data);
    // to [u8; 20]
    let result = hasher.finalize();
    let result: [u8; 20] = result[..].try_into().unwrap();
    result
}
