//! Persistent state-key namespace helpers.
//!
//! Textual keys always begin with `_` (`0x5f`). Numeric state namespaces begin
//! at `0x01`, use one non-zero byte, and must skip that value, so the first
//! byte alone keeps the two key spaces disjoint.

use field::Encode;

/// First byte reserved for human-readable persistent keys.
pub const TEXT_KEY_PREFIX: u8 = b'_';

/// Validate a numeric state namespace byte.
pub const fn numeric_state_prefix(prefix: u8) -> u8 {
    assert!(prefix != 0, "0x00 is reserved in the state key space");
    assert!(
        prefix != TEXT_KEY_PREFIX,
        "0x5f is reserved for textual state keys"
    );
    prefix
}

pub fn numeric_state_key(prefix: u8, key: &impl Encode) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + key.size());
    out.push(numeric_state_prefix(prefix));
    key.encode_to(&mut out);
    out
}

pub fn numeric_state_empty_key(prefix: u8) -> [u8; 1] {
    [numeric_state_prefix(prefix)]
}
