//! Base-256 byte-array big-number core (the Amount path when `num-bigint` is
//! disabled, used by codec-only builds such as SDK/wasm).
//!
//! `Amount`'s mantissa is a big-endian byte array with no leading zeros and at
//! most 127 bytes; its decimal representation is the base-256 value. This
//! module implements decimal parse/format, `10^k` scaling and comparison on
//! byte arrays, without depending on `num-bigint`.
//!
//! This module is **compiled unconditionally**: builds with `num-bigint`
//! enabled do not reference it (the linker discards it), but under
//! `#[cfg(test)]` it is cross-checked against the `num-bigint` implementation
//! with random vectors to guarantee the two paths agree byte for byte.
#![allow(dead_code)] // referenced only by tests when num-bigint is on; by callers when off

use std::cmp::Ordering;

use sys::{Ret, errf};

/// Decimal string -> canonical mantissa bytes (no leading zeros; all zeros ->
/// empty). Values over 127 bytes (>306 decimal digits) error, matching the
/// `Amount` wire limit.
pub(crate) fn from_decimal_b256(digits: &str) -> Ret<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new(); // big-endian, may hold leading zeros, reversed at the end
    for ch in digits.bytes() {
        debug_assert!(ch.is_ascii_digit());
        let digit = ch - b'0';
        // bytes = bytes * 10 + digit (carry from the low end, reversed at the end)
        let mut carry = digit as u16;
        for b in bytes.iter_mut() {
            let v = (*b as u16) * 10 + carry;
            *b = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
        if bytes.len() > 127 {
            return errf!("Amount is too wide.");
        }
    }
    bytes.reverse(); // big-endian
    Ok(drop_left_zero_b256(&bytes))
}

/// Canonical mantissa bytes -> decimal string (empty bytes -> "0").
pub(crate) fn to_decimal_b256(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0".to_owned();
    }
    let mut buf = bytes.to_vec(); // big-endian
    let mut digits = Vec::with_capacity(40);
    loop {
        // Divide the whole number by 10; the remainder is the current lowest digit
        let mut rem = 0u16;
        for b in buf.iter_mut() {
            let v = (rem << 8) | (*b as u16);
            *b = (v / 10) as u8;
            rem = v % 10;
        }
        digits.push(b'0' + rem as u8);
        // Check whether done after stripping leading zeros
        let mut nonzero = false;
        for &b in &buf {
            if b != 0 {
                nonzero = true;
                break;
            }
        }
        if !nonzero {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits).expect("decimal digits are ascii")
}

/// Multiply by `10^exp` (no upper-bound check; the result is at most
/// len + exp/2 bytes, so the cost is bounded).
pub(crate) fn mul_pow10_b256(bytes: &[u8], exp: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..exp {
        mul10_in_place(&mut out);
    }
    out
}

/// Divide by `10^exp` (truncating division, same semantics as BigUint `/`;
/// the result stays canonical).
pub(crate) fn div_pow10_b256(bytes: &[u8], exp: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..exp {
        div10_in_place(&mut out);
    }
    drop_left_zero_b256(&out)
}

/// Compare big-endian byte arrays (callers must align units first; empty bytes
/// = 0).
pub(crate) fn cmp_b256(a: &[u8], b: &[u8]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.cmp(b))
}

fn mul10_in_place(bytes: &mut Vec<u8>) {
    // Big-endian: carry from the low byte (end) toward the high byte
    let mut carry = 0u16;
    for b in bytes.iter_mut().rev() {
        let v = (*b as u16) * 10 + carry;
        *b = (v & 0xff) as u8;
        carry = v >> 8;
    }
    while carry > 0 {
        bytes.insert(0, (carry & 0xff) as u8);
        carry >>= 8;
    }
}

fn div10_in_place(bytes: &mut [u8]) {
    // Big-endian: propagate the remainder from the high byte toward the low byte
    let mut rem = 0u16;
    for b in bytes.iter_mut() {
        let v = (rem << 8) | (*b as u16);
        *b = (v / 10) as u8;
        rem = v % 10;
    }
}

fn drop_left_zero_b256(v: &[u8]) -> Vec<u8> {
    let mut res = v;
    while res.len() > 1 && res[0] == 0 {
        res = &res[1..];
    }
    if res.len() == 1 && res[0] == 0 {
        Vec::new()
    } else {
        res.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;
    use num_traits::Zero;

    /// Simple LCG, avoiding a rand dependency for tests.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn byte(&mut self) -> u8 {
            (self.next() & 0xff) as u8
        }
        fn digit(&mut self) -> u8 {
            (self.next() % 10) as u8
        }
    }

    #[test]
    fn from_decimal_matches_bigint() {
        let mut lcg = Lcg(0x5eed);
        for _ in 0..2000 {
            let len = 1 + (lcg.next() % 400) as usize;
            let mut digits = String::with_capacity(len);
            for _ in 0..len {
                digits.push((b'0' + lcg.digit()) as char);
            }
            // Only assert agreement for values within 127 bytes; wider ones should error identically
            let b256 = from_decimal_b256(&digits);
            let big = BigUint::parse_bytes(digits.as_bytes(), 10).unwrap();
            let big_bytes = big.to_bytes_be();
            match (&b256, big_bytes.len() > 127) {
                (Ok(v), false) => {
                    let expected = if big.is_zero() {
                        Vec::new()
                    } else {
                        big_bytes
                    };
                    assert_eq!(v, &expected, "digits={}", digits);
                }
                (Err(_), true) => {}
                (Ok(v), true) => {
                    assert!(v.len() <= 127, "digits={}", digits);
                }
                (Err(e), false) => {
                    panic!("unexpected error {e} for digits={digits}");
                }
            }
        }
    }

    #[test]
    fn from_decimal_edge_cases() {
        assert_eq!(from_decimal_b256("0").unwrap(), Vec::<u8>::new());
        assert_eq!(from_decimal_b256("000").unwrap(), Vec::<u8>::new());
        assert_eq!(from_decimal_b256("1").unwrap(), vec![1]);
        assert_eq!(from_decimal_b256("255").unwrap(), vec![0xff]);
        assert_eq!(from_decimal_b256("256").unwrap(), vec![1, 0]);
        // 127 bytes = 2^1016 ≈ 10^305.85: a 305-digit decimal fits, 306 digits overflow
        let max127 = "9".repeat(305);
        assert!(from_decimal_b256(&max127).is_ok());
        assert!(from_decimal_b256(&format!("{max127}9")).is_err());
    }

    #[test]
    fn to_decimal_matches_bigint() {
        let mut lcg = Lcg(0xdecaf);
        for _ in 0..2000 {
            let len = (lcg.next() % 127) as usize + 1;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(lcg.byte());
            }
            // Build canonical form (strip leading zeros)
            let mut first = 0;
            while first < bytes.len() - 1 && bytes[first] == 0 {
                first += 1;
            }
            let bytes = &bytes[first..];
            let expected = if bytes.iter().all(|&b| b == 0) {
                "0".to_owned()
            } else {
                BigUint::from_bytes_be(bytes).to_string()
            };
            assert_eq!(to_decimal_b256(bytes), expected);
        }
        assert_eq!(to_decimal_b256(&[]), "0");
        assert_eq!(to_decimal_b256(&[0xff; 1]), "255");
    }

    #[test]
    fn mul_pow10_matches_bigint() {
        let mut lcg = Lcg(0xfeed);
        for _ in 0..1000 {
            let len = (lcg.next() % 16) as usize + 1;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(lcg.byte());
            }
            let exp = (lcg.next() % 256) as u8;
            let got = mul_pow10_b256(&bytes, exp);
            let expected = BigUint::from_bytes_be(&bytes) * BigUint::from(10u8).pow(exp as u32);
            assert_eq!(got, expected.to_bytes_be());
        }
    }

    #[test]
    fn div_pow10_matches_bigint() {
        let mut lcg = Lcg(0x0dd);
        for _ in 0..1000 {
            let len = (lcg.next() % 20) as usize + 1;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(lcg.byte());
            }
            let exp = (lcg.next() % 256) as u8;
            let got = div_pow10_b256(&bytes, exp);
            let quotient = BigUint::from_bytes_be(&bytes) / BigUint::from(10u8).pow(exp as u32);
            let expected = if quotient.is_zero() {
                Vec::new() // matches Amount's zero representation (empty bytes)
            } else {
                quotient.to_bytes_be()
            };
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn cmp_matches_bigint() {
        let mut lcg = Lcg(0xbeef);
        for _ in 0..1000 {
            let a_len = (lcg.next() % 20) as usize;
            let b_len = (lcg.next() % 20) as usize;
            let mut a = Vec::new();
            let mut b = Vec::new();
            for _ in 0..a_len {
                a.push(lcg.byte());
            }
            for _ in 0..b_len {
                b.push(lcg.byte());
            }
            let got = cmp_b256(&a, &b);
            let expected =
                BigUint::from_bytes_be(&a).cmp(&BigUint::from_bytes_be(&b));
            assert_eq!(got, expected);
        }
    }
}
