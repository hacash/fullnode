#![no_std]
#![no_main]
use tiny_keccak::{Hasher, Keccak};

fn kernel(input: &[u8], out: &mut [u8]) -> usize {
    let mode = input.first().copied().unwrap_or(0);
    let dig = match mode {
        1 => merkle(16),
        2 => merkle(32),
        _ => hash_n(32),
    };
    out[..32].copy_from_slice(&dig);
    32
}

fn hash_n(n: usize) -> [u8; 32] {
    let mut buf = [0u8; 64];
    let mut i = 0;
    while i < n && i < buf.len() {
        buf[i] = i as u8;
        i += 1;
    }
    let mut k = Keccak::v256();
    k.update(&buf[..n]);
    let mut dig = [0u8; 32];
    k.finalize(&mut dig);
    dig
}

fn merkle(depth: usize) -> [u8; 32] {
    let mut cur = hash_n(32);
    let sib = [0x11u8; 32];
    let mut d = 0;
    while d < depth {
        let mut k = Keccak::v256();
        k.update(&cur);
        k.update(&sib);
        k.finalize(&mut cur);
        d += 1;
    }
    cur
}

include!("../boot.rs");
