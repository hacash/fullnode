//! Pure Rust implementation of the X16RS hash.
//!
//! This module replaces the C implementation (sphlib, shipped via
//! `x16rs-sys`) bit-for-bit. The C reference always hashes exactly one
//! 32-byte state per round, so every algorithm here is specialized to that
//! shape: one padded 32-byte message, one finalization, a full 64-byte
//! digest out. The chain keeps only the first 32 digest bytes between
//! rounds, and selects the next algorithm with the low nibble of the last
//! state byte — identical to `c_x16rs_hash` in x16rs-sys `src/x16rs.c`.
//!
//! Porting contract for every algorithm (enforced by the vector tests in
//! `tests/pure_vectors.rs`, generated from the C reference — see
//! `gen/gen_vectors.c`):
//!
//! * faithful to the `SPH_LITTLE_ENDIAN` + `SPH_64` sphlib code path;
//! * no allocation, no `unsafe`, no panics on any input: all arithmetic on
//!   fixed-size arrays, `wrapping_*` everywhere C relies on unsigned wrap;
//! * single public entry point `pub fn hash64(msg: &[u8; 32]) -> [u8; 64]`;
//!   init constants, 32-byte padding and digest serialization are folded in
//!   (no streaming context structs).

pub mod blake;
pub mod bmw;
pub mod cubehash;
pub mod echo;
pub mod fugue;
pub mod groestl;
pub mod hamsi;
pub mod jh;
pub mod keccak;
pub mod luffa;
pub mod shabal;
pub mod shavite;
pub mod sha512;
pub mod simd;
pub mod skein;
pub mod whirlpool;

/// Digest size in bytes, same value as the C binding's `H32S`.
pub const H32S: usize = 32;

/// Pure Rust X16RS hash. Drop-in replacement for `c_x16rs_hash` /
/// `x16rs_sys::x16rs_hash`: `loopnum <= 0` copies the input through,
/// matching the C loop `for (n = 0; n < loopnum; n++)`.
#[inline]
pub fn x16rs_hash(loopnum: i32, indata: &[u8; H32S]) -> [u8; H32S] {
    let mut st = *indata;
    for _ in 0..loopnum {
        st = round(st);
    }
    st
}

#[inline]
fn round(st: [u8; H32S]) -> [u8; H32S] {
    // The C reads `inputoutput[7] % 16`: the little-endian u32 over bytes
    // 28..32, so the selector is the low nibble of byte 28.
    let d = match st[28] & 15 {
        0 => blake::hash64(&st),
        1 => bmw::hash64(&st),
        2 => groestl::hash64(&st),
        3 => jh::hash64(&st),
        4 => keccak::hash64(&st),
        5 => skein::hash64(&st),
        6 => luffa::hash64(&st),
        7 => cubehash::hash64(&st),
        8 => shavite::hash64(&st),
        9 => simd::hash64(&st),
        10 => echo::hash64(&st),
        11 => hamsi::hash64(&st),
        12 => fugue::hash64(&st),
        13 => shabal::hash64(&st),
        14 => whirlpool::hash64(&st),
        _ => sha512::hash64(&st),
    };
    let mut out = [0u8; H32S];
    out.copy_from_slice(&d[..H32S]);
    out
}
