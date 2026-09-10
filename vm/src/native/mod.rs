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

mod argv;
mod ascii;
mod call;
mod intent;
mod patches;
use argv::{func_address, func_argv, func_bytes, func_u8, func_u64, func_u128};
use ascii::*;
#[allow(unused_imports)] // interpreter NTFUNC Packed branch (wired by parent)
pub use call::call_ntfunc_packed;
pub use call::{call_ntctl, call_ntenv, call_ntfunc};
use patches::patches;

pub use crate::rt::{NativeArgvPack, NativeCtl, NativeEnv, NativeFnEnv, NativeFunc};

fn finish_ntfunc(cty: NativeFunc, r: Value) -> VmrtRes<(Value, i64)> {
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

impl NativeFunc {
    pub fn call(env: NativeFnEnv<'_>, idx: u8, v: &[u8]) -> VmrtRes<(Value, i64)> {
        let cty = Self::try_from_u8(idx)?;
        if cty.argv_pack_of() != NativeArgvPack::Concat {
            return itr_err_fmt!(
                NativeFuncError,
                "native func {} requires packed argv",
                cty.name()
            );
        }
        let r = match cty {
            Self::sha2 => sha2(env, v)?,
            Self::sha3 => sha3(env, v)?,
            Self::ripemd160 => ripemd160(env, v)?,
            Self::keccak256 => keccak256(env, v)?,
            Self::blake2s256 => blake2s256(env, v)?,
            Self::blake2b256 => blake2b256(env, v)?,
            Self::Null
            | Self::hac_to_mei
            | Self::hac_to_zhu
            | Self::u64_to_fold64
            | Self::fold64_to_u64
            | Self::mei_to_hac
            | Self::zhu_to_hac
            | Self::address_ptr
            | Self::pack_asset
            | Self::patches
            | Self::verify_signature
            | Self::ascii_parse_flat_kv
            | Self::ascii_validate_transform
            | Self::ascii_u128_dec_unit
            | Self::ascii_hex_lower
            | Self::ascii_base58_validate_or_echo => {
                unreachable!("catalog argv_pack_of Concat")
            }
        };
        finish_ntfunc(cty, r)
    }

    pub fn call_packed(env: NativeFnEnv<'_>, idx: u8, argv: Value) -> VmrtRes<(Value, i64)> {
        let cty = Self::try_from_u8(idx)?;
        if cty.argv_pack_of() != NativeArgvPack::Packed {
            return itr_err_fmt!(
                NativeFuncError,
                "native func {} requires concat argv",
                cty.name()
            );
        }
        let r = match cty {
            Self::hac_to_mei => hac_to_mei(env, argv)?,
            Self::hac_to_zhu => hac_to_zhu(env, argv)?,
            Self::u64_to_fold64 => u64_to_fold64(env, argv)?,
            Self::fold64_to_u64 => fold64_to_u64(env, argv)?,
            Self::mei_to_hac => mei_to_hac(env, argv)?,
            Self::zhu_to_hac => zhu_to_hac(env, argv)?,
            Self::address_ptr => address_ptr(env, argv)?,
            Self::pack_asset => pack_asset(env, argv)?,
            Self::patches => patches(env, argv)?,
            Self::verify_signature => verify_signature(env, argv)?,
            Self::ascii_parse_flat_kv => ascii_parse_flat_kv(env, argv)?,
            Self::ascii_validate_transform => ascii_validate_transform(env, argv)?,
            Self::ascii_u128_dec_unit => ascii_u128_dec_unit(env, argv)?,
            Self::ascii_hex_lower => ascii_hex_lower(env, argv)?,
            Self::ascii_base58_validate_or_echo => ascii_base58_validate_or_echo(env, argv)?,
            Self::sha2
            | Self::sha3
            | Self::ripemd160
            | Self::keccak256
            | Self::blake2s256
            | Self::blake2b256
            | Self::Null => unreachable!("catalog argv_pack_of Packed"),
        };
        finish_ntfunc(cty, r)
    }
}

fn digest_value<D: sha2::Digest>(buf: &[u8]) -> Value {
    Value::bytes(D::digest(buf).to_vec())
}

fn sha2(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Sha256>(buf))
}

fn sha3(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Sha3_256>(buf))
}

fn keccak256(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let mut keccak = Keccak::v256();
    let mut out = [0u8; 32];
    keccak.update(buf);
    keccak.finalize(&mut out);
    Ok(Value::bytes(out.to_vec()))
}

fn blake2s256(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Blake2s256>(buf))
}

fn blake2b256(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    Ok(digest_value::<Blake2b<U32>>(buf))
}

fn ripemd160(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
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

fn mei_to_hac(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::mei_to_hac;
    let num = func_u64(&packed_arg(argv, cty)?, cty, "amount")?;
    Ok(Value::Bytes(Amount::mei(num).encode()))
}

fn hac_to_mei(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::hac_to_mei;
    let buf = func_bytes(&packed_arg(argv, cty)?, cty, "amount")?;
    let hacash: Amount = decode_exact(&buf, cty.name())?;
    let mei = hacash
        .to_mei_u64()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U64(mei))
}

fn hac_to_zhu(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::hac_to_zhu;
    let buf = func_bytes(&packed_arg(argv, cty)?, cty, "amount")?;
    let hacash: Amount = decode_exact(&buf, cty.name())?;
    let zhu = hacash
        .to_zhu_u128()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U128(zhu))
}

fn zhu_to_hac(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::zhu_to_hac;
    let num = func_u128(&packed_arg(argv, cty)?, cty, "amount")?;
    Ok(Value::Bytes(Amount::coin_u128(num, UNIT_ZHU).encode()))
}

fn pack_asset(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
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

fn u64_to_fold64(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::u64_to_fold64;
    let num = func_u64(&packed_arg(argv, cty)?, cty, "value")?;
    let fold = Fold64::from(num).map_ire(NativeFuncError)?;
    Ok(Value::Bytes(fold.encode()))
}

fn fold64_to_u64(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::fold64_to_u64;
    let buf = func_bytes(&packed_arg(argv, cty)?, cty, "fold")?;
    let fold: Fold64 = decode_exact(&buf, cty.name())?;
    Ok(Value::U64(fold.uint()))
}

fn address_ptr(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
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

fn verify_signature(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
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
