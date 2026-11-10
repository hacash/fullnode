#![no_std]
#![no_main]
use sha2::{Digest, Sha256};

fn kernel(input: &[u8], out: &mut [u8]) -> usize {
    let mode = input.first().copied().unwrap_or(0);
    let dig = match mode {
        1 => merkle(16),
        2 => merkle(32),
        3 => hash_n(1024),
        _ => hash_n(32),
    };
    out[..32].copy_from_slice(&dig);
    32
}

fn hash_n(n: usize) -> [u8; 32] {
    let mut h = Sha256::new();
    let mut block = [0u8; 64];
    let mut filled = 0usize;
    let mut i = 0;
    while i < n {
        block[filled] = i as u8;
        filled += 1;
        if filled == 64 {
            h.update(&block);
            filled = 0;
        }
        i += 1;
    }
    if filled != 0 {
        h.update(&block[..filled]);
    }
    let dig = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(dig.as_slice());
    out
}

fn merkle(depth: usize) -> [u8; 32] {
    let mut cur: [u8; 32] = Sha256::digest(b"leaf").into();
    let sib = [0x11u8; 32];
    let mut d = 0;
    while d < depth {
        let mut h = Sha256::new();
        h.update(cur);
        h.update(sib);
        cur = {
            let dig = h.finalize();
            let mut o = [0u8; 32];
            o.copy_from_slice(dig.as_slice());
            o
        };
        d += 1;
    }
    cur
}

include!("../boot.rs");
