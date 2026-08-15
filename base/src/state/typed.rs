//! Shared typed-read helper (§6.2 of the error-system normalization design).

use field::Decode;
use sys::Ret;

use crate::{STATE_DECODE_FAILED_CODE, StateRead};

/// Decode a typed value out of a `StateRead` layer. Backend read failures are
/// propagated unchanged (`Abort + STATE_READ_FAILED_CODE`); bytes that were
/// successfully read but fail protocol decode are reported as
/// `Abort + STATE_DECODE_FAILED_CODE`, never
/// as a missing key. A missing key stays `Ok(None)`.
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
