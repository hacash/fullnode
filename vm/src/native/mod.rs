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

mod amount;
mod argv;
mod ascii;
mod call;
mod intent;
mod patches;
pub(crate) use amount::{
    hac_is_exact_mei, hac_is_exact_unit, hac_is_exact_zhu, hac_to_mei_checked, hac_to_unit,
    hac_to_unit_checked, hac_to_zhu_checked, unit_to_hac,
};
use argv::*;
pub(crate) use ascii::*;
#[allow(unused_imports)] // interpreter NTFUNC Packed branch (wired by parent)
pub use call::call_ntfunc_packed;
pub use call::{call_ntctl, call_ntenv, call_ntfunc};
pub(crate) use patches::patches;

pub use crate::rt::{NativeCtl, NativeEnv, NativeFnEnv, NativeFunc};

pub(crate) fn finish_ntfunc(cty: NativeFunc, r: Value) -> VmrtRes<(Value, i64)> {
    if cty.rty_of() != r.ty() {
        return itr_err_fmt!(
            NativeFuncError,
            "native func {} return type mismatch: catalog {:?}, got {:?}",
            cty.name(),
            cty.rty_of(),
            r.ty()
        );
    }
    Ok((r, cty.gas_of()))
}

fn digest_value<D: sha2::Digest>(buf: &[u8]) -> Value {
    Value::bytes(D::digest(buf).to_vec())
}

pub(crate) fn sha2(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Sha256>(buf))
}

pub(crate) fn sha3(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Sha3_256>(buf))
}

pub(crate) fn keccak256(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let mut keccak = Keccak::v256();
    let mut out = [0u8; 32];
    keccak.update(buf);
    keccak.finalize(&mut out);
    Ok(Value::bytes(out.to_vec()))
}

pub(crate) fn blake2s256(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Blake2s256>(buf))
}

pub(crate) fn blake2b256(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Blake2b<U32>>(buf))
}

pub(crate) fn ripemd160(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
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

fn packed_arg(argv: Value, cty: NativeFunc) -> VmrtRes<Value> {
    debug_assert_eq!(cty.argv_len_of(), 1);
    let mut args = func_argv(argv, cty)?;
    Ok(args.pop().expect("catalog arity 1 is Raw"))
}

pub(crate) fn mei_to_hac(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::mei_to_hac;
    let num = func_u64(&packed_arg(argv, cty)?, cty, "amount")?;
    Ok(Value::Bytes(Amount::mei(num).encode()))
}

pub(crate) fn hac_to_mei(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_to_mei")?;
    let mei = hacash
        .to_mei_u64()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U64(mei))
}

pub(crate) fn hac_to_zhu(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_to_zhu")?;
    let zhu = hacash
        .to_zhu_u128()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U128(zhu))
}

pub(crate) fn zhu_to_hac(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::zhu_to_hac;
    let num = func_u128(&packed_arg(argv, cty)?, cty, "amount")?;
    Ok(Value::Bytes(Amount::coin_u128(num, UNIT_ZHU).encode()))
}

pub(crate) fn pack_asset(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::pack_asset;
    let args = func_argv(argv, cty)?;
    let serial = func_u64(&args[0], cty, "serial")?;
    let amount = func_u64(&args[1], cty, "amount")?;
    let asset = AssetAmt {
        serial: Fold64::from(serial).map_ire(NativeFuncError)?,
        amount: Fold64::from(amount).map_ire(NativeFuncError)?,
    }
    .checked()
    .map_ire(NativeFuncError)?;
    Ok(Value::Bytes(asset.encode()))
}

pub(crate) fn u64_to_fold64(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::u64_to_fold64;
    let num = func_u64(&packed_arg(argv, cty)?, cty, "value")?;
    let fold = Fold64::from(num).map_ire(NativeFuncError)?;
    Ok(Value::Bytes(fold.encode()))
}

pub(crate) fn fold64_to_u64(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let fold: Fold64 = decode_exact(buf, "fold64_to_u64")?;
    Ok(Value::U64(fold.uint()))
}

pub(crate) fn address_ptr(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::address_ptr;
    const DVN: u8 = ADDR_REF_MARKER_BASE;
    let idx = func_u8(&packed_arg(argv, cty)?, cty, "index")?;
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

pub(crate) fn verify_signature(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::verify_signature;
    let args = func_argv(argv, cty)?;
    let hash_buf = func_bytes(&args[0], cty, "hash")?;
    let addr = func_address(&args[1], cty, "address")?;
    let sign_buf = func_bytes(&args[2], cty, "sign")?;
    let hash: Hash = decode_exact(&hash_buf, "verify_signature hash")?;
    let sign: Sign = decode_exact(&sign_buf, "verify_signature sign")?;
    let ok = sys::Account::verify_signature(&hash.0, &sign.publickey, &sign.signature)
        && sys::Account::get_address_by_public_key(sign.publickey.into_array()) == *addr.as_array();
    Ok(Value::Bool(ok))
}
