//! Randomized cross-verification of the pure Rust x16rs against the C
//! reference (x16rs-sys). Requires the `c-hash` feature; run with:
//!
//! ```sh
//! cargo test -p x16rs --features c-hash --test cross_c --release
//! ```
//!
//! The fixed vectors in `pure_vectors.rs` cover 26 inputs per algorithm and
//! 26 chain inputs × 6 loop counts; this test hammers the full chain with
//! tens of thousands of pseudo-random inputs across loopnum -1..=64, which
//! exercises every algorithm on continuously varying random states, plus
//! exhaustive low loopnums on edge-pattern inputs to pin down behavior
//! drift (selector byte handling, loopnum <= 0 pass-through, wraparound in
//! the repeat counters).
#![cfg(feature = "c-hash")]

use x16rs::{pure, x16rs_hash_c};

/// xorshift64* — deterministic across platforms/runs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(2685821657736338717)
    }

    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

fn patterns() -> Vec<[u8; 32]> {
    let mut v = vec![[0u8; 32], [0xffu8; 32]];
    let mut asc = [0u8; 32];
    let mut desc = [0u8; 32];
    for i in 0..32usize {
        asc[i] = i as u8;
        desc[i] = (0xff - i) as u8;
    }
    v.push(asc);
    v.push(desc);
    // selector-byte corner cases: every nibble on byte 28, with the rest
    // zeroed and with the rest randomized
    for nib in 0u8..16 {
        let mut p = [0u8; 32];
        p[28] = nib;
        v.push(p);
        let mut q = [0xa5u8; 32];
        q[28] = nib << 4 | nib;
        v.push(q);
    }
    v
}

#[test]
fn cross_edge_patterns_all_low_loopnums() {
    for input in patterns() {
        for loopnum in -1i32..=64 {
            assert_eq!(
                pure::x16rs_hash(loopnum, &input),
                x16rs_hash_c(loopnum, &input),
                "input={input:02x?} loopnum={loopnum}"
            );
        }
    }
}

#[test]
fn cross_random_inputs_random_loopnums() {
    let mut rng = Rng(0x243F6A8885A308D3);
    for _ in 0..20_000 {
        let mut input = [0u8; 32];
        rng.fill(&mut input);
        let loopnum = (rng.next() % 65) as i32 - 1; // -1..=64
        assert_eq!(
            pure::x16rs_hash(loopnum, &input),
            x16rs_hash_c(loopnum, &input),
            "input={input:02x?} loopnum={loopnum}"
        );
    }
}

/// Mining-shaped runs: high loopnums on a rotating nonce space, the hot path
/// of `x16rs::block_hash` (repeat 1..=16) and diamond mining.
#[test]
fn cross_mining_shaped() {
    let mut rng = Rng(0xDEADBEEFCAFEBABE);
    for _ in 0..2_000 {
        let mut input = [0u8; 32];
        rng.fill(&mut input);
        for loopnum in 1..=16 {
            assert_eq!(
                pure::x16rs_hash(loopnum, &input),
                x16rs_hash_c(loopnum, &input)
            );
        }
    }
}

/// The pure chain must also agree with the C through the public wrapper for
/// loopnum 0 (identity copy) and the boundary between "no rounds" and one
/// round.
#[test]
fn cross_loopnum_zero_identity() {
    let mut input = [0u8; 32];
    input[0] = 0xab;
    input[28] = 0x1f;
    input[31] = 0xcd;
    assert_eq!(pure::x16rs_hash(0, &input), input);
    assert_eq!(x16rs_hash_c(0, &input), input);
    assert_eq!(pure::x16rs_hash(-7, &input), input);
    assert_eq!(x16rs_hash_c(-7, &input), input);
}

/// The `mining` module routes to the C implementation under `c-hash`; its
/// outputs must be identical to the always-pure top-level functions (this is
/// what makes "poworker mines with C, fullnode verifies with Rust" safe).
#[test]
fn cross_mining_module_matches_pure() {
    let mut rng = Rng(0x5EED_1234_ABCD_F00Du64);
    for _ in 0..200 {
        let height = rng.next() % 9_000_000;
        let mut stuff = [0u8; 96];
        rng.fill(&mut stuff);
        assert_eq!(
            x16rs::mining::block_hash(height, &stuff),
            x16rs::block_hash(height, &stuff)
        );

        let mut prev = [0u8; 32];
        let mut nonce = [0u8; 8];
        let mut addr = [0u8; 21];
        rng.fill(&mut prev);
        rng.fill(&mut nonce);
        rng.fill(&mut addr);
        let mut custom = vec![0u8; (rng.next() % 40) as usize];
        rng.fill(&mut custom);
        assert_eq!(
            x16rs::mining::mine_diamond(42_000, &prev, &nonce, &addr, &custom),
            x16rs::mine_diamond(42_000, &prev, &nonce, &addr, &custom)
        );
    }
}
