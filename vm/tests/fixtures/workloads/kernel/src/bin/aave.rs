#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let ray = from_u128(10u128.pow(27));
    let index = ray;
    let rate = wrapping_add(&ray, &div(&ray, &from_u64(100)));
    let delta = from_u64(86_400);
    let linear = wrapping_add(&ray, &muldiv(&rate, &delta, &ray));
    let next = muldiv(&index, &linear, &ray);
    write32(out, &next)
}

include!("../boot.rs");
