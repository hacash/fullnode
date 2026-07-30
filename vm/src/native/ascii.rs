use std::collections::BTreeMap;

use crate::rt::SpaceCap;
use crate::value::checked_value_output_len;

use super::*;

const ASCII_ERR_OK: u16 = 0;
const ASCII_ERR_FORMAT: u16 = 1;
const ASCII_ERR_INVALID_CHAR: u16 = 2;
const ASCII_ERR_EMPTY: u16 = 3;
const ASCII_ERR_DUP_KEY: u16 = 4;
const ASCII_ERR_OVERFLOW: u16 = 5;
const ASCII_ERR_HEX_ODD: u16 = 6;
const ASCII_ERR_SPACE_CAP: u16 = 7;

const ASCII_CLASS_ALNUM: u8 = 1;
const ASCII_CLASS_IDENT: u8 = 2;
const ASCII_CLASS_DEC: u8 = 3;
const ASCII_CLASS_HEX: u8 = 4;
const ASCII_CLASS_BASE58: u8 = 5;
const ASCII_CLASS_LOWER: u8 = 6;
const ASCII_CLASS_UPPER: u8 = 7;

const KV_FLAG_ALLOW_OUTER_WS: u8 = 1 << 0;
const KV_FLAG_ALLOW_INNER_WS: u8 = 1 << 1;
const KV_FLAG_ALLOW_EMPTY_DOC: u8 = 1 << 2;
const KV_FLAG_QUOTED_VALUE: u8 = 1 << 3;
const KV_FLAG_LOWER_KEYS: u8 = 1 << 4;
const KV_FLAG_LOWER_VALUES: u8 = 1 << 5;

const TXT_FLAG_TRIM_ASCII_WS: u8 = 1 << 0;
const TXT_FLAG_ALLOW_EMPTY: u8 = 1 << 1;
const TXT_FLAG_ALLOW_DASH_AS_EMPTY: u8 = 1 << 2;
const TXT_FLAG_TO_LOWER: u8 = 1 << 3;
const TXT_FLAG_TO_UPPER: u8 = 1 << 4;

const DEC_FLAG_TRIM_ASCII_WS: u8 = 1 << 0;
const DEC_FLAG_ALLOW_EMPTY_AS_ZERO: u8 = 1 << 1;
const DEC_FLAG_ALLOW_DASH_AS_ZERO: u8 = 1 << 2;
const DEC_FLAG_SUFFIX_CASE_INSENSITIVE: u8 = 1 << 3;

const DEC_UNIT_H100: u8 = 1 << 0;
const DEC_UNIT_K1E3: u8 = 1 << 1;
const DEC_UNIT_M1E6: u8 = 1 << 2;
const DEC_UNIT_B1E9: u8 = 1 << 3;
const DEC_UNIT_T1E12: u8 = 1 << 4;

fn ascii_ws(ch: u8) -> bool {
    matches!(ch, b' ' | b'\t' | b'\n' | b'\r')
}

fn ascii_to_lower(ch: u8) -> u8 {
    if ch.is_ascii_uppercase() { ch + 32 } else { ch }
}

fn ascii_to_upper(ch: u8) -> u8 {
    if ch.is_ascii_lowercase() { ch - 32 } else { ch }
}

fn ascii_is_base58(ch: u8) -> bool {
    matches!(ch, b'1'..=b'9')
        || matches!(ch, b'A'..=b'H')
        || matches!(ch, b'J'..=b'N')
        || matches!(ch, b'P'..=b'Z')
        || matches!(ch, b'a'..=b'k')
        || matches!(ch, b'm'..=b'z')
}

fn class_byte_predicate(class_id: u8) -> VmrtRes<fn(u8) -> bool> {
    Ok(match class_id {
        ASCII_CLASS_ALNUM => |ch| ch.is_ascii_alphanumeric(),
        ASCII_CLASS_IDENT => |ch| ch.is_ascii_alphanumeric() || ch == b'_',
        ASCII_CLASS_DEC => |ch| ch.is_ascii_digit(),
        ASCII_CLASS_HEX => |ch| ch.is_ascii_hexdigit(),
        ASCII_CLASS_BASE58 => ascii_is_base58,
        ASCII_CLASS_LOWER => |ch| ch.is_ascii_lowercase(),
        ASCII_CLASS_UPPER => |ch| ch.is_ascii_uppercase(),
        _ => return itr_err_fmt!(NativeFuncError, "unsupported ascii class {}", class_id),
    })
}

fn decode_prefixed_u16<'a>(buf: &'a [u8], name: &str) -> VmrtRes<(u16, &'a [u8])> {
    if buf.len() < 2 {
        return itr_err_fmt!(
            NativeFuncError,
            "{} expects at least 2 bytes, got {}",
            name,
            buf.len()
        );
    }
    Ok((u16::from_be_bytes([buf[0], buf[1]]), &buf[2..]))
}

fn decode_prefixed_u64<'a>(buf: &'a [u8], name: &str) -> VmrtRes<(u64, &'a [u8])> {
    if buf.len() < 8 {
        return itr_err_fmt!(
            NativeFuncError,
            "{} expects at least 8 bytes, got {}",
            name,
            buf.len()
        );
    }
    Ok((u64::from_be_bytes(buf[0..8].try_into().unwrap()), &buf[8..]))
}

fn ascii_trim_range(buf: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = buf.len();
    while start < end && ascii_ws(buf[start]) {
        start += 1;
    }
    while end > start && ascii_ws(buf[end - 1]) {
        end -= 1;
    }
    &buf[start..end]
}

fn make_tuple(items: Vec<Value>) -> VmrtRes<Value> {
    Ok(Value::Tuple(TupleItem::new(items)?))
}

fn tuple_errno_bytes(errno: u16, out: Vec<u8>) -> VmrtRes<Value> {
    make_tuple(vec![Value::U16(errno), Value::Bytes(out)])
}

fn tuple_errno_u128(errno: u16, out: u128) -> VmrtRes<Value> {
    make_tuple(vec![Value::U16(errno), Value::U128(out)])
}

fn tuple_errno_map(errno: u16, map: BTreeMap<Vec<u8>, Value>) -> VmrtRes<Value> {
    make_tuple(vec![Value::U16(errno), Value::Compo(CompoItem::map(map)?)])
}

fn max_kv_map_payload_bytes(cap: &SpaceCap) -> Option<usize> {
    cap.compo_length.checked_mul(2)?.checked_mul(cap.value_size)
}

fn validate_kv_map_against_space_cap(
    cap: &SpaceCap,
    map: &BTreeMap<Vec<u8>, Value>,
) -> Option<u16> {
    if map.len() > cap.compo_length {
        return Some(ASCII_ERR_SPACE_CAP);
    }
    let max_total = max_kv_map_payload_bytes(cap);
    let mut total = 0usize;
    for (k, v) in map {
        let Value::Bytes(b) = v else {
            return Some(ASCII_ERR_SPACE_CAP);
        };
        if checked_value_output_len(cap, k.len()).is_err()
            || checked_value_output_len(cap, b.len()).is_err()
        {
            return Some(ASCII_ERR_SPACE_CAP);
        }
        let add = match k.len().checked_add(b.len()) {
            Some(x) => x,
            None => return Some(ASCII_ERR_SPACE_CAP),
        };
        total = match total.checked_add(add) {
            Some(x) => x,
            None => return Some(ASCII_ERR_SPACE_CAP),
        };
        if let Some(m) = max_total {
            if total > m {
                return Some(ASCII_ERR_SPACE_CAP);
            }
        }
    }
    None
}

fn tuple_errno_map_ok_kv(cap: &SpaceCap, map: BTreeMap<Vec<u8>, Value>) -> VmrtRes<Value> {
    if let Some(errno) = validate_kv_map_against_space_cap(cap, &map) {
        return tuple_errno_map(errno, BTreeMap::new());
    }
    let v = tuple_errno_map(ASCII_ERR_OK, map)?;
    v.check_container_cap(cap)?;
    Ok(v)
}

fn lowercase_ascii_vec_in_place(buf: &mut [u8]) {
    for ch in buf {
        *ch = ascii_to_lower(*ch);
    }
}

fn parse_quoted_ascii_value(raw: &[u8], start: usize) -> Result<(Vec<u8>, usize), u16> {
    if raw.get(start) != Some(&b'"') {
        return Err(ASCII_ERR_FORMAT);
    }
    let mut i = start + 1;
    let mut out = Vec::new();
    while i < raw.len() {
        match raw[i] {
            b'"' => return Ok((out, i + 1)),
            b'\\' => {
                if i + 1 >= raw.len() {
                    return Err(ASCII_ERR_FORMAT);
                }
                match raw[i + 1] {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    _ => return Err(ASCII_ERR_FORMAT),
                }
                i += 2;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Err(ASCII_ERR_FORMAT)
}

pub(super) fn ascii_validate_transform(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let (mode, raw) = decode_prefixed_u16(buf, "ascii_validate_transform")?;
    let class_id = (mode & 0x00ff) as u8;
    let flags = (mode >> 8) as u8;
    let trim = flags & TXT_FLAG_TRIM_ASCII_WS != 0;
    let allow_empty = flags & TXT_FLAG_ALLOW_EMPTY != 0;
    let allow_dash = flags & TXT_FLAG_ALLOW_DASH_AS_EMPTY != 0;
    let to_lower = flags & TXT_FLAG_TO_LOWER != 0;
    let to_upper = flags & TXT_FLAG_TO_UPPER != 0;

    if to_lower && to_upper {
        return itr_err_fmt!(
            NativeFuncError,
            "ascii_validate_transform lower and upper cannot both be enabled"
        );
    }

    let data = if trim { ascii_trim_range(raw) } else { raw };
    if data.is_empty() {
        return tuple_errno_bytes(
            if allow_empty {
                ASCII_ERR_OK
            } else {
                ASCII_ERR_EMPTY
            },
            vec![],
        );
    }
    if data == b"-" {
        return tuple_errno_bytes(
            if allow_dash {
                ASCII_ERR_OK
            } else {
                ASCII_ERR_FORMAT
            },
            vec![],
        );
    }

    let class_ok = class_byte_predicate(class_id)?;
    let mut out = Vec::with_capacity(data.len());
    for &ch in data {
        if !class_ok(ch) {
            return tuple_errno_bytes(ASCII_ERR_INVALID_CHAR, vec![]);
        }
        out.push(match (to_lower, to_upper) {
            (true, false) => ascii_to_lower(ch),
            (false, true) => ascii_to_upper(ch),
            _ => ch,
        });
    }

    tuple_errno_bytes(ASCII_ERR_OK, out)
}

pub(super) fn ascii_u128_dec_unit(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let (mode, raw) = decode_prefixed_u16(buf, "ascii_u128_dec_unit")?;
    let flags = (mode >> 8) as u8;
    let unit_mask = (mode & 0x00ff) as u8;
    let trim = flags & DEC_FLAG_TRIM_ASCII_WS != 0;
    let allow_empty = flags & DEC_FLAG_ALLOW_EMPTY_AS_ZERO != 0;
    let allow_dash = flags & DEC_FLAG_ALLOW_DASH_AS_ZERO != 0;
    let case_fold = flags & DEC_FLAG_SUFFIX_CASE_INSENSITIVE != 0;
    let data = if trim { ascii_trim_range(raw) } else { raw };

    if data.is_empty() {
        return tuple_errno_u128(
            if allow_empty {
                ASCII_ERR_OK
            } else {
                ASCII_ERR_FORMAT
            },
            0,
        );
    }
    if data == b"-" {
        return tuple_errno_u128(
            if allow_dash {
                ASCII_ERR_OK
            } else {
                ASCII_ERR_FORMAT
            },
            0,
        );
    }

    let mut num = 0u128;
    let mut mul = 1u128;
    let mut seen_digit = false;
    let mut in_suffix = false;

    for &raw_ch in data {
        let ch = if case_fold {
            ascii_to_lower(raw_ch)
        } else {
            raw_ch
        };
        if ch.is_ascii_digit() {
            if in_suffix {
                return tuple_errno_u128(ASCII_ERR_FORMAT, 0);
            }
            seen_digit = true;
            let digit = (ch - b'0') as u128;
            num = match num.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                Some(v) => v,
                None => return tuple_errno_u128(ASCII_ERR_OVERFLOW, 0),
            };
            continue;
        }

        in_suffix = true;
        let factor = match ch {
            b'h' if unit_mask & DEC_UNIT_H100 != 0 => 100u128,
            b'k' if unit_mask & DEC_UNIT_K1E3 != 0 => 1_000u128,
            b'm' if unit_mask & DEC_UNIT_M1E6 != 0 => 1_000_000u128,
            b'b' if unit_mask & DEC_UNIT_B1E9 != 0 => 1_000_000_000u128,
            b't' if unit_mask & DEC_UNIT_T1E12 != 0 => 1_000_000_000_000u128,
            _ => return tuple_errno_u128(ASCII_ERR_FORMAT, 0),
        };
        mul = match mul.checked_mul(factor) {
            Some(v) => v,
            None => return tuple_errno_u128(ASCII_ERR_OVERFLOW, 0),
        };
    }

    if !seen_digit {
        return tuple_errno_u128(ASCII_ERR_FORMAT, 0);
    }
    let out = match num.checked_mul(mul) {
        Some(v) => v,
        None => return tuple_errno_u128(ASCII_ERR_OVERFLOW, 0),
    };
    tuple_errno_u128(ASCII_ERR_OK, out)
}

pub(super) fn ascii_hex_lower(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    if buf.len() % 2 != 0 {
        return tuple_errno_bytes(ASCII_ERR_HEX_ODD, vec![]);
    }
    let mut out = Vec::with_capacity(buf.len());
    for &ch in buf {
        let lowered = ascii_to_lower(ch);
        if !lowered.is_ascii_hexdigit() {
            return tuple_errno_bytes(ASCII_ERR_INVALID_CHAR, vec![]);
        }
        out.push(lowered);
    }
    tuple_errno_bytes(ASCII_ERR_OK, out)
}

pub(super) fn ascii_base58_validate_or_echo(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    for &ch in buf {
        if !ascii_is_base58(ch) {
            return tuple_errno_bytes(ASCII_ERR_INVALID_CHAR, vec![]);
        }
    }
    tuple_errno_bytes(ASCII_ERR_OK, buf.to_vec())
}

pub(super) fn ascii_parse_flat_kv(height: u64, buf: &[u8]) -> VmrtRes<Value> {
    parse_flat_kv_impl(&SpaceCap::new(height), buf)
}

fn parse_flat_kv_impl(cap: &SpaceCap, buf: &[u8]) -> VmrtRes<Value> {
    let (spec, raw) = decode_prefixed_u64(buf, "ascii_parse_flat_kv")?;
    let parts = spec.to_be_bytes();
    let open = parts[0];
    let kv_sep = parts[1];
    let pair_sep = parts[2];
    let close = parts[3];
    let key_class = parts[4];
    let value_class = parts[5];
    if parts[6] != 0 {
        return itr_err_fmt!(NativeFuncError, "ascii_parse_flat_kv spec byte 6 must be 0");
    }
    let flags = parts[7];
    let key_ok = class_byte_predicate(key_class)?;
    let value_ok = class_byte_predicate(value_class)?;
    if open == 0 || kv_sep == 0 || pair_sep == 0 || close == 0 {
        return itr_err_fmt!(
            NativeFuncError,
            "ascii_parse_flat_kv delimiters must be non-zero bytes"
        );
    }
    if open == kv_sep
        || open == pair_sep
        || open == close
        || kv_sep == pair_sep
        || kv_sep == close
        || pair_sep == close
    {
        return itr_err_fmt!(
            NativeFuncError,
            "ascii_parse_flat_kv delimiters must be distinct"
        );
    }

    let allow_outer_ws = flags & KV_FLAG_ALLOW_OUTER_WS != 0;
    let allow_inner_ws = flags & KV_FLAG_ALLOW_INNER_WS != 0;
    let allow_empty_doc = flags & KV_FLAG_ALLOW_EMPTY_DOC != 0;
    let quoted_value = flags & KV_FLAG_QUOTED_VALUE != 0;
    let lower_keys = flags & KV_FLAG_LOWER_KEYS != 0;
    let lower_values = flags & KV_FLAG_LOWER_VALUES != 0;

    let mut i = 0usize;
    if allow_outer_ws {
        while i < raw.len() && ascii_ws(raw[i]) {
            i += 1;
        }
    }
    if i >= raw.len() || raw[i] != open {
        return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
    }
    i += 1;

    let mut map = BTreeMap::<Vec<u8>, Value>::new();

    loop {
        if allow_inner_ws {
            while i < raw.len() && ascii_ws(raw[i]) {
                i += 1;
            }
        }
        if i >= raw.len() {
            return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
        }
        if raw[i] == close {
            i += 1;
            if allow_outer_ws {
                while i < raw.len() && ascii_ws(raw[i]) {
                    i += 1;
                }
            }
            if i != raw.len() {
                return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
            }
            if map.is_empty() && !allow_empty_doc {
                return tuple_errno_map(ASCII_ERR_EMPTY, BTreeMap::new());
            }
            return tuple_errno_map_ok_kv(cap, map);
        }

        let key_start = i;
        while i < raw.len() && key_ok(raw[i]) {
            i += 1;
        }
        if i == key_start {
            return tuple_errno_map(ASCII_ERR_INVALID_CHAR, BTreeMap::new());
        }
        let mut key = raw[key_start..i].to_vec();
        if lower_keys {
            lowercase_ascii_vec_in_place(&mut key);
        }

        if allow_inner_ws {
            while i < raw.len() && ascii_ws(raw[i]) {
                i += 1;
            }
        }
        if i >= raw.len() || raw[i] != kv_sep {
            return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
        }
        i += 1;

        if allow_inner_ws {
            while i < raw.len() && ascii_ws(raw[i]) {
                i += 1;
            }
        }

        let mut value = if quoted_value && i < raw.len() && raw[i] == b'"' {
            match parse_quoted_ascii_value(raw, i) {
                Ok((v, next_i)) => {
                    i = next_i;
                    v
                }
                Err(errno) => return tuple_errno_map(errno, BTreeMap::new()),
            }
        } else {
            let value_start = i;
            while i < raw.len() && value_ok(raw[i]) {
                i += 1;
            }
            if i == value_start {
                return tuple_errno_map(ASCII_ERR_INVALID_CHAR, BTreeMap::new());
            }
            raw[value_start..i].to_vec()
        };
        if lower_values {
            lowercase_ascii_vec_in_place(&mut value);
        }

        if checked_value_output_len(cap, key.len()).is_err()
            || checked_value_output_len(cap, value.len()).is_err()
        {
            return tuple_errno_map(ASCII_ERR_SPACE_CAP, BTreeMap::new());
        }
        if map.contains_key(&key) {
            return tuple_errno_map(ASCII_ERR_DUP_KEY, BTreeMap::new());
        }
        if map.len() >= cap.compo_length {
            return tuple_errno_map(ASCII_ERR_SPACE_CAP, BTreeMap::new());
        }
        map.insert(key, Value::Bytes(value));

        if allow_inner_ws {
            while i < raw.len() && ascii_ws(raw[i]) {
                i += 1;
            }
        }
        if i >= raw.len() {
            return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
        }
        if raw[i] == pair_sep {
            i += 1;
            continue;
        }
        if raw[i] == close {
            i += 1;
            if allow_outer_ws {
                while i < raw.len() && ascii_ws(raw[i]) {
                    i += 1;
                }
            }
            if i != raw.len() {
                return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
            }
            return tuple_errno_map_ok_kv(cap, map);
        }
        return tuple_errno_map(ASCII_ERR_FORMAT, BTreeMap::new());
    }
}
