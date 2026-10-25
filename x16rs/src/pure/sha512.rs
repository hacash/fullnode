//! SHA-512 via the RustCrypto `sha2` crate: sphlib's `sph_sha512` is
//! standard SHA-512, and the `sha2` crate additionally uses SHA-NI on
//! x86-64. Equivalence with the C reference is checked by
//! `tests/pure_vectors.rs` (`sha512t`).

/// Full 64-byte SHA-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    use sha2::Digest;
    let mut h = sha2::Sha512::new();
    h.update(msg);
    h.finalize()[..].try_into().unwrap()
}
