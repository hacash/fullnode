#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let amount_in = one_18();
    let reserve_in = from_u128(1_000_000_000_000_000_000_000_000);
    let reserve_out = reserve_in;
    let fee_in = muldiv(&amount_in, &from_u64(997), &from_u64(1000));
    let num = mul(&fee_in, &reserve_out);
    let den = wrapping_add(&reserve_in, &fee_in);
    let amount_out = div(&num, &den);
    write32(out, &amount_out)
}

include!("../boot.rs");
