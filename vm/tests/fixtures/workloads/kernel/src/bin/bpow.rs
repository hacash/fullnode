#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let one = one_18();
    let base = wrapping_add(&one, &div(&one, &from_u64(2)));
    let exp = div(&one, &from_u64(2));
    let ln_base = ln(&base);
    let y = exp_1e18(&muldiv(&exp, &ln_base, &one));
    write32(out, &y)
}

fn exp_1e18(x: &U) -> U {
    let one = one_18();
    let ln2 = from_u128(693_147_180_559_945_309);
    let k = div(x, &ln2);
    let r = wrapping_sub(x, &mul(&k, &ln2));
    let mut term = one;
    let mut sum = one;
    let mut i = 1u64;
    while i <= 16 {
        term = muldiv(&term, &r, &mul(&from_u64(i), &one));
        sum = wrapping_add(&sum, &term);
        i += 1;
    }
    let mut s = 0u32;
    let kb = k.to_be_bytes();
    let shift = u32::from_be_bytes([kb[28], kb[29], kb[30], kb[31]]);
    while s < shift && s < 64 {
        sum = wrapping_add(&sum, &sum);
        s += 1;
    }
    sum
}

fn ln(x: &U) -> U {
    let one = one_18();
    let mut z = *x;
    let mut k = 0u32;
    let two = wrapping_add(&one, &one);
    while le(&two, &z) && k < 64 {
        z = div(&z, &from_u64(2));
        k += 1;
    }
    let y = wrapping_sub(&z, &one);
    let mut term = y;
    let mut sum = U::ZERO;
    let mut i = 1u64;
    let mut sign_add = true;
    while i <= 16 {
        if sign_add {
            sum = wrapping_add(&sum, &div(&term, &from_u64(i)));
        } else {
            sum = wrapping_sub(&sum, &div(&term, &from_u64(i)));
        }
        term = muldiv(&term, &y, &one);
        sign_add = !sign_add;
        i += 1;
    }
    let ln2 = from_u128(693_147_180_559_945_309);
    wrapping_add(&sum, &mul(&from_u64(k as u64), &ln2))
}

include!("../boot.rs");
