use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;
use std::sync::OnceLock;

use sys::{Rerr, Ret, errf, normalf};

use crate::codec::{Decode, Encode, Reader};
use crate::json::{FromJSON, JSONFormater, ToJSON, json_expect_quoted_decoded};

/// Amount's big-integer path is the `amount_base256` byte-array core;
/// `num-bigint` remains only as a dev-dependency test oracle.
use crate::types::amount_base256 as b256;

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
        // Match BigUint: leading zeros are insignificant, all-zero bytes are 0.
        // Production constructors/decode already collapse this to empty bytes.
        self.byte.iter().all(|&b| b == 0)
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
        // >u128 (>39 decimal digits): base-256 byte-array core (the BigUint
        // reference path is exercised by the test oracle).
        let byte = b256::from_decimal_b256(digits)?;
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

    pub fn to_fin_string(&self) -> String {
        if self.is_zero() {
            return "0:0".to_owned();
        }
        let digits = mantissa_string(&self.byte);
        let mut s = String::with_capacity(digits.len() + 5);
        if self.is_negative() {
            s.push('-');
        }
        s.push_str(&digits);
        s.push(':');
        // unit is a 0..=255 decimal exponent; write it without the fmt machinery
        let mut buf = [0u8; 3];
        let mut n = 0;
        let mut unit = self.unit;
        loop {
            buf[n] = b'0' + unit % 10;
            n += 1;
            unit /= 10;
            if unit == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            s.push(buf[i] as char);
        }
        s
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
        u64::try_from(self.to_unit_u128(UNIT_MEI)?)
            .map_err(|_| sys::Error::fault(format!("amount {} overflow mei u64", self)))
    }

    pub fn to_mei_u128(&self) -> Ret<u128> {
        self.to_unit_u128(UNIT_MEI)
    }

    pub fn to_244_u64(&self) -> Ret<u64> {
        u64::try_from(self.to_unit_u128(UNIT_244)?)
            .map_err(|_| sys::Error::fault(format!("amount {} overflow 244 u64", self)))
    }

    pub fn to_244_u128(&self) -> Ret<u128> {
        self.to_unit_u128(UNIT_244)
    }

    pub fn to_zhu_u64(&self) -> Ret<u64> {
        u64::try_from(self.to_unit_u128(UNIT_ZHU)?)
            .map_err(|_| sys::Error::fault(format!("amount {} overflow zhu u64", self)))
    }

    pub fn to_zhu_u128(&self) -> Ret<u128> {
        self.to_unit_u128(UNIT_ZHU)
    }

    pub fn to_238_u64(&self) -> Ret<u64> {
        u64::try_from(self.to_unit_u128(UNIT_238)?)
            .map_err(|_| sys::Error::fault(format!("amount {} overflow unit238 u64", self)))
    }

    pub fn to_238_u128(&self) -> Ret<u128> {
        self.to_unit_u128(UNIT_238)
    }

    /// u128 value of the amount scaled to `base_unit`, identical to the BigUint
    /// path (negative values, wide quotients and scaling overflow all error).
    fn to_unit_u128(&self, base_unit: u8) -> Ret<u128> {
        if self.is_negative() {
            return errf!("amount {} overflow unit u128", self);
        }
        if self.unit >= base_unit {
            let magnitude = tail_to_u128(&self.byte, u128::MAX)?;
            scale_u128(magnitude, self.unit - base_unit, u128::MAX)
        } else if self.byte.len() <= size_of::<u128>() {
            let magnitude = tail_to_u128(&self.byte, u128::MAX)?;
            let k = (base_unit - self.unit) as u32;
            if k > 38 {
                // 10^k exceeds u128 range (magnitude <= u128::MAX < 10^39 <= 10^k),
                // so the exact quotient is always 0, matching the BigUint path.
                Ok(0)
            } else {
                Ok(magnitude / 10u128.pow(k))
            }
        } else {
            // Arbitrary-precision division (>16-byte mantissa): the quotient may
            // still fit in u128 (e.g. 0 after a large enough 10^k).
            let quotient = b256::div_pow10_b256(&self.byte, base_unit - self.unit);
            tail_to_u128(&quotient, u128::MAX)
        }
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
    /// Legacy Type1/Type2 extra9 fee accounting uses this to expose the miner-side fee one unit below the paid fee.
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
            return normalf!("dist cannot be {}", i8::MIN);
        }
        let dist = dist_u as i8;
        let len = dist.unsigned_abs() as usize;
        let byte = r.read_bytes(len)?.to_vec();
        if len > 1 && byte.iter().all(|b| *b == 0) {
            return normalf!("multi-byte amount cannot be all zero");
        }
        if len > 1 && byte[0] == 0 {
            return normalf!("amount leading zero byte is not canonical");
        }
        if byte.iter().all(|b| *b == 0) && (unit != 0 || dist != 0 || !byte.is_empty()) {
            return normalf!("amount semantic zero is not canonical");
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
        return normalf!("buffer too short for WireAmount");
    }
    let unit = buf[0];
    let dist_raw = buf[1];
    if dist_raw == i8::MIN as u8 {
        return normalf!("dist cannot be {}", i8::MIN);
    }
    let dist = dist_raw as i8;
    let len = dist.unsigned_abs() as usize;
    if buf.len() < 2 + len {
        return normalf!("buffer too short for WireAmount");
    }
    let byte = &buf[2..2 + len];
    if len > 1 && byte.iter().all(|b| *b == 0) {
        return normalf!("multi-byte amount cannot be all zero");
    }
    if len > 1 && byte[0] == 0 {
        return normalf!("amount leading zero byte is not canonical");
    }
    if byte.iter().any(|b| *b != 0) {
        return normalf!("wire amount fallback only accepts semantic zero");
    }
    if unit == 0 && dist == 0 && byte.is_empty() {
        return normalf!("canonical zero must use Amount decode");
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

impl ToJSON for WireAmount {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        self.amount.to_json_fmt(fmt)
    }
}

impl FromJSON for WireAmount {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let amount = Amount::from(&json_expect_quoted_decoded(json)?)?;
        *self = Self::from_amount(amount);
        Ok(())
    }
}

fn drop_left_zero(v: &[u8]) -> Vec<u8> {
    match v.iter().position(|&b| b != 0) {
        Some(i) => v[i..].to_vec(),
        None => Vec::new(),
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

fn mantissa_string(bytes: &[u8]) -> String {
    if bytes.len() <= size_of::<u128>() {
        let value = tail_to_u128(bytes, u128::MAX).expect("16-byte amount fits u128");
        u128_to_decimal(value)
    } else {
        b256::to_decimal_b256(bytes)
    }
}

/// u128 → decimal string without the `core::fmt::num` machinery (amount
/// formatting is on the JSON hot path; avoids pulling u128 Display into wasm).
fn u128_to_decimal(value: u128) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let mut buf = [0u8; 39]; // max 39 decimal digits for u128
    let mut i = buf.len();
    let mut v = value;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    // SAFETY: buf[i..] contains only ASCII digits
    unsafe { String::from_utf8_unchecked(buf[i..].to_vec()) }
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
        return b256::cmp_b256(&lhs.byte, &rhs.byte);
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
    // Arbitrary-precision fallback: base-256 aligned comparison (the BigUint
    // reference is exercised by the test oracle).
    magnitude_cmp_wide(lhs, rhs)
}

/// Arbitrary-precision comparison goes through base-256 alignment (the BigUint
/// reference is exercised by the test oracle).
fn magnitude_cmp_wide(lhs: &Amount, rhs: &Amount) -> Ordering {
    let base_unit = lhs.unit.min(rhs.unit);
    let lhs_value = b256::mul_pow10_b256(&lhs.byte, lhs.unit - base_unit);
    let rhs_value = b256::mul_pow10_b256(&rhs.byte, rhs.unit - base_unit);
    b256::cmp_b256(&lhs_value, &rhs_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::{BigInt, BigUint};
    use num_traits::{ToPrimitive, Zero};

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
        fn pick(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn random_amount(lcg: &mut Lcg) -> Amount {
        let len = (lcg.next() % 20) as usize + 1;
        let mut byte = Vec::with_capacity(len);
        for _ in 0..len {
            byte.push(lcg.byte());
        }
        // Canonical form: no leading zeros (all zeros collapse to empty)
        while byte.len() > 1 && byte[0] == 0 {
            byte.remove(0);
        }
        if byte.iter().all(|&b| b == 0) {
            byte.clear();
        }
        let unit = (lcg.next() % 256) as u8;
        let dist = if byte.is_empty() {
            0
        } else if lcg.next() % 2 == 0 {
            byte.len() as i8
        } else {
            -(byte.len() as i8)
        };
        Amount { unit, dist, byte }
    }

    /// Amounts with a >16-byte mantissa: base-256 `to_fin_string` and the BigUint
    /// formatter must produce byte-identical output (the two cores compared directly).
    #[test]
    fn fin_string_matches_bigint_for_wide_mantissa() {
        let mut lcg = Lcg(0x77aa);
        for _ in 0..200 {
            let len = 17 + (lcg.next() % 110) as usize;
            let mut byte = Vec::with_capacity(len);
            for _ in 0..len {
                byte.push(lcg.byte());
            }
            byte[0] |= 0x80; // guarantees no leading zero and >16 bytes
            let expected = BigUint::from_bytes_be(&byte).to_string();
            assert_eq!(mantissa_string(&byte), expected);
        }
    }

    // --- Wide-mantissa oracle tests (num-bigint as test oracle) ---
    // Random amounts up to the 127-byte cap checked against an independent num-bigint oracle.

    /// Magnitude oracle: mantissa as BigUint, scaled by 10^unit (exact).
    fn oracle_magnitude(a: &Amount) -> BigUint {
        if a.is_zero() {
            return BigUint::zero();
        }
        BigUint::from_bytes_be(a.byte()) * BigUint::from(10u8).pow(a.unit() as u32)
    }

    /// Signed oracle.
    fn oracle_signed(a: &Amount) -> BigInt {
        let m = oracle_magnitude(a);
        if a.is_negative() {
            -BigInt::from(m)
        } else {
            BigInt::from(m)
        }
    }

    /// Oracle for to_fin_string: the fin form is the mantissa decimal with the
    /// unit as a suffix, not the scaled value.
    fn oracle_fin_string(a: &Amount) -> String {
        if a.is_zero() {
            return "0:0".to_owned();
        }
        let m = BigUint::from_bytes_be(a.byte()).to_string();
        format!(
            "{}{}:{}",
            if a.is_negative() { "-" } else { "" },
            m,
            a.unit()
        )
    }

    /// Wide random amount: mantissa 1..=127 bytes (weighted toward the u128
    /// boundary and the 127-byte cap), random unit and sign.
    fn random_wide_amount(lcg: &mut Lcg) -> Amount {
        let len = match lcg.pick(10) {
            0 => 15,
            1 => 16,
            2 => 17,
            3 => 18,
            4 => 31,
            5 => 63,
            6 => 127,
            _ => 1 + lcg.pick(127),
        };
        let mut v = vec![0u8; len];
        for b in v.iter_mut() {
            *b = lcg.byte();
        }
        v[0] |= 0x80; // canonical: msb nonzero
        let unit = lcg.pick(256) as u8;
        let neg = lcg.pick(2) == 0;
        let dist = if neg { -(v.len() as i8) } else { v.len() as i8 };
        Amount {
            unit,
            dist,
            byte: v,
        }
    }

    /// Independent oracle for to_unit_string (bases 1..=255): exact terminating
    /// decimal of value/10^base from the BigUint magnitude (base 0 never reaches this path).
    fn oracle_unit_string(a: &Amount, base: u8) -> String {
        debug_assert!(base > 0);
        if a.is_zero() {
            return "0".to_owned();
        }
        let m = oracle_magnitude(a); // M × 10^unit
        let mut s = if a.unit() >= base {
            (m / BigUint::from(10u8).pow(base as u32)).to_string()
        } else {
            let k = base as usize;
            let ms = m.to_string();
            let mut out = String::new();
            if ms.len() <= k {
                out.push_str("0.");
                out.push_str(&"0".repeat(k - ms.len()));
                out.push_str(&ms);
            } else {
                out.push_str(&ms[..ms.len() - k]);
                out.push('.');
                out.push_str(&ms[ms.len() - k..]);
            }
            while out.ends_with('0') {
                out.pop();
            }
            if out.ends_with('.') {
                out.pop();
            }
            out
        };
        if a.is_negative() {
            s.insert(0, '-');
        }
        s
    }

    fn oracle_cmp(a: &Amount, b: &Amount) -> Ordering {
        match (a.is_negative(), b.is_negative()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => oracle_mag_cmp(a, b).reverse(),
            (false, false) => oracle_mag_cmp(a, b),
        }
    }

    fn oracle_mag_cmp(a: &Amount, b: &Amount) -> Ordering {
        if a.is_zero() || b.is_zero() {
            return a.not_zero().cmp(&b.not_zero());
        }
        oracle_magnitude(a).cmp(&oracle_magnitude(b))
    }

    #[test]
    fn fin_string_matches_oracle_for_wide() {
        let mut lcg = Lcg(0x1234);
        for _ in 0..3000 {
            let a = random_wide_amount(&mut lcg);
            assert_eq!(a.to_fin_string(), oracle_fin_string(&a), "amount={a:?}");
        }
    }

    #[test]
    fn parse_roundtrip_preserves_value_for_wide() {
        let mut lcg = Lcg(0x5678);
        for _ in 0..2000 {
            let a = random_wide_amount(&mut lcg);
            let s = a.to_fin_string();
            let b = Amount::from(&s).unwrap();
            assert_eq!(oracle_signed(&b), oracle_signed(&a), "roundtrip {s}");
        }
        for s in ["0", "0:0", "-0", "0:244"] {
            let a = Amount::from(s).unwrap();
            assert_eq!(oracle_signed(&a), BigInt::from(0), "zero form {s}");
        }
        // Cap boundary through the full parser: 127-byte mantissa cap — all-nines
        // 305 fit / 306 do not; powers of ten raise the unit instead of erroring.
        assert!(Amount::from(&"9".repeat(305)).is_ok());
        assert!(Amount::from(&"9".repeat(306)).is_err());
        let a = Amount::from(&format!("1{}", "0".repeat(305))).unwrap();
        assert_eq!(oracle_signed(&a), BigInt::from(10u8).pow(553));
    }

    #[test]
    fn unit_string_matches_oracle_for_wide() {
        let mut lcg = Lcg(0x9abc);
        for _ in 0..3000 {
            let a = random_wide_amount(&mut lcg);
            let base = 1 + lcg.pick(255) as u8; // base 0 falls back to fin form
            let expected = oracle_unit_string(&a, base);
            assert_eq!(
                a.to_unit_string(&base.to_string()),
                expected,
                "amount={a:?} base={base}"
            );
            // documented fallback: unit 0 and unknown units return the fin form
            assert_eq!(a.to_unit_string("0"), a.to_fin_string(), "amount={a:?}");
            assert_eq!(a.to_unit_string("bogus"), a.to_fin_string(), "amount={a:?}");
        }
    }

    #[test]
    fn ordering_matches_oracle_for_wide() {
        let mut lcg = Lcg(0x0f0f);
        for _ in 0..3000 {
            let a = random_wide_amount(&mut lcg);
            let b = random_wide_amount(&mut lcg);
            assert_eq!(a.cmp(&b), oracle_cmp(&a, &b), "a={a:?} b={b:?}");
            assert_eq!(a.cmp(&a.clone()), Ordering::Equal);
        }
        // sign/zero sanity beyond random coverage
        let z = Amount::zero();
        let p = Amount::small(1, 0);
        let n = Amount {
            unit: 0,
            dist: -1,
            byte: vec![1],
        };
        assert_eq!(z.cmp(&z), Ordering::Equal);
        assert_eq!(z.cmp(&p), Ordering::Less);
        assert_eq!(p.cmp(&z), Ordering::Greater);
        assert_eq!(n.cmp(&z), Ordering::Less);
        assert_eq!(n.cmp(&p), Ordering::Less);
        assert_eq!(p.cmp(&n), Ordering::Greater);
    }

    #[test]
    fn small_mantissa_matches_oracle() {
        // u128 fast path and zero-with-arbitrary-unit forms: 1..=20-byte mantissas
        // straddle the 16-byte u128 boundary and may collapse to zero.
        let mut lcg = Lcg(0x2020);
        for _ in 0..1500 {
            let a = random_amount(&mut lcg);
            let base = 1 + lcg.pick(255) as u8;
            assert_eq!(a.to_fin_string(), oracle_fin_string(&a), "amount={a:?}");
            assert_eq!(
                a.to_unit_string(&base.to_string()),
                oracle_unit_string(&a, base),
                "amount={a:?} base={base}"
            );
            let b = random_amount(&mut lcg);
            assert_eq!(a.cmp(&b), oracle_cmp(&a, &b), "a={a:?} b={b:?}");
            let got = a.to_unit_u128(base);
            let expected = if a.is_negative() {
                Err(())
            } else {
                let q = oracle_magnitude(&a) / BigUint::from(10u8).pow(base as u32);
                q.to_u128().ok_or(())
            };
            match (got, expected) {
                (Ok(g), Ok(e)) => assert_eq!(g, e, "amount={a:?} base={base}"),
                (Err(_), Err(())) => {}
                (Ok(g), Err(())) => panic!("Ok({g}) but oracle overflow: {a:?} @{base}"),
                (Err(e), Ok(_)) => panic!("Err({e}) but oracle ok: {a:?} @{base}"),
            }
        }
    }

    #[test]
    fn to_unit_u128_matches_oracle_for_wide() {
        let mut lcg = Lcg(0x5a5a);
        for _ in 0..2000 {
            let a = random_wide_amount(&mut lcg);
            let base = lcg.pick(256) as u8;
            let got = a.to_unit_u128(base);
            let expected = if a.is_negative() {
                Err(())
            } else {
                let q = oracle_magnitude(&a) / BigUint::from(10u8).pow(base as u32);
                q.to_u128().ok_or(())
            };
            match (got, expected) {
                (Ok(g), Ok(e)) => assert_eq!(g, e, "amount={a:?} base={base}"),
                (Err(_), Err(())) => {}
                (Ok(g), Err(())) => panic!("Ok({g}) but oracle overflow: {a:?} @{base}"),
                (Err(e), Ok(_)) => panic!("Err({e}) but oracle ok: {a:?} @{base}"),
            }
        }
    }

    #[test]
    fn unit_converters_match_oracle_for_wide() {
        // the 8 public to_*_u64/u128 converters vs an oracle derived directly
        // from the BigUint magnitude (negative or overflowing => Err)
        let mut lcg = Lcg(0xfeed);
        let cases: Vec<(Box<dyn Fn(&Amount) -> Ret<u128>>, u8, bool)> = vec![
            (
                Box::new(|a| a.to_mei_u64().map(u128::from)),
                UNIT_MEI,
                false,
            ),
            (Box::new(|a| a.to_mei_u128()), UNIT_MEI, true),
            (
                Box::new(|a| a.to_244_u64().map(u128::from)),
                UNIT_244,
                false,
            ),
            (Box::new(|a| a.to_244_u128()), UNIT_244, true),
            (
                Box::new(|a| a.to_zhu_u64().map(u128::from)),
                UNIT_ZHU,
                false,
            ),
            (Box::new(|a| a.to_zhu_u128()), UNIT_ZHU, true),
            (
                Box::new(|a| a.to_238_u64().map(u128::from)),
                UNIT_238,
                false,
            ),
            (Box::new(|a| a.to_238_u128()), UNIT_238, true),
        ];
        for _ in 0..1500 {
            let a = random_wide_amount(&mut lcg);
            for (conv, base, wide) in &cases {
                let got = conv(&a);
                let expected = if a.is_negative() {
                    None
                } else {
                    let q = oracle_magnitude(&a) / BigUint::from(10u8).pow(*base as u32);
                    if *wide {
                        q.to_u128()
                    } else {
                        q.to_u64().map(u128::from)
                    }
                };
                match (got, expected) {
                    (Ok(g), Some(e)) => assert_eq!(g, e, "amount={a:?} base={base}"),
                    (Err(_), None) => {}
                    (Ok(g), None) => panic!("converter Ok({g}) but oracle overflow: {a:?} @{base}"),
                    (Err(e), Some(_)) => panic!("converter Err({e}) but oracle ok: {a:?} @{base}"),
                }
            }
        }
    }

    #[test]
    fn parse_wide_with_unit_matches_oracle() {
        let mut lcg = Lcg(0x7777);
        for _ in 0..2000 {
            let len = 1 + lcg.pick(400);
            let mut s = String::with_capacity(len);
            for _ in 0..len {
                s.push((b'0' + lcg.pick(10) as u8) as char);
            }
            let t = s.trim_start_matches('0');
            let m = if t.is_empty() {
                BigUint::zero()
            } else {
                BigUint::parse_bytes(t.as_bytes(), 10).unwrap()
            };
            if m.to_bytes_be().len() > 127 {
                continue; // over-cap errors covered elsewhere
            }
            let unit = lcg.pick(256) as u8;
            let neg = lcg.pick(2) == 0;
            let fin = format!("{}{}:{}", if neg { "-" } else { "" }, &s, unit);
            let a = Amount::from(&fin).unwrap();
            let expected = m * BigUint::from(10u8).pow(unit as u32);
            assert_eq!(oracle_magnitude(&a), expected, "fin={fin}");
        }
    }

    #[test]
    fn wire_roundtrip_for_wide() {
        let mut lcg = Lcg(0x4242);
        for _ in 0..2000 {
            let a = random_wide_amount(&mut lcg);
            let enc = a.encode();
            let (d, used) = Amount::decode(&enc).unwrap();
            assert_eq!(used, enc.len());
            assert_eq!(d, a, "wire roundtrip {a:?}");
        }
        // the 127-byte wire cap
        assert!(Amount::from_unit_byte(0, vec![0u8; 128]).is_err());
        assert!(Amount::from_unit_byte(0, vec![0xffu8; 127]).is_ok());
    }

    fn raw_amount(unit: u8, byte: Vec<u8>, negative: bool) -> Amount {
        let dist = if byte.is_empty() {
            0
        } else if negative {
            -(byte.len() as i8)
        } else {
            byte.len() as i8
        };
        Amount { unit, dist, byte }
    }

    #[test]
    fn equivalent_encodings_compare_equal() {
        // Parser collapses trailing decimal zeros into unit; these structs
        // keep distinct (unit, mantissa) pairs of the same value.
        let a = raw_amount(2, vec![1], false); // 1 × 10^2
        let b = raw_amount(1, vec![10], false); // 10 × 10^1
        let c = raw_amount(0, vec![100], false); // 100 × 10^0
        assert_eq!(a.cmp(&b), Ordering::Equal);
        assert_eq!(b.cmp(&c), Ordering::Equal);
        assert_eq!(a.cmp(&c), Ordering::Equal);
        assert_ne!(a, b); // PartialEq is structural, not numeric

        let na = raw_amount(2, vec![1], true);
        let nb = raw_amount(1, vec![10], true);
        assert_eq!(na.cmp(&nb), Ordering::Equal);
        assert_eq!(na.cmp(&a), Ordering::Less);

        // leading-zero mantissa (non-canonical in-memory) vs canonical
        let padded = raw_amount(0, vec![0, 100], false);
        assert_eq!(padded.cmp(&c), Ordering::Equal);
        assert!(padded.is_zero() == c.is_zero());

        let z_empty = Amount::zero();
        let z_byte = raw_amount(7, vec![0, 0], false);
        assert!(z_byte.is_zero());
        assert_eq!(z_empty.cmp(&z_byte), Ordering::Equal);
        assert_eq!(z_byte.cmp(&a), Ordering::Less);
    }

    #[test]
    fn equivalent_wide_encodings_compare_equal() {
        let mut lcg = Lcg(0xec0d);
        for _ in 0..400 {
            let mut mantissa = vec![0u8; 17 + lcg.pick(8)];
            for b in mantissa.iter_mut() {
                *b = lcg.byte();
            }
            mantissa[0] |= 0x80;
            let shift = 1 + lcg.pick(40) as u8;
            if mantissa.len() + (shift as usize) / 2 + 2 > 120 {
                continue;
            }
            let scaled = b256::mul_pow10_b256(&mantissa, shift);
            let high = raw_amount(shift, mantissa, false);
            let low = raw_amount(0, scaled, false);
            assert_eq!(high.cmp(&low), oracle_cmp(&high, &low));
            assert_eq!(high.cmp(&low), Ordering::Equal, "high={high:?} low={low:?}");
        }
    }

    #[test]
    fn cmp_falls_back_to_b256_when_u128_scale_overflows() {
        // 16-byte mantissa × 10^55 exceeds u128, so the fast path must not
        // silently mis-order; the wide core has to return Equal / Greater.
        let one = raw_amount(255, vec![1], false);
        let eq = raw_amount(200, b256::mul_pow10_b256(&[1], 55), false);
        assert_eq!(one.cmp(&eq), Ordering::Equal);
        assert_eq!(oracle_cmp(&one, &eq), Ordering::Equal);

        let max16 = vec![0xffu8; 16];
        let hi = raw_amount(255, max16.clone(), false);
        let lo = raw_amount(200, max16, false);
        assert_eq!(hi.cmp(&lo), Ordering::Greater);
        assert_eq!(oracle_cmp(&hi, &lo), Ordering::Greater);
        assert_eq!(lo.cmp(&hi), Ordering::Less);
    }

    #[test]
    fn u128_fast_path_matches_b256_core() {
        let cases: &[u128] = &[
            0,
            1,
            10,
            255,
            256,
            u64::MAX as u128,
            u64::MAX as u128 + 1,
            u128::MAX,
        ];
        for &v in cases {
            let via_u128 = drop_left_zero(&v.to_be_bytes());
            let via_b256 = b256::from_decimal_b256(&v.to_string()).unwrap();
            assert_eq!(via_u128, via_b256, "digits={v}");
            assert_eq!(mantissa_string(&via_u128), v.to_string());
            assert_eq!(b256::to_decimal_b256(&via_u128), v.to_string());
            let amt = Amount::from(&format!("{v}:0")).unwrap();
            if v == 0 {
                assert!(amt.is_zero());
            } else {
                assert_eq!(oracle_signed(&amt), BigInt::from(v));
            }
        }
        let mut lcg = Lcg(0x1212);
        for _ in 0..500 {
            let mut buf = [0u8; 16];
            for b in buf.iter_mut() {
                *b = lcg.byte();
            }
            buf[0] |= 0x80;
            let v = u128::from_be_bytes(buf);
            let via_u128 = drop_left_zero(&buf);
            let via_b256 = b256::from_decimal_b256(&v.to_string()).unwrap();
            assert_eq!(via_u128, via_b256);
            assert_eq!(mantissa_string(&via_u128), b256::to_decimal_b256(&via_b256));
        }
    }

    #[test]
    fn decode_rejects_non_canonical_and_wireamount_zero_fallback() {
        let canonical_zero = Amount::decode(&[0, 0]).unwrap();
        assert!(canonical_zero.0.is_zero());
        assert_eq!(canonical_zero.1, 2);

        let one = Amount::decode(&[248, 1, 1]).unwrap();
        assert_eq!(one.0, Amount::small(1, 248));

        for (label, buf) in [
            ("leading zero", &[248u8, 2, 0, 1][..]),
            ("multi-byte all zero", &[248, 2, 0, 0][..]),
            ("semantic zero unit", &[248, 0][..]),
            ("semantic zero dist=1 byte=0", &[0, 1, 0][..]),
            ("dist i8::MIN", &[0, 128][..]),
        ] {
            assert!(
                Amount::decode(buf).is_err(),
                "Amount::decode must reject {label}: {buf:?}"
            );
        }

        // WireAmount keeps the raw bytes for historical semantic zeros.
        let (wa, used) = WireAmount::decode(&[248, 0]).unwrap();
        assert!(wa.amount().is_zero());
        assert_eq!(used, 2);
        assert_eq!(wa.wire(), &[248, 0]);
        assert!(!wa.is_canonical_wire());
        assert!(wa.require_canonical_wire().is_err());

        let (wa1, used1) = WireAmount::decode(&[248, 1, 0]).unwrap();
        assert!(wa1.amount().is_zero());
        assert_eq!(used1, 3);
        assert_eq!(wa1.wire(), &[248, 1, 0]);

        let (wz, _) = WireAmount::decode(&[0, 0]).unwrap();
        assert!(wz.amount().is_zero());
        assert!(wz.is_canonical_wire());

        let stripped = Amount::from_unit_byte(5, vec![0, 0, 7]).unwrap();
        assert_eq!(stripped.unit(), 5);
        assert_eq!(stripped.byte(), &vec![7]);
        assert!(Amount::from_unit_byte(5, vec![0, 0]).unwrap().is_zero());
    }
}
