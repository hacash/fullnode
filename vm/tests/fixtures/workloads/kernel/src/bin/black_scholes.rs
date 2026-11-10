#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let one = one_18();
    let sig = div(&one, &from_u64(5));
    let sqrt_t = one;
    let d1 = div(&sig, &from_u64(2));
    let d2 = wrapping_sub(&U::ZERO, &d1);
    let nd1 = norm_cdf(&d1);
    let nd2 = norm_cdf(&d2);
    let call = wrapping_sub(&muldiv(&one, &nd1, &one), &muldiv(&one, &nd2, &sqrt_t));
    let _ = (sig, d2, call);
    write32(out, &nd1)
}

fn norm_cdf(x: &U) -> U {
    let one = one_18();
    let half = div(&one, &from_u64(2));
    let a1 = from_u128(254_829_592);
    let a2 = from_u128(284_496_736);
    let t = div(&one, &wrapping_add(&one, &muldiv(&from_u64(327_591), x, &one)));
    let poly = muldiv(&a1, &t, &one);
    let poly = wrapping_sub(&poly, &muldiv(&a2, &muldiv(&t, &t, &one), &one));
    wrapping_add(&half, &div(&poly, &from_u64(1_000_000_000)))
}

include!("../boot.rs");
