use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;
use std::sync::OnceLock;

use num_bigint::{BigInt, BigUint, Sign as BigSign};
use num_traits::{Num, ToPrimitive, Zero};
use sys::{Rerr, Ret, decodef, errf};

use crate::codec::{Decode, Encode, Reader};

pub const UNIT_MEI: u8 = 248;
pub const UNIT_244: u8 = 244;
pub const UNIT_ZHU: u8 = 240;
pub const UNIT_238: u8 = 238;
pub const UNIT_SHUO: u8 = 232;
pub const UNIT_AI: u8 = 224;
pub const UNIT_MIAO: u8 = 216;

/// Compress policy when shortening the mantissa byte length.
pub enum AmtCpr {
    Discard, // Truncate toward zero (`/10`)
    Grow,    // Round up after divide (`/10 + 1`) — used by diamond bid fees
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Amount {
    unit: u8,
    dist: i8,
    byte: Vec<u8>,
}

impl Amount {
    pub const AMOUNT_STORE_MAX_SIZE: usize = 12;

    pub fn zero() -> Self {
        Self::default()
    }

    pub fn zero_ref() -> &'static Amount {
        static Z: OnceLock<Amount> = OnceLock::new();
        Z.get_or_init(Amount::zero)
    }

    pub fn unit(&self) -> u8 {
        self.unit
    }

    pub fn dist(&self) -> i8 {
        self.dist
    }

    pub fn byte(&self) -> &Vec<u8> {
        &self.byte
    }

    pub fn tail_len(&self) -> usize {
        self.dist.unsigned_abs() as usize
    }

    pub fn is_zero(&self) -> bool {
        self.byte.is_empty()
    }

    pub fn not_zero(&self) -> bool {
        !self.is_zero()
    }

    pub fn is_positive(&self) -> bool {
        self.dist > 0 && self.not_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.dist < 0 && self.not_zero()
    }

    pub fn first_byte(&self) -> Option<u8> {
        self.byte.first().copied()
    }

    pub fn small(v: u8, u: u8) -> Self {
        if v == 0 {
            return Self::zero();
        }
        Self {
            unit: u,
            dist: 1,
            byte: vec![v],
        }
    }

    pub fn mei(v: u64) -> Self {
        Self::coin(v, UNIT_MEI)
    }

    pub fn zhu(v: u64) -> Self {
        Self::coin(v, UNIT_ZHU)
    }

    pub fn unit238(v: u64) -> Self {
        Self::coin(v, UNIT_238)
    }

    pub fn coin(v: u64, u: u8) -> Self {
        Self::coin_u64(v, u)
    }

    pub fn coin_u64(v: u64, u: u8) -> Self {
        Self::coin_u128(v as u128, u)
    }

    /// Same as `coin_u64` but for `u128` values (e.g. `fee_purity * bytes * periods`
    /// can overflow `u64` for large contract deployments).
    pub fn coin_u128(mut v: u128, mut u: u8) -> Self {
        if v == 0 {
            return Self::zero();
        }
        while v % 10 == 0 {
            if u == 255 {
                break;
            }
            v /= 10;
            u += 1;
        }
        let byte = drop_left_zero(&v.to_be_bytes());
        Self {
            unit: u,
            dist: byte.len() as i8,
            byte,
        }
    }

    pub fn from(v: &str) -> Ret<Self> {
        let v = normalize_grouping(v.trim())?;
        if v.contains(':') {
            return Self::from_fin(&v);
        }
        Self::from_mei(&v)
    }

    fn from_fin(v: &str) -> Ret<Self> {
        let Some((number, unit)) = v.split_once(':') else {
            return errf!("amount format invalid: {}", v);
        };
        if unit.contains(':') {
            return errf!("amount format invalid: {}", v);
        }
        let unit = unit
            .parse::<u8>()
            .map_err(|_| sys::Error::fault(format!("amount unit invalid: {}", unit)))?;
        let (negative, digits) = split_sign(number);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return errf!("amount value invalid: {}", number);
        }
        Self::from_decimal_digits(negative, digits, unit)
    }

    fn from_mei(v: &str) -> Ret<Self> {
        let (negative, digits) = split_sign(v);
        let parts: Vec<&str> = digits.split('.').collect();
        if parts.len() > 2 {
            return errf!("amount value invalid: {}", v);
        }
        let int_part = parts[0];
        let frac_part = if parts.len() == 2 { parts[1] } else { "" };
        if (int_part.is_empty() && frac_part.is_empty())
            || !int_part.bytes().all(|b| b.is_ascii_digit())
            || !frac_part.bytes().all(|b| b.is_ascii_digit())
        {
            return errf!("amount value invalid: {}", v);
        }
        let frac_trimmed = frac_part.trim_end_matches('0');
        if frac_trimmed.len() > UNIT_MEI as usize {
            return errf!("amount value invalid: {}", v);
        }
        let unit = UNIT_MEI - frac_trimmed.len() as u8;
        let mut body = String::with_capacity(int_part.len() + frac_trimmed.len());
        body.push_str(int_part);
        body.push_str(frac_trimmed);
        Self::from_decimal_digits(negative, &body, unit)
    }

    pub fn from_bigint(bignum: &BigInt) -> Ret<Self> {
        if bignum.is_zero() {
            return Ok(Self::zero());
        }
        let negative = bignum.sign() == BigSign::Minus;
        let mut magnitude = bignum.magnitude().clone();
        let mut unit = 0u8;
        while unit < u8::MAX && (&magnitude % 10u8).is_zero() {
            magnitude /= 10u8;
            unit += 1;
        }
        Self::from_sign_magnitude(negative, unit, magnitude)
    }

    fn from_decimal_digits(negative: bool, digits: &str, mut unit: u8) -> Ret<Self> {
        let mut digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Ok(Self::zero());
        }
        while unit < u8::MAX && digits.ends_with('0') {
            digits = &digits[..digits.len() - 1];
            unit += 1;
        }
        if let Ok(magnitude) = digits.parse::<u128>() {
            let byte = drop_left_zero(&magnitude.to_be_bytes());
            let mut dist = byte.len() as i8;
            if negative {
                dist *= -1;
            }
            return Ok(Self { unit, dist, byte });
        }
        let magnitude = BigUint::from_str_radix(digits, 10)
            .map_err(|_| sys::Error::fault("amount value invalid"))?;
        Self::from_sign_magnitude(negative, unit, magnitude)
    }

    fn from_sign_magnitude(negative: bool, unit: u8, magnitude: BigUint) -> Ret<Self> {
        let byte = magnitude.to_bytes_be();
        if byte.len() > 127 {
            return errf!("Amount is too wide.");
        }
        let mut dist = byte.len() as i8;
        if negative {
            dist *= -1;
        }
        Ok(Self { unit, dist, byte })
    }

    pub fn from_unit_byte(unit: u8, byte: Vec<u8>) -> Ret<Self> {
        if byte.len() > 127 {
            return errf!("amount bytes len overflow 127.");
        }
        let byte = drop_left_zero(&byte);
        if byte.is_empty() {
            return Ok(Self::zero());
        }
        Ok(Self {
            unit,
            dist: byte.len() as i8,
            byte,
        })
    }

    pub fn to_bigint(&self) -> BigInt {
        if self.is_zero() {
            return BigInt::from(0u8);
        }
        let sign = if self.dist > 0 {
            BigSign::Plus
        } else {
            BigSign::Minus
        };
        let bignum = BigInt::from_bytes_be(sign, &self.byte);
        bignum * BigInt::from(10u64).pow(self.unit as u32)
    }

    pub fn to_biguint(&self) -> BigUint {
        if self.is_negative() {
            return BigUint::ZERO;
        }
        let magnitude = BigUint::from_bytes_be(&self.byte);
        magnitude * pow10_big(self.unit)
    }

    pub fn to_fin_string(&self) -> String {
        if self.is_zero() {
            return "0:0".to_owned();
        }
        let sign = if self.is_negative() { "-" } else { "" };
        let digits = mantissa_string(&self.byte);
        format!("{}{}:{}", sign, digits, self.unit)
    }

    pub fn to_unit_float(&self, base_unit: u8) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let mantissa = BigUint::from_bytes_be(&self.byte)
            .to_f64()
            .unwrap_or(f64::NAN);
        let delta = (base_unit as i64 - self.unit as i64).unsigned_abs() as f64;
        let scale = 10f64.powf(delta);
        let value = if self.unit > base_unit {
            mantissa * scale
        } else {
            mantissa / scale
        };
        if self.is_negative() { -value } else { value }
    }

    pub fn to_unit_string(&self, unit_str: &str) -> String {
        let unit = unit_str.parse::<u8>().ok().or_else(|| match unit_str {
            "mei" => Some(UNIT_MEI),
            "zhu" => Some(UNIT_ZHU),
            "shuo" => Some(UNIT_SHUO),
            "ai" => Some(UNIT_AI),
            "miao" => Some(UNIT_MIAO),
            _ => None,
        });
        match unit {
            Some(unit) if unit > 0 => self.to_unit_decimal_string(unit),
            _ => self.to_fin_string(),
        }
    }

    fn to_unit_decimal_string(&self, base_unit: u8) -> String {
        if self.is_zero() {
            return "0".to_owned();
        }
        let mut digits = mantissa_string(&self.byte);
        if self.unit >= base_unit {
            digits.extend(std::iter::repeat_n('0', (self.unit - base_unit) as usize));
        } else {
            let decimal_places = (base_unit - self.unit) as usize;
            if digits.len() <= decimal_places {
                let mut value = String::with_capacity(2 + decimal_places);
                value.push_str("0.");
                value.extend(std::iter::repeat_n('0', decimal_places - digits.len()));
                value.push_str(&digits);
                digits = value;
            } else {
                digits.insert(digits.len() - decimal_places, '.');
            }
            while digits.ends_with('0') {
                digits.pop();
            }
            if digits.ends_with('.') {
                digits.pop();
            }
        }
        if self.is_negative() {
            digits.insert(0, '-');
        }
        digits
    }

    pub fn to_mei_u64(&self) -> Ret<u64> {
        self.to_unit_biguint(UNIT_MEI)
            .and_then(|v| v.to_u64())
            .ok_or_else(|| sys::Error::fault(format!("amount {} overflow mei u64", self)))
    }

    pub fn to_mei_u128(&self) -> Ret<u128> {
        self.to_unit_biguint(UNIT_MEI)
            .and_then(|v| v.to_u128())
            .ok_or_else(|| sys::Error::fault(format!("amount {} overflow mei u128", self)))
    }

    pub fn to_244_u64(&self) -> Ret<u64> {
        self.to_244_u128().and_then(|v| {
            u64::try_from(v)
                .map_err(|_| sys::Error::fault(format!("amount {} overflow 244 u64", self)))
        })
    }

    pub fn to_244_u128(&self) -> Ret<u128> {
        self.to_unit_biguint(UNIT_244)
            .and_then(|v| v.to_u128())
            .ok_or_else(|| sys::Error::fault(format!("amount {} overflow 244 u128", self)))
    }

    pub fn to_zhu_u64(&self) -> Ret<u64> {
        self.to_zhu_u128().and_then(|v| {
            u64::try_from(v)
                .map_err(|_| sys::Error::fault(format!("amount {} overflow zhu u64", self)))
        })
    }

    pub fn to_zhu_u128(&self) -> Ret<u128> {
        self.to_unit_biguint(UNIT_ZHU)
            .and_then(|v| v.to_u128())
            .ok_or_else(|| sys::Error::fault(format!("amount {} overflow zhu u128", self)))
    }

    pub fn to_238_u64(&self) -> Ret<u64> {
        self.to_unit_biguint(UNIT_238)
            .and_then(|v| v.to_u64())
            .ok_or_else(|| sys::Error::fault(format!("amount {} overflow unit238 u64", self)))
    }

    pub fn to_238_u128(&self) -> Ret<u128> {
        self.to_unit_biguint(UNIT_238)
            .and_then(|v| v.to_u128())
            .ok_or_else(|| sys::Error::fault(format!("amount {} overflow unit238 u128", self)))
    }

    pub fn to_unit_biguint(&self, base_unit: u8) -> Option<BigUint> {
        if self.is_negative() {
            return None;
        }
        let magnitude = BigUint::from_bytes_be(&self.byte);
        Some(if self.unit >= base_unit {
            magnitude * pow10_big(self.unit - base_unit)
        } else {
            magnitude / pow10_big(base_unit - self.unit)
        })
    }

    pub fn check_store_long(&self) -> Ret<()> {
        if self.size() > Self::AMOUNT_STORE_MAX_SIZE {
            return errf!(
                "amount {} size exceeds max {}",
                self,
                Self::AMOUNT_STORE_MAX_SIZE
            );
        }
        Ok(())
    }

    pub fn add_mode_u128(&self, rhs: &Amount) -> Ret<Self> {
        self.compute_unsigned(rhs, u128::MAX, true)
    }

    pub fn sub_mode_u128(&self, rhs: &Amount) -> Ret<Self> {
        self.compute_unsigned(rhs, u128::MAX, false)
    }

    pub fn add_mode_u64(&self, rhs: &Amount) -> Ret<Self> {
        self.compute_unsigned(rhs, u64::MAX as u128, true)
    }

    pub fn sub_mode_u64(&self, rhs: &Amount) -> Ret<Self> {
        self.compute_unsigned(rhs, u64::MAX as u128, false)
    }

    fn compute_unsigned(&self, rhs: &Amount, limit: u128, add: bool) -> Ret<Self> {
        if self.is_negative() || rhs.is_negative() {
            return errf!("amount operands cannot be negative");
        }
        if self.is_zero() {
            if add || rhs.is_zero() {
                return Ok(rhs.clone());
            }
            return errf!("amount computing size overflow");
        }
        if rhs.is_zero() {
            return Ok(self.clone());
        }
        let mut lhs = tail_to_u128(&self.byte, limit)?;
        let mut rhs_value = tail_to_u128(&rhs.byte, limit)?;
        let base_unit = self.unit.min(rhs.unit);
        if self.unit > base_unit {
            lhs = scale_u128(lhs, self.unit - base_unit, limit)?;
        } else if rhs.unit > base_unit {
            rhs_value = scale_u128(rhs_value, rhs.unit - base_unit, limit)?;
        }
        let value = if add {
            lhs.checked_add(rhs_value)
        } else {
            lhs.checked_sub(rhs_value)
        }
        .filter(|value| *value <= limit)
        .ok_or_else(|| sys::Error::fault("amount computing size overflow"))?;
        Ok(Self::coin_u128(value, base_unit))
    }

    /// Lower the amount's unit exponent without changing its mantissa.
    ///
    /// Legacy Type1/Type2 extra9 fee accounting uses this operation to expose
    /// the miner-side fee at one unit below the transaction's paid fee.
    pub fn unit_sub(&self, sub: u8) -> Ret<Self> {
        if sub == 0 {
            return Ok(self.clone());
        }
        if sub > self.unit {
            return errf!("unit_sub failed: unit must be greater than {}", sub);
        }
        let mut result = self.clone();
        result.unit -= sub;
        Ok(result)
    }

    pub fn dist_mul(&self, n: u128) -> Ret<Self> {
        if self.is_zero() {
            return Ok(Self::zero());
        }
        if self.is_negative() {
            return errf!("dist_mul not supported for negative amount");
        }
        let value = tail_to_u128(&self.byte, u128::MAX)?
            .checked_mul(n)
            .ok_or_else(|| sys::Error::fault("dist_mul failed: u128 overflow"))?;
        Ok(Self::coin_u128(value, self.unit))
    }

    /// Shorten mantissa to at most `btn` bytes by raising `unit` (÷10 each step).
    /// Matches fullnodedev diamond-bid fee encoding (`compress(2, AmtCpr::Grow)`).
    pub fn compress(&self, btn: usize, cpr: AmtCpr) -> Ret<Self> {
        if self.dist < 0 {
            return errf!("cannot compress negative amount");
        }
        const U128S: usize = (u128::BITS / 8) as usize;
        if self.tail_len() > U128S {
            return errf!("amount bytes too long to compress");
        }
        let mut value = tail_to_u128(&self.byte, u128::MAX)?;
        let mut unit = self.unit;
        while u128_byte_len(value) > btn {
            if unit == 255 {
                return errf!("amount uint too large to compress");
            }
            value /= 10;
            if let AmtCpr::Grow = cpr {
                value += 1;
            }
            unit += 1;
        }
        if value == 0 {
            return Ok(Self::zero());
        }
        let byte = drop_left_zero(&value.to_be_bytes());
        Ok(Self {
            unit,
            dist: byte.len() as i8,
            byte,
        })
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_fin_string())
    }
}

impl Ord for Amount {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.is_negative(), other.is_negative()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => magnitude_cmp(self, other).reverse(),
            (false, false) => magnitude_cmp(self, other),
        }
    }
}

impl PartialOrd for Amount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Encode for Amount {
    fn size(&self) -> usize {
        2 + self.byte.len()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        out.push(self.unit);
        out.push(self.dist as u8);
        out.extend_from_slice(&self.byte);
    }
}

impl Decode for Amount {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let unit = r.read_bytes(1)?[0];
        let dist_u = r.read_bytes(1)?[0];
        if dist_u == i8::MIN as u8 {
            return decodef!("dist cannot be {}", i8::MIN);
        }
        let dist = dist_u as i8;
        let len = dist.unsigned_abs() as usize;
        let byte = r.read_bytes(len)?.to_vec();
        if len > 1 && byte.iter().all(|b| *b == 0) {
            return decodef!("multi-byte amount cannot be all zero");
        }
        if len > 1 && byte[0] == 0 {
            return decodef!("amount leading zero byte is not canonical");
        }
        if byte.iter().all(|b| *b == 0) && (unit != 0 || dist != 0 || !byte.is_empty()) {
            return decodef!("amount semantic zero is not canonical");
        }
        Ok((Self { unit, dist, byte }, r.used()))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WireAmount {
    amount: Amount,
    wire: Vec<u8>,
}

impl Default for WireAmount {
    fn default() -> Self {
        Self::from_amount(Amount::zero())
    }
}

impl fmt::Debug for WireAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireAmount")
            .field("amount", &self.amount)
            .field("wire", &self.wire)
            .finish()
    }
}

impl fmt::Display for WireAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.amount)
    }
}

impl WireAmount {
    pub fn from_amount(amount: Amount) -> Self {
        let wire = amount.encode();
        Self { amount, wire }
    }

    pub fn amount(&self) -> &Amount {
        &self.amount
    }

    pub fn wire(&self) -> &[u8] {
        &self.wire
    }

    pub fn is_canonical_wire(&self) -> bool {
        self.wire == self.amount.encode()
    }

    pub fn require_canonical_wire(&self) -> Rerr {
        sys::maybe!(
            self.is_canonical_wire(),
            Ok(()),
            errf!("amount wire encoding is not canonical")
        )
    }
}

impl Deref for WireAmount {
    type Target = Amount;
    fn deref(&self) -> &Amount {
        &self.amount
    }
}

impl From<Amount> for WireAmount {
    fn from(amount: Amount) -> Self {
        Self::from_amount(amount)
    }
}

fn try_decode_non_canonical_semantic_zero(buf: &[u8]) -> Ret<(Amount, usize)> {
    if buf.len() < 2 {
        return decodef!("buffer too short for WireAmount");
    }
    let unit = buf[0];
    let dist_raw = buf[1];
    if dist_raw == i8::MIN as u8 {
        return decodef!("dist cannot be {}", i8::MIN);
    }
    let dist = dist_raw as i8;
    let len = dist.unsigned_abs() as usize;
    if buf.len() < 2 + len {
        return decodef!("buffer too short for WireAmount");
    }
    let byte = &buf[2..2 + len];
    if len > 1 && byte.iter().all(|b| *b == 0) {
        return decodef!("multi-byte amount cannot be all zero");
    }
    if len > 1 && byte[0] == 0 {
        return decodef!("amount leading zero byte is not canonical");
    }
    if byte.iter().any(|b| *b != 0) {
        return decodef!("wire amount fallback only accepts semantic zero");
    }
    if unit == 0 && dist == 0 && byte.is_empty() {
        return decodef!("canonical zero must use Amount decode");
    }
    Ok((Amount::zero(), 2 + len))
}

impl Encode for WireAmount {
    fn size(&self) -> usize {
        self.wire.len()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.wire);
    }
}

impl Decode for WireAmount {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        if let Ok((amount, used)) = Amount::decode(buf) {
            let wire = buf[..used].to_vec();
            return Ok((Self { amount, wire }, used));
        }
        let (amount, used) = try_decode_non_canonical_semantic_zero(buf)?;
        let wire = buf[..used].to_vec();
        Ok((Self { amount, wire }, used))
    }
}

fn drop_left_zero(v: &[u8]) -> Vec<u8> {
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

fn split_sign(value: &str) -> (bool, &str) {
    match value.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, value),
    }
}

fn normalize_grouping(value: &str) -> Ret<String> {
    if !value.contains(',') {
        return Ok(value.to_owned());
    }
    let number_end = value.find(['.', ':']).unwrap_or(value.len());
    let (number, suffix) = value.split_at(number_end);
    if suffix.contains(',') {
        return errf!("amount comma grouping invalid: {}", value);
    }
    let unsigned = number.strip_prefix('-').unwrap_or(number);
    let groups: Vec<&str> = unsigned.split(',').collect();
    let valid_first = groups.first().is_some_and(|group| {
        !group.is_empty() && group.len() <= 3 && group.bytes().all(|b| b.is_ascii_digit())
    });
    let valid_rest = groups
        .iter()
        .skip(1)
        .all(|group| group.len() == 3 && group.bytes().all(|b| b.is_ascii_digit()));
    if groups.len() < 2 || !valid_first || !valid_rest {
        return errf!("amount comma grouping invalid: {}", value);
    }
    Ok(value.replace(',', ""))
}

fn pow10_big(exp: u8) -> BigUint {
    BigUint::from(10u8).pow(exp as u32)
}

fn mantissa_string(bytes: &[u8]) -> String {
    if bytes.len() <= size_of::<u128>() {
        tail_to_u128(bytes, u128::MAX)
            .expect("16-byte amount fits u128")
            .to_string()
    } else {
        BigUint::from_bytes_be(bytes).to_string()
    }
}

fn tail_to_u128(bytes: &[u8], limit: u128) -> Ret<u128> {
    if bytes.len() > size_of::<u128>() {
        return errf!("amount computing size overflow");
    }
    let mut padded = [0u8; size_of::<u128>()];
    padded[size_of::<u128>() - bytes.len()..].copy_from_slice(bytes);
    let value = u128::from_be_bytes(padded);
    if value > limit {
        return errf!("amount computing size overflow");
    }
    Ok(value)
}

fn scale_u128(value: u128, exp: u8, limit: u128) -> Ret<u128> {
    let factor = 10u128
        .checked_pow(exp as u32)
        .ok_or_else(|| sys::Error::fault("amount computing size overflow"))?;
    value
        .checked_mul(factor)
        .filter(|value| *value <= limit)
        .ok_or_else(|| sys::Error::fault("amount computing size overflow"))
}

fn u128_byte_len(value: u128) -> usize {
    ((u128::BITS - value.leading_zeros()) as usize).div_ceil(8)
}

fn magnitude_cmp(lhs: &Amount, rhs: &Amount) -> Ordering {
    if lhs.is_zero() || rhs.is_zero() {
        return lhs.not_zero().cmp(&rhs.not_zero());
    }
    if lhs.unit == rhs.unit {
        return lhs
            .byte
            .len()
            .cmp(&rhs.byte.len())
            .then_with(|| lhs.byte.cmp(&rhs.byte));
    }
    if lhs.byte.len() <= size_of::<u128>() && rhs.byte.len() <= size_of::<u128>() {
        let lhs_value = tail_to_u128(&lhs.byte, u128::MAX).expect("16-byte amount fits u128");
        let rhs_value = tail_to_u128(&rhs.byte, u128::MAX).expect("16-byte amount fits u128");
        let base_unit = lhs.unit.min(rhs.unit);
        let scaled_lhs = scale_u128(lhs_value, lhs.unit - base_unit, u128::MAX);
        let scaled_rhs = scale_u128(rhs_value, rhs.unit - base_unit, u128::MAX);
        if let (Ok(scaled_lhs), Ok(scaled_rhs)) = (scaled_lhs, scaled_rhs) {
            return scaled_lhs.cmp(&scaled_rhs);
        }
    }
    let mut lhs_value = BigUint::from_bytes_be(&lhs.byte);
    let mut rhs_value = BigUint::from_bytes_be(&rhs.byte);
    if lhs.unit > rhs.unit {
        lhs_value *= pow10_big(lhs.unit - rhs.unit);
    } else {
        rhs_value *= pow10_big(rhs.unit - lhs.unit);
    }
    lhs_value.cmp(&rhs_value)
}
