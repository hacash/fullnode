//! Shared pure formatting rules for action `description()` output.
//!
//! Single source of the cross-crate description conventions
//! (spec: `doc/action-description-spec.md`, version [`DESC_SPEC_VERSION`]):
//! exact HAC decimals, fixed address-table reference notation, quoted
//! printable-ASCII user text vs `0x`-hex fallback, and bounded hex payload
//! previews. Every helper here is a pure function of its input — no RPC, no
//! clock, no locale — so the same action bytes always produce the same
//! description bytes on every platform (native, wasm).
//!
//! These helpers exist so `protocol` / `mint-core` / `vm` format descriptions
//! identically without any of them depending on the SDK or a wallet.

use sys::check_readable_string;

use crate::{AddrOrPtr, Amount};

/// Version tag of the frozen description wording rules (see the spec doc).
pub const DESC_SPEC_VERSION: &str = "action-description@1";

/// Shared byte-payload preview cap for blob-style description previews.
/// Previews never truncate addresses, amounts or safety-critical lists —
/// only opaque byte payloads.
pub const PAYLOAD_PREVIEW_MAX_BYTES: usize = 128;

impl Amount {
    /// Exact decimal HAC rendering: pure integer-string arithmetic, no float,
    /// no 8-digit cap, no rounding, no exponent or thousands separators,
    /// meaningless trailing zeros stripped. Sub-`zhu` residues keep their
    /// full precision (more than 8 decimal places when encoded).
    pub fn to_hac_string(&self) -> String {
        self.to_unit_string("mei")
    }
}

/// Fixed, distinguishable rendering of a wire endpoint: a concrete address
/// renders as its readable string; an address-table reference renders as
/// `<address pointer N>` so it can never be mistaken for a resolved address.
pub fn addr_or_ptr_text(ptr: &AddrOrPtr) -> String {
    match ptr {
        AddrOrPtr::Addr(addr) => addr.to_readable(),
        AddrOrPtr::Ptr(index) => format!("<address pointer {index}>"),
    }
}

/// User-writable byte content with an explicit, unforgeable boundary.
///
/// Fully printable ASCII (0x20..=0x7E) renders as a double-quoted string with
/// `\` escaping for the quote and the backslash itself — no other escapes
/// exist, so the quoted form can never inject a newline or reshape the line.
/// Anything else renders as `0x`-prefixed lowercase hex of the full content;
/// invalid encodings are never dressed up as decoded text.
pub fn user_text(bytes: &[u8]) -> String {
    if check_readable_string(bytes) {
        let mut out = String::with_capacity(bytes.len() + 2);
        out.push('"');
        for &b in bytes {
            if b == b'"' || b == b'\\' {
                out.push('\\');
            }
            out.push(b as char);
        }
        out.push('"');
        out
    } else {
        format!("0x{}", hex::encode(bytes))
    }
}

/// Bounded hex preview of an opaque payload: `0x` + lowercase hex of at most
/// `max_bytes` bytes, with a `...` marker appended only when content was
/// actually truncated. Full byte counts belong to the caller's sentence; the
/// preview itself never claims to be the whole payload unless it is.
pub fn hex_preview(bytes: &[u8], max_bytes: usize) -> String {
    let take = bytes.len().min(max_bytes);
    let mut out = format!("0x{}", hex::encode(&bytes[..take]));
    if take < bytes.len() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hac_decimals_are_exact_and_trim_trailing_zeros() {
        assert_eq!(Amount::from("0:0").unwrap().to_hac_string(), "0");
        assert_eq!(Amount::from("1:248").unwrap().to_hac_string(), "1");
        assert_eq!(Amount::from("37947332:240").unwrap().to_hac_string(), "0.37947332");
        // sub-unit value keeps leading zeros, trailing zeros are stripped
        assert_eq!(Amount::from("1:244").unwrap().to_hac_string(), "0.0001");
        assert_eq!(Amount::from("120:244").unwrap().to_hac_string(), "0.012");
        assert_eq!(Amount::from("123:236").unwrap().to_hac_string(), "0.000000000123");
        assert_eq!(Amount::from("-1:244").unwrap().to_hac_string(), "-0.0001");
    }

    #[test]
    fn addr_refs_stay_distinguishable_from_addresses() {
        let addr = crate::Address::from([0u8; 21]);
        assert_eq!(addr_or_ptr_text(&AddrOrPtr::Addr(addr)), addr.to_readable());
        assert_eq!(addr_or_ptr_text(&AddrOrPtr::Ptr(3)), "<address pointer 3>");
    }

    #[test]
    fn user_text_quotes_ascii_and_escapes_only_quote_and_backslash() {
        assert_eq!(user_text(b"hello"), "\"hello\"");
        assert_eq!(user_text(b"say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(user_text(b"a\\b"), "\"a\\\\b\"");
        // non-ASCII / control content falls back to full hex
        assert_eq!(user_text(&[0x68, 0xc3, 0xa9]), "0x68c3a9");
        assert_eq!(user_text(&[b'a', 0x00, b'b']), "0x610062");
    }

    #[test]
    fn hex_preview_marks_truncation_only_when_truncated() {
        assert_eq!(hex_preview(&[0x01, 0xab], 128), "0x01ab");
        assert_eq!(
            hex_preview(&[0x5a; 128], 128),
            format!("0x{}", "5a".repeat(128))
        );
        assert_eq!(
            hex_preview(&[0x5a; 129], 128),
            format!("0x{}...", "5a".repeat(128))
        );
    }
}
