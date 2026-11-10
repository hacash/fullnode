#![no_std]
#![no_main]

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let mut v = [0u64; 16];
    let mut i = 0;
    while i < 16 {
        v[i] = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(7);
        i += 1;
    }
    i = 1;
    while i < 16 {
        let key = v[i];
        let mut j = i;
        while j > 0 && v[j - 1] > key {
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = key;
        i += 1;
    }
    let med = v[8].to_be_bytes();
    out[..8].copy_from_slice(&med);
    8
}

include!("../boot.rs");
