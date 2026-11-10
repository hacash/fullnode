#![no_std]
#![no_main]
#[path = "../uintw.rs"]
mod uintw;
use uintw::*;

fn kernel(input: &[u8], out: &mut [u8]) -> usize {
    let mode = input.first().copied().unwrap_or(0);
    let e24 = from_u128(10u128.pow(24));
    let v = match mode {
        1 => {
            let xp = [e24, e24, e24];
            get_d(&xp, &from_u64(300))
        }
        2 => {
            let xp = [e24, e24];
            let d = get_d(&xp, &from_u64(200));
            let x = muldiv(&e24, &from_u64(1001), &from_u64(1000));
            get_y(0, 1, &x, &xp, &from_u64(200), &d)
        }
        _ => {
            let xp = [e24, e24];
            get_d(&xp, &from_u64(200))
        }
    };
    write32(out, &v)
}

fn get_d(xp: &[U], ann: &U) -> U {
    let n = from_u64(xp.len() as u64);
    let mut s = U::ZERO;
    let mut i = 0;
    while i < xp.len() {
        s = wrapping_add(&s, &xp[i]);
        i += 1;
    }
    if eq(&s, &U::ZERO) {
        return s;
    }
    let mut d = s;
    let mut iter = 0;
    while iter < 255 {
        let mut d_p = d;
        i = 0;
        while i < xp.len() {
            let den = mul(&xp[i], &n);
            d_p = muldiv(&d_p, &d, &den);
            i += 1;
        }
        let dprev = d;
        let term_s = mul(ann, &s);
        let term_p = mul(&d_p, &n);
        let num = mul(&wrapping_add(&term_s, &term_p), &d);
        let ann_m1 = wrapping_sub(ann, &U::ONE);
        let n_p1 = wrapping_add(&n, &U::ONE);
        let den = wrapping_add(&mul(&ann_m1, &d), &mul(&n_p1, &d_p));
        d = div(&num, &den);
        if abs_le_one(&d, &dprev) {
            return d;
        }
        iter += 1;
    }
    d
}

fn get_y(i_coin: usize, j_coin: usize, x: &U, xp: &[U], ann: &U, d: &U) -> U {
    let n = from_u64(xp.len() as u64);
    let mut c = *d;
    let mut s = U::ZERO;
    let mut k = 0;
    while k < xp.len() {
        if k != j_coin {
            let xx = if k == i_coin { *x } else { xp[k] };
            s = wrapping_add(&s, &xx);
            let den = mul(&xx, &n);
            c = muldiv(&c, d, &den);
        }
        k += 1;
    }
    c = muldiv(&c, d, &mul(ann, &n));
    let b = wrapping_add(&s, &div(d, ann));
    let mut y = *d;
    let mut iter = 0;
    while iter < 255 {
        let y_prev = y;
        let num = wrapping_add(&mul(&y, &y), &c);
        let den = wrapping_sub(&wrapping_add(&wrapping_add(&y, &y), &b), d);
        y = div(&num, &den);
        if abs_le_one(&y, &y_prev) {
            return y;
        }
        iter += 1;
    }
    y
}

include!("../boot.rs");
