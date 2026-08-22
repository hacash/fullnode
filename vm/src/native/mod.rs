use blake2::digest::consts::U32;
use blake2::{Blake2b, Blake2s256, Digest as BlakeDigest};
use field::*;
use ripemd::Ripemd160;
use sha2::Sha256;
use sha3::Sha3_256;
use tiny_keccak::{Hasher, Keccak};

use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::*;

mod ascii;
mod call;
mod intent;
use ascii::*;
pub use call::{call_ntctl, call_ntenv, call_ntfunc};

// Native enums and their metadata tables live in the codec-safe
// `rt::native_catalog` (shared with the fitsh decompiler); the `use crate::rt::*`
// glob above brings them into this module's scope. The execution dispatch for
// `NativeFunc` is generated here where the implementation functions live.

macro_rules! native_dispatch_method {
    (func, $EnumName:ident, $ErrCode:ident, $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $_tar_uint_tys:expr )+) => {
        pub fn call(height: u64, idx: u8, v: &[u8]) -> VmrtRes<(Value, i64)> {
            let cty = Self::try_from_u8(idx)?;
            match cty {
                $(
                    Self::$name => $name(height, v).map(|r| {
                        assert_eq!($rty, r.ty());
                        (r, $gas)
                    }),
                )+
                _ => unreachable!(),
            }
        }
    };
}

use ValueTy::*;

impl NativeFunc {
    native_dispatch_method!(func, NativeFunc, NativeFuncError,
        hac_to_mei         = 31,   1,        6,    U64,        &[]
        hac_to_zhu         = 32,   1,        6,    U128,       &[]
        u64_to_fold64      = 33,   1,        8,    Bytes,      &[]
        fold64_to_u64      = 34,   1,        8,    U64,        &[]
        pack_asset         = 37,   2,        8,    Bytes,      &[U64, U64]
        mei_to_hac         = 35,   1,        6,    Bytes,      &[]
        zhu_to_hac         = 36,   1,        6,    Bytes,      &[]
        address_ptr        = 41,   1,        4,    U8,         &[]
        sha2               = 101, 1,       32,    Bytes,      &[]
        sha3               = 102, 1,       32,    Bytes,      &[]
        ripemd160          = 103, 1,       20,    Bytes,      &[]
        verify_signature   = 104, 3,       96,    Bool,       &[]
        keccak256          = 105, 1,       32,    Bytes,      &[]
        blake2s256         = 106, 1,       32,    Bytes,      &[]
        blake2b256         = 107, 1,       32,    Bytes,      &[]
        ascii_parse_flat_kv = 120, 2,      64,    Tuple,      &[]
        ascii_validate_transform = 121, 2, 24,    Tuple,      &[]
        ascii_u128_dec_unit = 122, 2,      24,    Tuple,      &[]
        ascii_hex_lower    = 123, 1,       20,    Tuple,      &[]
        ascii_base58_validate_or_echo = 124, 1, 20, Tuple,      &[]
    );
}

fn digest_value<D: sha2::Digest>(buf: &[u8]) -> Value {
    Value::bytes(D::digest(buf).to_vec())
}

fn sha2(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Sha256>(buf))
}

fn sha3(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Sha3_256>(buf))
}

fn keccak256(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let mut keccak = Keccak::v256();
    let mut out = [0u8; 32];
    keccak.update(buf);
    keccak.finalize(&mut out);
    Ok(Value::bytes(out.to_vec()))
}

fn blake2s256(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Blake2s256>(buf))
}

fn blake2b256(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Blake2b<U32>>(buf))
}

fn ripemd160(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    Ok(Value::bytes(Ripemd160::digest(buf).to_vec()))
}

fn decode_exact<T: Decode>(buf: &[u8], label: &str) -> VmrtRes<T> {
    let (value, used) = T::decode(buf).map_ire(NativeFuncError)?;
    if used != buf.len() {
        return itr_err_fmt!(
            NativeFuncError,
            "call {} parse length mismatch: used {}, total {}",
            label,
            used,
            buf.len()
        );
    }
    Ok(value)
}

fn mei_to_hac(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let num = buf_to_uint(buf)?.extract_u128()?;
    if num > u64::MAX as u128 {
        return itr_err_fmt!(NativeFuncError, "call mei_to_hac amount too large");
    }
    Ok(Value::Bytes(Amount::mei(num as u64).encode()))
}

fn hac_to_mei(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_to_mei")?;
    let mei = hacash
        .to_mei_u64()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U64(mei))
}

fn hac_to_zhu(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_to_zhu")?;
    let zhu = hacash
        .to_zhu_u128()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U128(zhu))
}

fn zhu_to_hac(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let num = buf_to_uint(buf)?.extract_u128()?;
    if num > u64::MAX as u128 {
        return itr_err_fmt!(NativeFuncError, "call zhu_to_hac overflow");
    }
    Ok(Value::Bytes(Amount::zhu(num as u64).encode()))
}

fn pack_asset(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    if buf.len() != 16 {
        return itr_err_fmt!(
            NativeFuncError,
            "call pack_asset expects 16 bytes (u64 + u64), got {}",
            buf.len()
        );
    }
    let serial = u64::from_be_bytes(buf[0..8].try_into().unwrap());
    let amount = u64::from_be_bytes(buf[8..16].try_into().unwrap());
    let asset = AssetAmt {
        serial: Fold64::from(serial).map_ire(NativeFuncError)?,
        amount: Fold64::from(amount).map_ire(NativeFuncError)?,
    }
    .checked()
    .map_ire(NativeFuncError)?;
    Ok(Value::Bytes(asset.encode()))
}

fn u64_to_fold64(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let num = buf_to_uint(buf)?.extract_u128()?;
    if num > u64::MAX as u128 {
        return itr_err_fmt!(NativeFuncError, "call u64_to_fold64 overflow");
    }
    let fold = Fold64::from(num as u64).map_ire(NativeFuncError)?;
    Ok(Value::Bytes(fold.encode()))
}

fn fold64_to_u64(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let fold: Fold64 = decode_exact(buf, "fold64_to_u64")?;
    Ok(Value::U64(fold.uint()))
}

fn address_ptr(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    if buf.len() != 1 {
        return itr_err_fmt!(NativeFuncError, "param error");
    }
    const DVN: u8 = ADDR_REF_MARKER_BASE;
    let idx = buf[0];
    let max = u8::MAX - DVN;
    if idx > max {
        return itr_err_fmt!(
            NativeFuncError,
            "address_ptr param max {} but got {}",
            max,
            idx
        );
    }
    Ok(Value::U8(idx + DVN))
}

fn verify_signature(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let mut r = Reader::new(buf);
    let hash: Hash = r.read().map_ire(NativeFuncError)?;
    let addr: field::Address = r.read().map_ire(NativeFuncError)?;
    let sign: Sign = r.read().map_ire(NativeFuncError)?;
    if r.used() != buf.len() {
        return itr_err_fmt!(
            NativeFuncError,
            "call verify_signature parse length mismatch: used {}, total {}",
            r.used(),
            buf.len()
        );
    }
    let ok = sys::Account::verify_signature(&hash.0, &sign.publickey, &sign.signature)
        && sys::Account::get_address_by_public_key(sign.publickey) == *addr.as_array();
    Ok(Value::Bool(ok))
}

native_func_env_define! { ctl, NativeCtl, NativeCtlError,
    defer              = 1,     1,        8,    Nil,        &[]
    intent_new         = 21,    1,       32,    Handle,     &[]
    intent_use         = 22,    1,        8,    Nil,        &[]
    intent_pop         = 23,    0,        8,    Nil,        &[]
    intent_is_own_handle      = 24,    1,       10,    Bool,       &[]
    intent_kind        = 25,    0,        8,    Bytes,      &[]
    intent_kind_is     = 26,    1,        8,    Bool,       &[]
    intent_destroy     = 27,    0,       10,    Nil,        &[]
    intent_destroy_if_empty = 28, 0,     10,    Bool,       &[]
    intent_clear       = 29,    0,       10,    Nil,        &[]
    intent_len         = 30,    0,       10,    U64,        &[]
    intent_has         = 31,    1,       10,    Bool,       &[]
    intent_keys        = 32,    0,       16,    Compo,      &[]
    intent_keys_page   = 33,    2,       16,    Tuple,      &[]
    intent_keys_after   = 34,    2,       16,    Tuple,      &[]
    intent_get         = 35,    1,       10,    Nil,        &[]
    intent_get_or      = 36,    2,       12,    Nil,        &[]
    intent_require     = 37,    1,       10,    Nil,        &[]
    intent_require_eq  = 38,    2,       10,    Nil,        &[]
    intent_require_absent = 39, 1,       10,    Nil,        &[]
    intent_require_many = 40,   1,       16,    Compo,      &[]
    intent_require_map = 41,    1,       16,    Compo,      &[]
    intent_has_all     = 42,    1,       12,    Bool,       &[]
    intent_has_any     = 43,    1,       12,    Bool,       &[]
    intent_put         = 44,    2,       24,    Nil,        &[]
    intent_put_if_absent = 45,  2,       24,    Bool,       &[]
    intent_put_if_absent_or_match = 46, 2,  24, Bool,       &[]
    intent_put_flat_kv   = 47,    1,       32,    Nil,        &[]
    intent_replace     = 48,    2,       14,    Nil,        &[]
    intent_replace_if  = 49,    3,       16,    Bool,       &[]
    intent_rename        = 50,    2,       14,    Nil,        &[]
    intent_take        = 51,    1,       12,    Nil,        &[]
    intent_take_or     = 52,    2,       14,    Nil,        &[]
    intent_take_if     = 53,    2,       14,    Tuple,      &[]
    intent_take_many   = 54,    1,       16,    Compo,      &[]
    intent_take_map    = 55,    1,       16,    Compo,      &[]
    intent_consume     = 56,    1,       14,    Nil,        &[]
    intent_consume_many = 57,   1,       16,    Compo,      &[]
    intent_del         = 58,    1,       10,    Nil,        &[]
    intent_del_if      = 59,    2,       14,    Bool,       &[]
    intent_del_many    = 60,    1,       12,    U64,        &[]
    intent_append      = 61,    2,       14,    U64,        &[]
    intent_inc         = 62,    2,       14,    U64,        &[]
    intent_add         = 63,    2,       14,    U64,        &[]
    intent_sub         = 64,    2,       14,    U64,        &[]
}
