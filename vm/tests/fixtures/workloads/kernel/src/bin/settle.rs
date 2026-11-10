#![no_std]
#![no_main]

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    const N: usize = 100;
    let mut weight = [0u64; N];
    let mut i = 0;
    let mut sum = 0u128;
    while i < N {
        weight[i] = (i as u64) + 1;
        sum += weight[i] as u128;
        i += 1;
    }
    let pot = 1_000_000_000_000_000_000u128;
    i = 0;
    while i < N {
    let pay = (pot * (i as u128 + 1) / sum) as u64;
        out[i * 8..i * 8 + 8].copy_from_slice(&pay.to_be_bytes());
        i += 1;
    }
    N * 8
}

include!("../boot.rs");
