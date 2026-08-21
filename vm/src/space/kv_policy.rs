//! Shared limits and validation for volatile KV maps (global / memory / intent),
//! plus reusable scalar length checks for contract status and persistent storage.

use crate::rt::ItrErrCode;
use crate::rt::SpaceCap;
use crate::rt::*;
use crate::value::Value;

#[derive(Clone, Copy, Debug)]
pub struct VolatileKvLimits {
    pub key_max_bytes: usize,
    pub value_max_bytes: usize,
}

impl VolatileKvLimits {
    pub fn from_space_cap(cap: &SpaceCap) -> Self {
        Self {
            key_max_bytes: cap.kv_key_size,
            value_max_bytes: cap.value_size,
        }
    }
}

/// Scalar payload length vs `value_max_bytes` and field serialization (`< u16::MAX` bytes).
pub fn validate_scalar_payload_against_max(
    val: &Value,
    value_max_bytes: usize,
    ec: ItrErrCode,
) -> VmrtErr {
    let val_len = val.extract_bytes_len_with_error_code(ec)?;
    if !SpaceCap::scalar_field_len_fits(val_len, value_max_bytes) {
        let eff_max = value_max_bytes.min(SpaceCap::FIELD_BYTES_SERIALIZE_MAX);
        return itr_err_fmt!(
            ec,
            "value too long, max {} bytes but got {}",
            eff_max,
            val_len
        );
    }
    Ok(())
}

/// Scalar payload length vs `value_max_bytes` (encoded byte length from [`Value::extract_bytes_len`]).
pub fn validate_scalar_payload_len(val: &Value, value_max_bytes: usize, ec: ItrErrCode) -> VmrtErr {
    validate_scalar_payload_against_max(val, value_max_bytes, ec)
}

/// Non-nil scalar whose encoded length is within `value_max_bytes`.
pub fn validate_volatile_scalar_put(
    val: &Value,
    value_max_bytes: usize,
    ec: ItrErrCode,
) -> VmrtErr {
    val.check_non_nil_scalar(ec)
        .map_err(|ItrErr(_, msg)| ItrErr::new(ec, &msg))?;
    validate_scalar_payload_against_max(val, value_max_bytes, ec)
}

/// Validates one `(key, value)` pair before put into `GKVMap` / contract `MKVMap`. With `allow_nil_value`
/// and `Nil` val only `key` is checked (delete semantics); otherwise non-nil scalar ≤ `value_max_bytes`.
pub fn validate_volatile_kv_put(
    key: &Value,
    val: &Value,
    limits: &VolatileKvLimits,
    allow_nil_value: bool,
    ec: ItrErrCode,
) -> VmrtErr {
    let key_bytes = key.extract_key_bytes_with_error_code(ec)?;
    if key_bytes.len() > limits.key_max_bytes {
        return itr_err_fmt!(
            ec,
            "key too long, max {} bytes but got {}",
            limits.key_max_bytes,
            key_bytes.len()
        );
    }
    if allow_nil_value && matches!(val, Value::Nil) {
        return Ok(());
    }
    validate_volatile_scalar_put(val, limits.value_max_bytes, ec)?;
    Ok(())
}
