#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(input: &[u8], out: &mut [u8]) -> usize {
    let mode = input.first().copied().unwrap_or(0);
    let v = match mode {
        1 => tick_sqrt_worst(),
        2 => tick_log_refine(),
        _ => swap_steps(20),
    };
    write32(out, &v)
}

fn tick_sqrt_worst() -> U {
    let mut ratio = U::ONE;
    let mut i = 0u64;
    while i < 20 {
        let c = from_u64(0xfffc_b933_bd6f_ad37 ^ (i.wrapping_mul(0x9e37_79b9)));
        ratio = mul(&ratio, &c);
        ratio = div(&ratio, &from_u64(1 << 16));
        i += 1;
    }
    ratio
}

fn tick_log_refine() -> U {
    let mut r = from_u128(1 << 96);
    let mut i = 0;
    while i < 14 {
        r = mul_hi(&r, &r);
        if eq(&r, &U::ZERO) {
            r = U::ONE;
        }
        i += 1;
    }
    r
}

fn swap_steps(n: u32) -> U {
    let mut amount = one_18();
    let reserve_in = from_u128(10u128.pow(24));
    let reserve_out = reserve_in;
    let mut i = 0;
    while i < n {
        amount = muldiv(&amount, &from_u64(997), &from_u64(1000));
        let den = wrapping_add(&reserve_in, &amount);
        amount = muldiv(&amount, &reserve_out, &den);
        i += 1;
    }
    amount
}

include!("../boot.rs");
