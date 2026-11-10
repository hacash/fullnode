#![no_std]
#![no_main]

const P: u128 = (1u128 << 64) - (1u128 << 32) + 1;

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let mut st = [1u64, 2, 3];
    permute(&mut st);
    out[..8].copy_from_slice(&st[0].to_be_bytes());
    out[8..16].copy_from_slice(&st[1].to_be_bytes());
    out[16..24].copy_from_slice(&st[2].to_be_bytes());
    24
}

fn fe(x: u128) -> u64 {
    (x % P) as u64
}

fn mul(a: u64, b: u64) -> u64 {
    fe(a as u128 * b as u128)
}

fn add(a: u64, b: u64) -> u64 {
    fe(a as u128 + b as u128)
}

fn sbox(x: u64) -> u64 {
    let x2 = mul(x, x);
    let x4 = mul(x2, x2);
    mul(x4, x)
}

fn permute(st: &mut [u64; 3]) {
    const RF: usize = 8;
    const RP: usize = 56;
    let mut r = 0;
    while r < RF / 2 {
        full(st, r as u64);
        r += 1;
    }
    let mut p = 0;
    while p < RP {
        partial(st, p as u64);
        p += 1;
    }
    r = 0;
    while r < RF / 2 {
        full(st, 100 + r as u64);
        r += 1;
    }
}

fn full(st: &mut [u64; 3], round: u64) {
    let mut i = 0;
    while i < 3 {
        st[i] = add(st[i], rc(round, i as u64));
        st[i] = sbox(st[i]);
        i += 1;
    }
    mds(st);
}

fn partial(st: &mut [u64; 3], round: u64) {
    st[0] = add(st[0], rc(round, 0));
    st[0] = sbox(st[0]);
    mds(st);
}

fn mds(st: &mut [u64; 3]) {
    let m = [2u64, 1, 1, 1, 2, 1, 1, 1, 2];
    let s0 = st[0];
    let s1 = st[1];
    let s2 = st[2];
    let s = [s0, s1, s2];
    let mut i = 0;
    while i < 3 {
        let mut acc = 0u64;
        let mut j = 0;
        while j < 3 {
            acc = add(acc, mul(m[i * 3 + j], s[j]));
            j += 1;
        }
        st[i] = acc;
        i += 1;
    }
}

fn rc(round: u64, i: u64) -> u64 {
    fe((round.wrapping_mul(3) + i).wrapping_mul(0x9e37_79b9_7f4a_7c15) as u128)
}

include!("../boot.rs");
