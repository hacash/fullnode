//! Shared typed-read helper (§6.2 of the error-system normalization design).

use field::Decode;
use sys::Ret;

use crate::{STATE_DECODE_FAILED_CODE, StateRead};

/// Decode a typed value out of a `StateRead` layer. Backend failures propagate unchanged;
/// undecodable bytes are `Abort + STATE_DECODE_FAILED_CODE`, never a missing key — `Ok(None)` is the only not-found answer.
pub fn read_typed<T: Decode>(state: &dyn StateRead, key: &[u8]) -> Ret<Option<T>> {
    match state.get(key)? {
        Some(bytes) => match T::decode(bytes.as_ref()) {
            Ok((value, _)) => Ok(Some(value)),
            Err(e) => Err(sys::Error::abort(format!("state decode failed: {}", e))
                .with_code(STATE_DECODE_FAILED_CODE)),
        },
        None => Ok(None),
    }
}
