//! Keccak-512 via the RustCrypto `sha3` crate's `Keccak512` (original Keccak
//! 0x01 padding — sphlib's `sph_keccak512` predates SHA-3 and is NOT
//! SHA3-512). Equivalence with the C reference is checked by
//! `tests/pure_vectors.rs` (`keccak512`).

/// Full 64-byte Keccak-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    use sha3::Digest;
    let mut h = sha3::Keccak512::new();
    h.update(msg);
    h.finalize()[..].try_into().unwrap()
}
