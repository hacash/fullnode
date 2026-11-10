//! Variable-time 256-bit arithmetic for public financial inputs.
//! Limbs are little-endian. Division is restoring binary division of the
//! 512-bit product; it is correct and deliberately simple.
#![allow(dead_code)]

#[derive(Clone, Copy)]
pub struct U(pub [u64; 4]);

impl U {
    pub const ZERO: Self = Self([0, 0, 0, 0]);
    pub const ONE: Self = Self([1, 0, 0, 0]);

    pub fn to_be_bytes(self) -> [u8; 32] {
        let mut o = [0u8; 32];
        let mut i = 0;
        while i < 4 {
            let b = self.0[3 - i].to_be_bytes();
            o[i * 8..i * 8 + 8].copy_from_slice(&b);
            i += 1;
        }
        o
    }

    pub fn wrapping_add(self, rhs: Self) -> Self {
        let mut o = [0u64; 4];
        let mut c = 0u128;
        let mut i = 0;
        while i < 4 {
            c += self.0[i] as u128 + rhs.0[i] as u128;
            o[i] = c as u64;
            c >>= 64;
            i += 1;
        }
        Self(o)
    }

    pub fn wrapping_sub(self, rhs: Self) -> Self {
        let mut o = [0u64; 4];
        let mut br = 0i128;
        let mut i = 0;
        while i < 4 {
            let v = self.0[i] as i128 - rhs.0[i] as i128 - br;
            if v < 0 {
                o[i] = (v + (1i128 << 64)) as u64;
                br = 1;
            } else {
                o[i] = v as u64;
                br = 0;
            }
            i += 1;
        }
        Self(o)
    }
}

pub fn from_u64(n: u64) -> U {
    U([n, 0, 0, 0])
}

pub fn from_u128(n: u128) -> U {
    U([n as u64, (n >> 64) as u64, 0, 0])
}

pub fn one_18() -> U {
    from_u128(1_000_000_000_000_000_000)
}

pub fn eq(a: &U, b: &U) -> bool {
    a.0 == b.0
}

pub fn le(a: &U, b: &U) -> bool {
    let mut i = 4;
    while i > 0 {
        i -= 1;
        if a.0[i] < b.0[i] {
            return true;
        }
        if a.0[i] > b.0[i] {
            return false;
        }
    }
    true
}

pub fn wrapping_add(a: &U, b: &U) -> U {
    a.wrapping_add(*b)
}

pub fn wrapping_sub(a: &U, b: &U) -> U {
    a.wrapping_sub(*b)
}

pub fn abs_le_one(a: &U, b: &U) -> bool {
    let d = a.wrapping_sub(*b);
    let e = b.wrapping_sub(*a);
    eq(&d, &U::ZERO) || eq(&d, &U::ONE) || eq(&e, &U::ZERO) || eq(&e, &U::ONE)
}

fn ge4(a: &[u64; 4], b: &[u64; 4]) -> bool {
    let mut i = 4;
    while i > 0 {
        i -= 1;
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn shl1(a: &mut [u64; 4]) -> u64 {
    let mut c = 0u64;
    let mut i = 0;
    while i < 4 {
        let nc = a[i] >> 63;
        a[i] = (a[i] << 1) | c;
        c = nc;
        i += 1;
    }
    c
}

fn sub4(a: &mut [u64; 4], b: &[u64; 4]) {
    let mut br = 0u64;
    let mut i = 0;
    while i < 4 {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(br);
        a[i] = d2;
        br = (b1 | b2) as u64;
        i += 1;
    }
}

fn bit512(u: &[u64; 8], i: usize) -> u64 {
    (u[i / 64] >> (i % 64)) & 1
}

/// Floor of a 512-bit / 256-bit division. The quotient must fit in 256 bits.
fn div512(u: [u64; 8], v: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4];
    let mut q = [0u64; 4];
    let mut i = 512;
    while i > 0 {
        i -= 1;
        let carry = shl1(&mut r);
        r[0] |= bit512(&u, i);
        if carry != 0 || ge4(&r, &v) {
            sub4(&mut r, &v);
            if i < 256 {
                q[i / 64] |= 1u64 << (i % 64);
            }
        }
    }
    q
}

fn mul256(a: [u64; 4], b: [u64; 4]) -> [u64; 8] {
    let mut p = [0u64; 8];
    let mut i = 0;
    while i < 4 {
        let mut carry = 0u128;
        let mut j = 0;
        while j < 4 {
            let t = p[i + j] as u128 + a[i] as u128 * b[j] as u128 + carry;
            p[i + j] = t as u64;
            carry = t >> 64;
            j += 1;
        }
        p[i + 4] = carry as u64;
        i += 1;
    }
    p
}

pub fn mul(a: &U, b: &U) -> U {
    let p = mul256(a.0, b.0);
    U([p[0], p[1], p[2], p[3]])
}

pub fn muldiv(a: &U, b: &U, c: &U) -> U {
    U(div512(mul256(a.0, b.0), c.0))
}

pub fn div(a: &U, b: &U) -> U {
    muldiv(a, &U::ONE, b)
}

/// High 256 bits of a*b. Used by fixed-point squaring.
pub fn mul_hi(a: &U, b: &U) -> U {
    let p = mul256(a.0, b.0);
    U([p[4], p[5], p[6], p[7]])
}

pub fn write32(out: &mut [u8], v: &U) -> usize {
    out[..32].copy_from_slice(&v.to_be_bytes());
    32
}
