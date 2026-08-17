//! base-256 字节数组大数核心（Amount 的 SDK/wasm codec-only 路径）。
//!
//! `Amount` 的 mantissa 是"无前导零、最多 127 字节"的大端字节数组，十进制
//! 表示即 base-256 数值。本模块用字节数组实现十进制解析/格式化、`10^k`
//! 缩放和比较，不依赖 `num-bigint`。
//!
//! 本模块**无条件编译**：native 构建下它不被引用（链接器丢弃），但 `#[cfg(test)]`
//! 里与 `num-bigint` 实现做随机向量对比，保证两条路径逐字节一致。
#![allow(dead_code)] // native 构建下仅测试引用；SDK/wasm codec-only 下被上层引用

use std::cmp::Ordering;

use sys::{Ret, errf};

/// 十进制字符串 → 规范 mantissa 字节（无前导零；全零 → 空字节）。
/// 超过 127 字节（>306 位十进制）与 `Amount` wire 上限一致地报错。
pub(crate) fn from_decimal_b256(digits: &str) -> Ret<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new(); // 大端，可能含前导零，末尾先除
    for ch in digits.bytes() {
        debug_assert!(ch.is_ascii_digit());
        let digit = ch - b'0';
        // bytes = bytes * 10 + digit（从低位进位，最后反转）
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
    bytes.reverse(); // 大端
    Ok(drop_left_zero_b256(&bytes))
}

/// 规范 mantissa 字节 → 十进制字符串（空字节 → "0"）。
pub(crate) fn to_decimal_b256(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0".to_owned();
    }
    let mut buf = bytes.to_vec(); // 大端
    let mut digits = Vec::with_capacity(40);
    loop {
        // 整体除以 10，余数即当前最低位
        let mut rem = 0u16;
        for b in buf.iter_mut() {
            let v = (rem << 8) | (*b as u16);
            *b = (v / 10) as u8;
            rem = v % 10;
        }
        digits.push(b'0' + rem as u8);
        // 去掉前导零后检查是否结束
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

/// 乘以 `10^exp`（不做上限检查；结果字节数 ≤ len + exp/2，计算量可控）。
pub(crate) fn mul_pow10_b256(bytes: &[u8], exp: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..exp {
        mul10_in_place(&mut out);
    }
    out
}

/// 除以 `10^exp`（截断除法，与 BigUint `/` 语义一致；结果保持规范形态）。
pub(crate) fn div_pow10_b256(bytes: &[u8], exp: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..exp {
        div10_in_place(&mut out);
    }
    drop_left_zero_b256(&out)
}

/// 大端字节数组比较（调用方需先对齐单位；空字节 = 0）。
pub(crate) fn cmp_b256(a: &[u8], b: &[u8]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.cmp(b))
}

fn mul10_in_place(bytes: &mut Vec<u8>) {
    // 大端：从低位（末尾）向高位进位
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
    // 大端：从高位向低位传播余数
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

    /// 简易 LCG，避免为测试引入 rand 依赖。
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
            // 只对不超过 127 字节的值断言一致；更宽的应同样报错
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
        // 127 字节 = 2^1016 ≈ 10^305.85：305 位十进制数可容纳，306 位超出
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
            // 构造规范形态（去前导零）
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
                Vec::new() // 与 Amount 的零表示（空字节）一致
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
