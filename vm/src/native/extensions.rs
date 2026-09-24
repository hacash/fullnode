use field::{Address, AssetAmt, Decode, Encode, Fold64, UNIT_ZHU};
use sha2::{Digest, Sha256};

use crate::rt::{ItrErr, ItrErrCode::NativeFuncError, MapItrErr, NativeFnEnv, NativeFunc, VmrtRes};
use crate::value::{TupleItem, Value};

use super::{func_argv, func_bytes, func_list, func_u64};

const SCAN_COUNT_MAX: u64 = 64;
const CAT_PARTS_MAX: usize = 16;
const CAT_BYTES_MAX: usize = 4096;

fn args(argv: Value, cty: NativeFunc) -> VmrtRes<Vec<Value>> {
    func_argv(argv, cty)
}

fn read<'a>(buf: &'a [u8], offset: u64, width: usize, name: &str) -> VmrtRes<&'a [u8]> {
    let start = usize::try_from(offset).map_err(|_| {
        crate::rt::ItrErr::new(NativeFuncError, &format!("{} offset out of range", name))
    })?;
    let end = start.checked_add(width).ok_or_else(|| {
        crate::rt::ItrErr::new(NativeFuncError, &format!("{} range overflow", name))
    })?;
    buf.get(start..end).ok_or_else(|| {
        crate::rt::ItrErr::new(NativeFuncError, &format!("{} range out of bounds", name))
    })
}

pub(crate) fn buf_u16(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::buf_u16;
    let a = args(argv, cty)?;
    let b = func_bytes(&a[0], cty, "buf")?;
    let off = func_u64(&a[1], cty, "offset")?;
    Ok(Value::U16(u16::from_be_bytes(read(&b, off, 2, cty.name())?.try_into().unwrap())))
}

pub(crate) fn buf_u32(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::buf_u32;
    let a = args(argv, cty)?;
    let b = func_bytes(&a[0], cty, "buf")?;
    let off = func_u64(&a[1], cty, "offset")?;
    Ok(Value::U32(u32::from_be_bytes(read(&b, off, 4, cty.name())?.try_into().unwrap())))
}

pub(crate) fn buf_u64(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::buf_u64;
    let a = args(argv, cty)?;
    let b = func_bytes(&a[0], cty, "buf")?;
    let off = func_u64(&a[1], cty, "offset")?;
    Ok(Value::U64(u64::from_be_bytes(read(&b, off, 8, cty.name())?.try_into().unwrap())))
}

pub(crate) fn buf_address(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::buf_address;
    let a = args(argv, cty)?;
    let b = func_bytes(&a[0], cty, "buf")?;
    let off = func_u64(&a[1], cty, "offset")?;
    let raw: [u8; Address::SIZE] = read(&b, off, Address::SIZE, cty.name())?.try_into().unwrap();
    let (address, _) = Address::decode(&raw).map_ire(NativeFuncError)?;
    Ok(Value::Address(address))
}

pub(crate) fn asset_meta_fields(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::asset_meta_fields;
    let raw = func_bytes(&args(argv, cty)?[0], cty, "metadata")?;
    if raw.len() != 30 {
        return itr_err_fmt!(NativeFuncError, "asset metadata length {} is not 30", raw.len());
    }
    let (issuer, _) = Address::decode(&raw[9..30]).map_ire(NativeFuncError)?;
    Ok(Value::Tuple(TupleItem::new(vec![
        Value::U8(raw[0]),
        Value::U64(u64::from_be_bytes(raw[1..9].try_into().unwrap())),
        Value::Address(issuer),
    ])?))
}

pub(crate) fn asset_meta_require(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::asset_meta_require;
    let a = args(argv, cty)?;
    let raw = func_bytes(&a[0], cty, "metadata")?;
    let decimal = a[1].extract_u8().map_err(|_| crate::rt::ItrErr::new(NativeFuncError, "decimal must fit u8"))?;
    let supply = func_u64(&a[2], cty, "supply")?;
    let issuer = a[3].extract_address().map_err(|_| crate::rt::ItrErr::new(NativeFuncError, "issuer must be address"))?;
    if raw.len() != 30 {
        return itr_err_fmt!(NativeFuncError, "asset metadata length {} is not 30", raw.len());
    }
    let (actual_issuer, _) = Address::decode(&raw[9..30]).map_ire(NativeFuncError)?;
    Ok(Value::Bool(raw[0] == decimal
        && u64::from_be_bytes(raw[1..9].try_into().unwrap()) == supply
        && actual_issuer == issuer))
}

pub(crate) fn pack_fungible_amount(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::pack_fungible_amount;
    let a = args(argv, cty)?;
    let serial = func_u64(&a[0], cty, "serial")?;
    let amount = func_u64(&a[1], cty, "amount")?;
    if serial == 0 || (4..=10).contains(&serial) {
        return itr_err_fmt!(
            NativeFuncError,
            "fungible serial {serial} is not hac, sat, hacd, or an asset above 10"
        );
    }
    let packed = match serial {
        1 => field::Amount::coin_u128(amount as u128, UNIT_ZHU).encode(),
        2 => amount.to_be_bytes().to_vec(),
        3 => amount.to_be_bytes().to_vec(),
        _ => AssetAmt {
            serial: Fold64::from(serial).map_ire(NativeFuncError)?,
            amount: Fold64::from(amount).map_ire(NativeFuncError)?,
        }.checked().map_ire(NativeFuncError)?.encode(),
    };
    Ok(Value::bytes(packed))
}

pub(crate) fn sha2_prefix_u64(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let digest = Sha256::digest(buf);
    Ok(Value::U64(u64::from_be_bytes(digest[..8].try_into().unwrap())))
}

pub(crate) fn sha2_prefix_u160(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let digest = Sha256::digest(buf);
    Ok(Value::bytes(digest[..20].to_vec()))
}

pub(crate) fn fee_add_bps_ceil(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::fee_add_bps_ceil;
    let a = args(argv, cty)?;
    let amount = func_u64(&a[0], cty, "amount")?;
    let bps = func_u64(&a[1], cty, "bps")?;
    let fee = ((amount as u128 * bps as u128) / 10_000)
        + u128::from((amount as u128 * bps as u128) % 10_000 != 0);
    let repay = (amount as u128).checked_add(fee)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| crate::rt::ItrErr::new(NativeFuncError, "repayment exceeds u64"))?;
    Ok(Value::U64(repay))
}

/// First `u32` big-endian key on a fixed stride. `(offset, found)`.
/// A count above the cap, a stride below 4, or a window that does not fit
/// the buffer is a fault: those are not reported as "not found".
pub(crate) fn buf_scan_u32(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::buf_scan_u32;
    let a = args(argv, cty)?;
    let buf = func_bytes(&a[0], cty, "buf")?;
    let start = func_u64(&a[1], cty, "start")?;
    let count = func_u64(&a[2], cty, "count")?;
    let stride = func_u64(&a[3], cty, "stride")?;
    let key = func_u64(&a[4], cty, "key")?;
    if key > u32::MAX as u64 {
        return itr_err_fmt!(NativeFuncError, "buf_scan_u32 key does not fit u32");
    }
    if stride < 4 {
        return itr_err_fmt!(NativeFuncError, "buf_scan_u32 stride {stride} is below 4");
    }
    if count > SCAN_COUNT_MAX {
        return itr_err_fmt!(
            NativeFuncError,
            "buf_scan_u32 count {count} exceeds {SCAN_COUNT_MAX}"
        );
    }
    let found = if count == 0 {
        None
    } else {
        let start_usz = usize::try_from(start).map_err(|_| {
            crate::rt::ItrErr::new(NativeFuncError, "buf_scan_u32 start out of range")
        })?;
        let stride_usz = usize::try_from(stride).map_err(|_| {
            crate::rt::ItrErr::new(NativeFuncError, "buf_scan_u32 stride out of range")
        })?;
        let span = (count as usize).checked_mul(stride_usz).ok_or_else(|| {
            crate::rt::ItrErr::new(NativeFuncError, "buf_scan_u32 window overflow")
        })?;
        let end = start_usz.checked_add(span).ok_or_else(|| {
            crate::rt::ItrErr::new(NativeFuncError, "buf_scan_u32 window overflow")
        })?;
        if end > buf.len() {
            return itr_err_fmt!(NativeFuncError, "buf_scan_u32 window exceeds buffer");
        }
        let needle = (key as u32).to_be_bytes();
        let mut hit = None;
        for i in 0..count as usize {
            let at = start_usz + i * stride_usz;
            if buf[at..at + 4] == needle {
                hit = Some(start + i as u64 * stride);
                break;
            }
        }
        hit
    };
    let (offset, ok) = match found {
        Some(at) => (at, true),
        None => (0, false),
    };
    Ok(Value::Tuple(TupleItem::new(vec![Value::U64(offset), Value::Bool(ok)])?))
}

/// SHA-256 prefix of a list of byte strings, concatenated in order.
/// Only `Bytes` parts are accepted. Part count and total length are capped.
pub(crate) fn sha2_prefix_u64_cat(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::sha2_prefix_u64_cat;
    let parts = func_list(argv, cty)?;
    if parts.len() > CAT_PARTS_MAX {
        return itr_err_fmt!(
            NativeFuncError,
            "sha2_prefix_u64_cat part count {} exceeds {CAT_PARTS_MAX}",
            parts.len()
        );
    }
    let mut buf = Vec::new();
    for part in &parts {
        let bytes = func_bytes(part, cty, "part")?;
        let next = buf.len().saturating_add(bytes.len());
        if next > CAT_BYTES_MAX {
            return itr_err_fmt!(
                NativeFuncError,
                "sha2_prefix_u64_cat length {next} exceeds {CAT_BYTES_MAX}"
            );
        }
        buf.extend_from_slice(&bytes);
    }
    let digest = Sha256::digest(&buf);
    Ok(Value::U64(u64::from_be_bytes(digest[..8].try_into().unwrap())))
}
