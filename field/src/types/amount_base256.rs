//! Base-256 byte-array big-number core — the single production implementation of
//! `Amount`'s big-integer path; `num-bigint` survives only as a test oracle.

use std::cmp::Ordering;

use sys::{Ret, errf};

/// Decimal string -> canonical mantissa bytes (no leading zeros; all zeros -> empty).
/// Values over 127 bytes (>306 digits) error, matching the `Amount` wire limit.
pub(crate) fn from_decimal_b256(digits: &str) -> Ret<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new(); // little-endian while accumulating
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

/// Canonical mantissa bytes -> decimal string (empty / all-zero bytes -> "0").
pub(crate) fn to_decimal_b256(bytes: &[u8]) -> String {
    let bytes = skip_left_zero(bytes);
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

/// Multiply by `10^exp` (no upper-bound check; cost bounded). Leading zeros are
/// insignificant, matching `BigUint`; the result is Amount-canonical.
pub(crate) fn mul_pow10_b256(bytes: &[u8], exp: u8) -> Vec<u8> {
    let mut out = skip_left_zero(bytes).to_vec();
    for _ in 0..exp {
        mul10_in_place(&mut out);
    }
    drop_left_zero_b256(&out)
}

/// Divide by `10^exp` (truncating division, same semantics as BigUint `/`;
/// the result stays canonical). Leading zeros in `bytes` are insignificant.
pub(crate) fn div_pow10_b256(bytes: &[u8], exp: u8) -> Vec<u8> {
    let mut out = skip_left_zero(bytes).to_vec();
    for _ in 0..exp {
        div10_in_place(&mut out);
    }
    drop_left_zero_b256(&out)
}

/// Compare big-endian magnitudes like `BigUint::from_bytes_be` (leading zeros
/// insignificant, empty/all-zero = 0). Callers align units first.
pub(crate) fn cmp_b256(a: &[u8], b: &[u8]) -> Ordering {
    let a = skip_left_zero(a);
    let b = skip_left_zero(b);
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Multiply a canonical mantissa by a small u64 constant (exact), byte-identical to
/// BigUint's `* m`. Used by the channel-interest execute body (mint-core).
pub fn mul_u64_b256(bytes: &[u8], m: u64) -> Vec<u8> {
    let bytes = skip_left_zero(bytes);
    if bytes.is_empty() || m == 0 {
        return Vec::new();
    }
    let mut out = bytes.to_vec();
    let mut carry = 0u128;
    for b in out.iter_mut().rev() {
        let v = (*b as u128) * (m as u128) + carry;
        *b = (v & 0xff) as u8;
        carry = v >> 8;
    }
    while carry > 0 {
        out.insert(0, (carry & 0xff) as u8);
        carry >>= 8;
    }
    drop_left_zero_b256(&out)
}

/// Divide a canonical mantissa by a small nonzero u64 constant (truncating, like
/// `BigUint /`), returning the canonical quotient and remainder (mint-core).
pub fn divmod_u64_b256(bytes: &[u8], d: u64) -> (Vec<u8>, u64) {
    debug_assert!(d != 0, "division by zero");
    let mut out = skip_left_zero(bytes).to_vec();
    let mut rem = 0u128;
    for b in out.iter_mut() {
        let v = (rem << 8) | (*b as u128);
        *b = (v / (d as u128)) as u8;
        rem = v % (d as u128);
    }
    (drop_left_zero_b256(&out), rem as u64)
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

/// Strip leading zeros; all-zero (including empty) becomes empty.
fn skip_left_zero(v: &[u8]) -> &[u8] {
    match v.iter().position(|&b| b != 0) {
        Some(i) => &v[i..],
        None => &[],
    }
}

fn drop_left_zero_b256(v: &[u8]) -> Vec<u8> {
    skip_left_zero(v).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;
    use num_traits::{ToPrimitive, Zero};
    use std::cmp::Ordering;

    /// Simple LCG, avoiding a rand dependency for tests.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn byte(&mut self) -> u8 {
            (self.next() & 0xff) as u8
        }
        fn digit(&mut self) -> u8 {
            (self.next() % 10) as u8
        }
        fn pick(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
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
                    let expected = if big.is_zero() { Vec::new() } else { big_bytes };
                    assert_eq!(v, &expected, "digits={}", digits);
                }
                (Err(_), true) => {}
                (Ok(v), true) => {
                    panic!(
                        "b256 accepted over-wide value ({} bytes, digits={})",
                        v.len(),
                        digits
                    );
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
        // Cap is on byte length, not digit count: 127 bytes = 2^1016 ≈ 10^305.85,
        // so "9"×305 fits, "9"×306 overflows, while 10^305 (306 digits) still fits.
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
            let expected = BigUint::from_bytes_be(&bytes).to_string();
            assert_eq!(to_decimal_b256(&bytes), expected);
        }
        assert_eq!(to_decimal_b256(&[]), "0");
        assert_eq!(to_decimal_b256(&[0]), "0");
        assert_eq!(to_decimal_b256(&[0, 0, 0]), "0");
        assert_eq!(to_decimal_b256(&[0, 0, 1]), "1");
        assert_eq!(to_decimal_b256(&[0xff; 1]), "255");
    }

    fn expect_amt_bytes(v: &BigUint) -> Vec<u8> {
        if v.is_zero() {
            Vec::new()
        } else {
            v.to_bytes_be()
        }
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
            assert_eq!(got, expect_amt_bytes(&expected));
            assert!(got.is_empty() || got[0] != 0);
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
            let expected = BigUint::from_bytes_be(&a).cmp(&BigUint::from_bytes_be(&b));
            assert_eq!(got, expected);
        }
    }

    // --- Extreme-value / boundary equivalence (num-bigint as test oracle) ---
    // Cross-checks the base-256 core against num-bigint at the 127-byte cap, the u128 boundary, and random wide mantissas.

    /// Canonical form: strip leading zeros; all zeros collapse to empty.
    fn canon(mut v: Vec<u8>) -> Vec<u8> {
        let mut first = 0;
        while first + 1 < v.len() && v[first] == 0 {
            first += 1;
        }
        v = v[first..].to_vec();
        if v.iter().all(|&b| b == 0) {
            v.clear();
        }
        v
    }

    #[test]
    fn from_decimal_exhaustive_small() {
        for v in 0u64..=999999 {
            let digits = v.to_string();
            let got = from_decimal_b256(&digits).unwrap();
            let big = BigUint::parse_bytes(digits.as_bytes(), 10).unwrap();
            let expected = if big.is_zero() {
                Vec::new()
            } else {
                big.to_bytes_be()
            };
            assert_eq!(got, expected, "from_decimal {v}");
            if v <= 65535 {
                assert_eq!(to_decimal_b256(&got), digits, "roundtrip {v}");
            }
        }
    }

    #[test]
    fn power_of_256_boundaries() {
        // 256^k - 1 and 256^k for k = 1..=128, straddling the 127-byte cap
        for k in 1u32..=128u32 {
            let minus1 = BigUint::from(256u32).pow(k) - BigUint::from(1u8);
            let exact = BigUint::from(256u32).pow(k);
            for (name, v) in [("256^k-1", minus1), ("256^k", exact)] {
                let bytes = v.to_bytes_be();
                let expected = if bytes.len() > 127 { None } else { Some(bytes) };
                let got = from_decimal_b256(&v.to_string()).ok();
                assert_eq!(got, expected, "{name} k={k}");
                if let Some(b) = &expected {
                    assert_eq!(to_decimal_b256(b), v.to_string(), "to_decimal {name} k={k}");
                }
            }
        }
    }

    #[test]
    fn decimal_cap_boundaries() {
        // 9…9 and 10^k for k = 1..=310, straddling the 127-byte cap (≈10^305.85).
        for k in 1..=310usize {
            let nines = "9".repeat(k);
            let pow10 = format!("1{}", "0".repeat(k));
            for (name, s) in [("9s", nines), ("10^k", pow10)] {
                let v = BigUint::parse_bytes(s.as_bytes(), 10).unwrap();
                let bytes = v.to_bytes_be();
                let expected = if bytes.len() > 127 { None } else { Some(bytes) };
                assert_eq!(from_decimal_b256(&s).ok(), expected, "{name} k={k}");
            }
        }
        // exact 127-byte extremes
        let max127 = vec![0xffu8; 127];
        let max127s = BigUint::from_bytes_be(&max127).to_string();
        assert_eq!(to_decimal_b256(&max127), max127s);
        assert_eq!(from_decimal_b256(&max127s).unwrap(), max127);
        let min128 = BigUint::from(256u32).pow(127);
        assert!(from_decimal_b256(&min128.to_string()).is_err());
    }

    #[test]
    fn wide_roundtrip_random() {
        let mut lcg = Lcg(0xabcd);
        for _ in 0..4000 {
            let len = 1 + lcg.pick(127);
            let mut v = vec![0u8; len];
            for b in v.iter_mut() {
                *b = lcg.byte();
            }
            let v = canon(v);
            let bv = BigUint::from_bytes_be(&v);
            assert_eq!(to_decimal_b256(&v), bv.to_string(), "to_decimal len={len}");
            let s = bv.to_string();
            assert_eq!(from_decimal_b256(&s).unwrap(), v, "from_decimal len={len}");
        }
    }

    #[test]
    fn parse_cap_matches_bigint() {
        // 420-char digit strings: base-256 must accept exactly what BigUint accepts
        // (error iff the canonical byte length exceeds 127)
        let mut lcg = Lcg(0xbeef);
        for _ in 0..3000 {
            let len = 1 + lcg.pick(420);
            let mut s = String::with_capacity(len);
            for _ in 0..len {
                s.push((b'0' + lcg.pick(10) as u8) as char);
            }
            let t = s.trim_start_matches('0');
            let v = if t.is_empty() {
                BigUint::zero()
            } else {
                BigUint::parse_bytes(t.as_bytes(), 10).unwrap()
            };
            let bytes = v.to_bytes_be();
            let expected = if bytes.len() > 127 {
                None
            } else if v.is_zero() {
                Some(Vec::new())
            } else {
                Some(bytes)
            };
            assert_eq!(from_decimal_b256(&s).ok(), expected, "digits len={len}");
        }
    }

    #[test]
    fn scaling_matches_bigint_extremes() {
        let mut lcg = Lcg(0xf00d);
        for _ in 0..3000 {
            let len = 1 + lcg.pick(127);
            let mut v = vec![0u8; len];
            for b in v.iter_mut() {
                *b = lcg.byte();
            }
            let v = canon(v);
            let exp = lcg.pick(256) as u8;
            let expected = BigUint::from_bytes_be(&v) * BigUint::from(10u8).pow(exp as u32);
            let got = mul_pow10_b256(&v, exp);
            let expected = if expected.is_zero() {
                Vec::new()
            } else {
                expected.to_bytes_be()
            };
            assert_eq!(got, expected, "mul len={len} exp={exp}");
            // the result must stay canonical (no leading zero byte)
            assert!(
                got.is_empty() || got[0] != 0,
                "mul canonical len={len} exp={exp}"
            );
            let q = BigUint::from_bytes_be(&v) / BigUint::from(10u8).pow(exp as u32);
            let expected = if q.is_zero() {
                Vec::new()
            } else {
                q.to_bytes_be()
            };
            assert_eq!(div_pow10_b256(&v, exp), expected, "div len={len} exp={exp}");
        }
        assert_eq!(mul_pow10_b256(&[], 255), Vec::<u8>::new());
        assert_eq!(div_pow10_b256(&[], 255), Vec::<u8>::new());
    }

    #[test]
    fn cmp_matches_bigint_extremes() {
        let mut lcg = Lcg(0xc0de);
        for _ in 0..2000 {
            let la = 1 + lcg.pick(127);
            let lb = 1 + lcg.pick(127);
            let mut a = vec![0u8; la];
            let mut b = vec![0u8; lb];
            for x in a.iter_mut() {
                *x = lcg.byte();
            }
            for x in b.iter_mut() {
                *x = lcg.byte();
            }
            let a = canon(a);
            let b = canon(b);
            assert_eq!(
                cmp_b256(&a, &b),
                BigUint::from_bytes_be(&a).cmp(&BigUint::from_bytes_be(&b)),
                "la={la} lb={lb}"
            );
        }
        let max127 = vec![0xffu8; 127];
        assert_eq!(cmp_b256(&max127, &[0x01]), Ordering::Greater);
        assert_eq!(cmp_b256(&[], &[0x01]), Ordering::Less);
        assert_eq!(cmp_b256(&[], &[]), Ordering::Equal);
    }

    #[test]
    fn mul_div_u64_matches_bigint() {
        let mut lcg = Lcg(0x5150);
        for _ in 0..2000 {
            let len = 1 + lcg.pick(127);
            let mut v = vec![0u8; len];
            for b in v.iter_mut() {
                *b = lcg.byte();
            }
            let v = canon(v);
            let big = BigUint::from_bytes_be(&v);
            let m = 1 + (lcg.next() % 1_0000_0000_0000u64);
            let got_mul = mul_u64_b256(&v, m);
            // BigUint's to_bytes_be renders zero as [0]; the b256 core uses
            // the Amount convention (empty bytes)
            let expected_mul = if big.is_zero() {
                Vec::new()
            } else {
                (big.clone() * m).to_bytes_be()
            };
            assert_eq!(got_mul, expected_mul, "mul len={len} m={m}");
            // result must stay canonical (no leading zero byte)
            assert!(
                got_mul.is_empty() || got_mul[0] != 0,
                "mul canonical len={len} m={m}"
            );
            let d = 1 + (lcg.next() % 10_000u64);
            let (q, r) = divmod_u64_b256(&v, d);
            let bd = BigUint::from(d);
            let q_big = &big / &bd;
            let expected_q = if q_big.is_zero() {
                Vec::new()
            } else {
                q_big.to_bytes_be()
            };
            let expected_r = (&big % &bd).to_u64().unwrap();
            assert_eq!(q, expected_q, "div len={len} d={d}");
            assert_eq!(r, expected_r, "mod len={len} d={d}");
        }
        // edge cases
        assert_eq!(mul_u64_b256(&[], 5), Vec::<u8>::new());
        assert_eq!(mul_u64_b256(&[0], 5), Vec::<u8>::new());
        assert_eq!(mul_u64_b256(&[0, 0, 7], 0), Vec::<u8>::new());
        assert_eq!(mul_u64_b256(&[7], 0), Vec::<u8>::new());
        assert_eq!(divmod_u64_b256(&[], 10), (Vec::<u8>::new(), 0));
        assert_eq!(divmod_u64_b256(&[0, 0], 10), (Vec::<u8>::new(), 0));
        assert_eq!(divmod_u64_b256(&[255], 1), (vec![255], 0));
        assert_eq!(divmod_u64_b256(&[255], 10), (vec![25], 5));
    }

    /// Frozen BigUint-equivalent vectors, including the non-canonical inputs
    /// that used to make length-first `cmp_b256` diverge.
    #[test]
    fn frozen_bigint_equivalence_vectors() {
        assert_eq!(cmp_b256(&[], &[]), Ordering::Equal);
        assert_eq!(cmp_b256(&[], &[0]), Ordering::Equal);
        assert_eq!(cmp_b256(&[0, 0], &[0]), Ordering::Equal);
        assert_eq!(cmp_b256(&[0, 1], &[1]), Ordering::Equal);
        assert_eq!(cmp_b256(&[0, 0, 1], &[1]), Ordering::Equal);
        assert_eq!(cmp_b256(&[0, 2], &[1]), Ordering::Greater);
        assert_eq!(cmp_b256(&[1], &[0, 2]), Ordering::Less);
        assert_eq!(cmp_b256(&[0, 0, 1, 0], &[1, 0]), Ordering::Equal);
        assert_eq!(cmp_b256(&[0xff; 127], &[0x01]), Ordering::Greater);

        assert_eq!(mul_pow10_b256(&[0, 1], 0), vec![1]);
        assert_eq!(mul_pow10_b256(&[0, 1], 1), vec![10]);
        assert_eq!(mul_pow10_b256(&[0, 1], 2), vec![100]);
        assert_eq!(mul_pow10_b256(&[0, 1], 3), vec![3, 232]); // 1000
        assert_eq!(mul_pow10_b256(&[0, 0, 0], 255), Vec::<u8>::new());
        assert_eq!(div_pow10_b256(&[0, 100], 1), vec![10]);
        assert_eq!(div_pow10_b256(&[0, 1], 1), Vec::<u8>::new());

        let padded = [0, 0, 7];
        assert_eq!(mul_u64_b256(&padded, 1), vec![7]);
        assert_eq!(mul_u64_b256(&padded, 256), vec![7, 0]);
        let (q, r) = divmod_u64_b256(&padded, 2);
        assert_eq!(q, vec![3]);
        assert_eq!(r, 1);

        // u64::MAX carry path vs BigUint
        let wide = vec![0xffu8; 16];
        let big = BigUint::from_bytes_be(&wide);
        assert_eq!(
            mul_u64_b256(&wide, u64::MAX),
            expect_amt_bytes(&(big.clone() * u64::MAX))
        );
        let padded_wide: Vec<u8> = std::iter::repeat_n(0, 4)
            .chain(wide.iter().copied())
            .collect();
        assert_eq!(
            mul_u64_b256(&padded_wide, u64::MAX),
            expect_amt_bytes(&(big.clone() * u64::MAX))
        );
        let (q, r) = divmod_u64_b256(&padded_wide, u64::MAX);
        assert_eq!(q, expect_amt_bytes(&(&big / u64::MAX)));
        assert_eq!(r, (&big % u64::MAX).to_u64().unwrap());
        let (q, r) = divmod_u64_b256(&[0, 1, 0], 256);
        assert_eq!(q, vec![1]);
        assert_eq!(r, 0);
    }

    #[test]
    fn non_canonical_random_matches_bigint() {
        let mut lcg = Lcg(0xa11c);
        for _ in 0..3000 {
            let pad = lcg.pick(8);
            let len = lcg.pick(40);
            let mut v = vec![0u8; pad];
            for _ in 0..len {
                v.push(lcg.byte());
            }
            let big = BigUint::from_bytes_be(&v);
            assert_eq!(cmp_b256(&v, &expect_amt_bytes(&big)), Ordering::Equal);
            assert_eq!(to_decimal_b256(&v), big.to_string());
            let exp = lcg.pick(64) as u8;
            let scaled = &big * BigUint::from(10u8).pow(exp as u32);
            assert_eq!(mul_pow10_b256(&v, exp), expect_amt_bytes(&scaled));
            let q = &big / BigUint::from(10u8).pow(exp as u32);
            assert_eq!(div_pow10_b256(&v, exp), expect_amt_bytes(&q));
            let m = match lcg.pick(6) {
                0 => 0,
                1 => 1,
                2 => 256,
                3 => u64::MAX,
                _ => 1 + lcg.next() % 1_000_000,
            };
            assert_eq!(mul_u64_b256(&v, m), expect_amt_bytes(&(&big * m)));
            let d = match lcg.pick(5) {
                0 => 1,
                1 => 10,
                2 => 256,
                3 => 10_000,
                _ => 1 + lcg.next() % 1_000_000,
            };
            let (got_q, got_r) = divmod_u64_b256(&v, d);
            assert_eq!(got_q, expect_amt_bytes(&(&big / d)));
            assert_eq!(got_r, (&big % d).to_u64().unwrap());
        }
    }

    #[test]
    fn u128_boundary_matches_bigint() {
        let cases: &[u128] = &[
            0,
            1,
            9,
            10,
            255,
            256,
            u64::MAX as u128,
            u64::MAX as u128 + 1,
            (1u128 << 120) - 1,
            1u128 << 120,
            u128::MAX - 1,
            u128::MAX,
        ];
        for &v in cases {
            let digits = v.to_string();
            let got = from_decimal_b256(&digits).unwrap();
            let expected = if v == 0 {
                Vec::new()
            } else {
                drop_left_zero_b256(&v.to_be_bytes())
            };
            assert_eq!(got, expected, "from_decimal {v}");
            assert_eq!(to_decimal_b256(&got), digits, "to_decimal {v}");
            assert_eq!(to_decimal_b256(&v.to_be_bytes()), digits);
        }
        let over = BigUint::from(1u8) << 128u32;
        let over_bytes = from_decimal_b256(&over.to_string()).unwrap();
        assert_eq!(over_bytes.len(), 17);
        assert_eq!(over_bytes, over.to_bytes_be());
        assert_eq!(
            cmp_b256(&u128::MAX.to_be_bytes(), &over_bytes),
            Ordering::Less
        );
    }
}
