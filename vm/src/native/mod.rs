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

mod addr;
mod amount;
mod argv;
mod ascii;
mod call;
mod crypto;
mod extensions;
mod hacd;
mod intent;
mod memory;
mod patches;
pub(crate) use addr::{
    address_version, check_addr_set, is_contract, is_privkey, is_privkey_not_unknown,
    is_privkey_unknown, is_scriptmh,
};
pub(crate) use amount::{
    hac_is_exact_mei, hac_is_exact_unit, hac_is_exact_zhu, hac_to_mei_checked, hac_to_unit,
    hac_to_unit_checked, hac_to_zhu_checked, hac_zhu_tail, unit_to_hac,
};
use argv::*;
pub(crate) use ascii::*;
#[allow(unused_imports)] // interpreter NTFUNC Packed branch (wired by parent)
pub use call::call_ntfunc_packed;
pub use call::{call_ntctl, call_ntenv, call_ntfunc};
pub(crate) use crypto::{bitmap_find, merkle_multi_root, p256_verify};
pub(crate) use extensions::*;
pub(crate) use hacd::*;
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

pub(crate) fn finish_ntfunc_with_extra(
    cty: NativeFunc,
    r: Value,
    extra_gas: i64,
) -> VmrtRes<(Value, i64)> {
    let (r, gas) = finish_ntfunc(cty, r)?;
    Ok((r, gas.saturating_add(extra_gas)))
}

pub(crate) fn ntfunc_dynamic_gas(cty: NativeFunc, argv: &Value) -> i64 {
    match cty {
        NativeFunc::merkle_root => {
            let Ok(args) = func_argv(argv.clone(), cty) else {
                return 0;
            };
            let Ok(hash_kind) = func_u8(&args[0], cty, "hash_kind") else {
                return 0;
            };
            let Ok(siblings) = func_bytes(&args[3], cty, "siblings") else {
                return 0;
            };
            if siblings.len() > 32 * 32 || siblings.len() % 32 != 0 {
                return 0;
            }
            let Some(hash_gas) = hash_kind_gas(hash_kind) else {
                return 0;
            };
            (siblings.len() as i64 / 32).saturating_mul(hash_gas)
        }
        NativeFunc::merkle_multi_root => {
            let Ok(args) = func_argv(argv.clone(), cty) else {
                return 0;
            };
            let Ok(hash_kind) = func_u8(&args[0], cty, "hash_kind") else {
                return 0;
            };
            let (Ok(leaves), Ok(proof)) = (
                func_bytes(&args[1], cty, "leaves"),
                func_bytes(&args[2], cty, "proof"),
            ) else {
                return 0;
            };
            if leaves.is_empty()
                || leaves.len() % 32 != 0
                || proof.len() % 32 != 0
                || leaves.len() / 32 + proof.len() / 32 > 32
            {
                return 0;
            }
            let Some(hash_gas) = hash_kind_gas(hash_kind) else {
                return 0;
            };
            ((leaves.len() + proof.len()) as i64 / 32 - 1).saturating_mul(hash_gas)
        }
        NativeFunc::bitmap_find => {
            let Ok(args) = func_argv(argv.clone(), cty) else {
                return 0;
            };
            let (Ok(bitmap), Ok(start), Ok(end)) = (
                func_bytes(&args[0], cty, "bitmap"),
                func_u64(&args[1], cty, "start"),
                func_u64(&args[2], cty, "end"),
            ) else {
                return 0;
            };
            let Some(bit_len) = u64::try_from(bitmap.len())
                .ok()
                .and_then(|len| len.checked_mul(8))
            else {
                return 0;
            };
            if start >= end || end > bit_len {
                return 0;
            }
            let bytes = ((end - 1) / 8 - start / 8 + 1) as i64;
            bytes.saturating_mul(2)
        }
        _ => 0,
    }
}

fn hash_kind_gas(hash_kind: u8) -> Option<i64> {
    match hash_kind {
        0 => Some(16),
        1 => Some(20),
        _ => None,
    }
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

pub(crate) fn merkle_root(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::merkle_root;
    let args = func_argv(argv, cty)?;
    let hash_kind = func_u8(&args[0], cty, "hash_kind")?;
    let pair_mode = func_u8(&args[1], cty, "pair_mode")?;
    let leaf = func_bytes(&args[2], cty, "leaf")?;
    let siblings = func_bytes(&args[3], cty, "siblings")?;
    let path = func_u64(&args[4], cty, "path")?;
    if hash_kind > 1 {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_root invalid hash_kind {}",
            hash_kind
        );
    }
    if pair_mode > 1 {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_root invalid pair_mode {}",
            pair_mode
        );
    }
    if leaf.len() != 32 {
        return itr_err_fmt!(NativeFuncError, "merkle_root leaf must be 32 bytes");
    }
    if siblings.len() % 32 != 0 {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_root siblings length must be a multiple of 32"
        );
    }
    let depth = siblings.len() / 32;
    if depth > 32 {
        return itr_err_fmt!(NativeFuncError, "merkle_root depth exceeds 32");
    }
    if (pair_mode == 0 && (depth < 64 && path >> depth != 0)) || (pair_mode == 1 && path != 0) {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_root path has bits outside the proof"
        );
    }

    let mut node: [u8; 32] = leaf.try_into().expect("length checked");
    let mut input = [0u8; 64];
    for (i, sibling) in siblings.chunks_exact(32).enumerate() {
        let mut other = [0u8; 32];
        other.copy_from_slice(sibling);
        let (left, right) = if pair_mode == 1 {
            if node <= other {
                (node, other)
            } else {
                (other, node)
            }
        } else if path & (1u64 << i) == 0 {
            (node, other)
        } else {
            (other, node)
        };
        input[..32].copy_from_slice(&left);
        input[32..].copy_from_slice(&right);
        node = if hash_kind == 0 {
            Sha256::digest(input).into()
        } else {
            let mut keccak = Keccak::v256();
            let mut out = [0u8; 32];
            keccak.update(&input);
            keccak.finalize(&mut out);
            out
        };
    }
    Ok(Value::bytes(node.to_vec()))
}

pub(crate) fn secp256k1_recover(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::secp256k1_recover;
    let args = func_argv(argv, cty)?;
    let digest = func_bytes(&args[0], cty, "digest")?;
    let signature = func_bytes(&args[1], cty, "signature")?;
    let recovery_id = func_u8(&args[2], cty, "recovery_id")?;
    let digest: [u8; 32] = digest
        .try_into()
        .map_err(|_| ItrErr::new(NativeFuncError, "secp256k1_recover digest must be 32 bytes"))?;
    let signature: [u8; 64] = signature.try_into().map_err(|_| {
        ItrErr::new(
            NativeFuncError,
            "secp256k1_recover signature must be 64 bytes",
        )
    })?;
    let public_key = sys::Account::recover_public_key(&digest, &signature, recovery_id)
        .ok_or_else(|| {
            ItrErr::new(
                NativeFuncError,
                "secp256k1_recover invalid signature or recovery id",
            )
        })?;
    Ok(Value::bytes(public_key.to_vec()))
}

#[cfg(test)]
mod crypto_tests;
