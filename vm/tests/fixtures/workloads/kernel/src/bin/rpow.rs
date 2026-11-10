#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let base = one_18();
    // 5% APR, compounded per second: x = base * (1 + 0.05/31_536_000).
    // 25 squarings, and the result stays inside 256 bits.
    let x = wrapping_add(&base, &muldiv(&base, &from_u64(5), &from_u64(31_536_000)));
    let z = rpow(x, 31_536_000, &base);
    write32(out, &z)
}

fn rpow(mut x: U, mut n: u64, base: &U) -> U {
    let mut z = if n % 2 == 0 { *base } else { x };
    n /= 2;
    while n > 0 {
        x = muldiv(&x, &x, base);
        if n % 2 == 1 {
            z = muldiv(&z, &x, base);
        }
        n /= 2;
    }
    z
}

include!("../boot.rs");
