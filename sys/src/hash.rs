//! Hash helpers: sha3-256 for consensus hashes and PoW, sha2-256 for
//! node ids / account secrets. No C dependency (recode.loc.md goal #2).

use sha2::Sha256;
use sha3::{Digest, Sha3_256};

pub const HASH_SIZE: usize = 32;

/// sha3-256 PoW
pub fn calculate_hash(data: impl AsRef<[u8]>) -> [u8; HASH_SIZE] {
    let mut hasher = Sha3_256::new();
    hasher.update(data.as_ref());
    let out = hasher.finalize();
    let mut buf = [0u8; HASH_SIZE];
    buf.copy_from_slice(&out);
    buf
}

/// sha2-256 (node.id generation, account secrets, etc.)
pub fn sha2_256(data: impl AsRef<[u8]>) -> [u8; HASH_SIZE] {
    let out = Sha256::digest(data.as_ref());
    let mut buf = [0u8; HASH_SIZE];
    buf.copy_from_slice(&out);
    buf
}
