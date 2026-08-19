use std::collections::HashMap;

use sys::{Ret, errf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JSONBinaryFormat {
    Hex,
    Base64,
}

#[derive(Clone, Debug)]
pub struct JSONFormater {
    pub binary: JSONBinaryFormat,
    pub unit: String,
}

impl Default for JSONFormater {
    fn default() -> Self {
        Self {
            binary: JSONBinaryFormat::Hex,
            unit: String::new(),
        }
    }
}

impl JSONFormater {
    pub fn new_unit(unit: &str) -> Self {
        Self {
            binary: JSONBinaryFormat::Hex,
            unit: unit.to_owned(),
        }
    }
}

pub trait ToJSON {
    fn to_json(&self) -> String {
        self.to_json_fmt(&JSONFormater::default())
    }

    fn to_json_fmt(&self, fmt: &JSONFormater) -> String;
}

pub trait FromJSON {
    fn from_json(&mut self, json: &str) -> Ret<()>;
}

/// Construct a JSON value through the field's existing mutable decoder.
///
/// The helper keeps object decoders transactional: callers can parse every
/// field into temporaries and assign the finished value only after all fields
/// have succeeded.
pub fn json_decode_value<T>(json: &str) -> Ret<T>
where
    T: Default + FromJSON,
{
    let mut value = T::default();
    value.from_json(json)?;
    Ok(value)
}

/// Generate the standard object JSON decoder for a field struct.
#[macro_export]
macro_rules! impl_struct_from_json {
    ($class:ty { $($field:ident),* $(,)? } optional $optional:ident when $condition:ident) => {
        impl $crate::FromJSON for $class {
            fn from_json(&mut self, json: &str) -> sys::Ret<()> {
                let mut next = self.clone();
                let mut seen = std::collections::HashSet::new();
                for (key, value) in $crate::json_split_object(json)? {
                    if !seen.insert(key) {
                        return sys::errf!("{} JSON field {} is duplicated", stringify!($class), key);
                    }
                    match key {
                        $(stringify!($field) => next.$field.from_json(value)?,)*
                        stringify!($optional) => next.$optional.from_json(value)?,
                        _ => {}
                    }
                }
                $(
                    if !seen.contains(stringify!($field)) {
                        return sys::errf!("{} JSON missing field {}", stringify!($class), stringify!($field));
                    }
                )*
                *self = next;
                Ok(())
            }
        }
    };
    ($class:ty { $($field:ident),* $(,)? }) => {
        impl $crate::FromJSON for $class {
            fn from_json(&mut self, json: &str) -> sys::Ret<()> {
                let mut next = self.clone();
                let mut seen = std::collections::HashSet::new();
                for (key, value) in $crate::json_split_object(json)? {
                    if !seen.insert(key) {
                        return sys::errf!("{} JSON field {} is duplicated", stringify!($class), key);
                    }
                    match key {
                        $(stringify!($field) => next.$field.from_json(value)?,)*
                        _ => {}
                    }
                }
                $(
                    if !seen.contains(stringify!($field)) {
                        return sys::errf!("{} JSON missing field {}", stringify!($class), stringify!($field));
                    }
                )*
                *self = next;
                Ok(())
            }
        }
    };
}

/// Generate both directions of the standard object JSON representation.
#[macro_export]
macro_rules! impl_struct_json {
    ($class:ty { $($field:ident),* $(,)? } optional $optional:ident when $condition:ident) => {
        $crate::impl_struct_to_json!($class { $($field),* } optional $optional when $condition);
        $crate::impl_struct_from_json!($class { $($field),* } optional $optional when $condition);
    };
    ($class:ty { $($field:ident),* $(,)? }) => {
        $crate::impl_struct_to_json!($class { $($field),* });
        $crate::impl_struct_from_json!($class { $($field),* });
    };
}

/// Generate the standard object JSON representation for a field struct.
///
/// Builds the object with direct string pushes instead of `format!` so the
/// generated `to_json_fmt` bodies carry no fmt machinery (wasm size; these
/// impls are instantiated for every wire struct).
#[macro_export]
macro_rules! impl_struct_to_json {
    ($class:ty { $($field:ident),* $(,)? } optional $optional:ident when $condition:ident) => {
        impl $crate::ToJSON for $class {
            fn to_json_fmt(&self, fmt: &$crate::JSONFormater) -> String {
                let mut s = String::new();
                s.push('{');
                $(
                    s.push('"');
                    s.push_str(stringify!($field));
                    s.push_str("\":");
                    s.push_str(&$crate::ToJSON::to_json_fmt(&self.$field, fmt));
                    s.push(',');
                )*
                if self.$condition() {
                    s.push('"');
                    s.push_str(stringify!($optional));
                    s.push_str("\":");
                    s.push_str(&$crate::ToJSON::to_json_fmt(&self.$optional, fmt));
                    s.push(',');
                }
                if s.len() > 1 {
                    s.pop();
                }
                s.push('}');
                s
            }
        }
    };
    ($class:ty { $($field:ident),* $(,)? }) => {
        impl $crate::ToJSON for $class {
            fn to_json_fmt(&self, fmt: &$crate::JSONFormater) -> String {
                let mut s = String::new();
                s.push('{');
                $(
                    s.push('"');
                    s.push_str(stringify!($field));
                    s.push_str("\":");
                    s.push_str(&$crate::ToJSON::to_json_fmt(&self.$field, fmt));
                    s.push(',');
                )*
                if s.len() > 1 {
                    s.pop();
                }
                s.push('}');
                s
            }
        }
    };
}

pub fn json_unquote(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

pub fn json_expect_quoted(s: &str) -> Ret<&str> {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        return Ok(&s[1..s.len() - 1]);
    }
    errf!("json string must be quoted")
}

pub fn json_expect_unquoted(s: &str) -> Ret<&str> {
    let s = s.trim();
    if s.starts_with('"') || s.ends_with('"') {
        return errf!("json value must not be quoted");
    }
    Ok(s)
}

// ─── JSON engine (dual implementation) ─────────────────────────────────
//
// The same wire JSON contract (kind/field names, bare numbers, quoted
// strings, binary "0x.."/"b64:..") is implemented by two engines, selected by
// field's `json-serde` feature:
//
//   * serde_engine: fullnode builds (the feature is enabled explicitly by
//     node-side crates app/mint). String unescaping and structure scanning are
//     delegated to serde_json, so the hand-written parser is not compiled into
//     the node binary — hand-written parsing errors stay isolated in the SDK
//     and cannot affect the node core.
//   * handwritten_engine: SDK/wasm builds (json-serde not enabled, field has
//     no serde dependency). Shares the same calling contract as serde_engine:
//     returns raw slices in document order, keeps duplicate keys one by one
//     (the caller does duplicate detection), errors distinguish only success
//     vs failure, not message text.
//
// The two engines intentionally differ in how much malformed input they accept
// (serde stricter, handwritten more lenient): escaped key names, missing
// colons, extra closing brackets/trailing commas, `""""` and similar garbage
// are silently tolerated by the handwritten engine and rejected by serde —
// real data (node ToJSON / SDK / codec.ts output) is always well-formed JSON,
// so this has no effect. Under cfg(test) both engines compile together and the
// engine_equivalence test pins their agreement.

/// serde_json engine (fullnode). Kept compiled under cfg(test) for the
/// equivalence tests.
#[cfg(any(feature = "json-serde", test))]
mod serde_engine {
    use serde::Deserializer;
    use serde::de::{MapAccess, SeqAccess, Visitor};
    use serde_json::value::RawValue;
    use sys::{Ret, errf};

    pub fn quoted_decoded(s: &str) -> Ret<String> {
        let s = s.trim();
        if !(s.starts_with('"') && s.ends_with('"') && s.len() >= 2) {
            return errf!("json string must be quoted");
        }
        serde_json::from_str::<String>(s).map_err(|e| sys::Error::normal(e.to_string()))
    }

    /// Collect raw slices of the top-level array elements in document order
    /// (including duplicates).
    pub fn split_array(s: &str) -> Ret<Vec<&str>> {
        let s = s.trim();
        if !(s.starts_with('[') && s.ends_with(']')) {
            return errf!("json root must be wrapped by '[' and ']'");
        }
        struct ItemsVisitor<'de> {
            items: Vec<&'de str>,
        }
        impl<'de> Visitor<'de> for ItemsVisitor<'de> {
            type Value = Vec<&'de str>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a json array")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                mut self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                while let Some(raw) = access.next_element::<&RawValue>()? {
                    self.items.push(raw.get());
                }
                Ok(self.items)
            }
        }
        let mut de = serde_json::Deserializer::from_str(s);
        let items = Deserializer::deserialize_seq(&mut de, ItemsVisitor { items: Vec::new() })
            .map_err(|e| sys::Error::normal(e.to_string()))?;
        de.end().map_err(|e| sys::Error::normal(e.to_string()))?;
        Ok(items)
    }

    /// Collect top-level (key, raw value slice) pairs in document order,
    /// keeping duplicate keys one by one. Keys must be unescaped strings (wire
    /// field names are all ASCII; escaped keys error, though the handwritten
    /// engine tolerates them — see the module comment).
    pub fn split_object(s: &str) -> Ret<Vec<(&str, &str)>> {
        let s = s.trim();
        if !(s.starts_with('{') && s.ends_with('}')) {
            return errf!("json root must be wrapped by '{}' and '{}'", '{', '}');
        }
        struct PairsVisitor<'de> {
            pairs: Vec<(&'de str, &'de str)>,
        }
        impl<'de> Visitor<'de> for PairsVisitor<'de> {
            type Value = Vec<(&'de str, &'de str)>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a json object")
            }
            fn visit_map<A: MapAccess<'de>>(
                mut self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                while let Some(key) = access.next_key::<&str>()? {
                    let raw: &RawValue = access.next_value()?;
                    self.pairs.push((key, raw.get()));
                }
                Ok(self.pairs)
            }
        }
        let mut de = serde_json::Deserializer::from_str(s);
        let pairs = Deserializer::deserialize_map(&mut de, PairsVisitor { pairs: Vec::new() })
            .map_err(|e| sys::Error::normal(e.to_string()))?;
        de.end().map_err(|e| sys::Error::normal(e.to_string()))?;
        Ok(pairs)
    }
}

/// Handwritten engine (SDK/wasm). Kept compiled under cfg(test) for the
/// equivalence tests.
#[cfg(any(not(feature = "json-serde"), test))]
mod handwritten_engine {
    use sys::{Ret, errf};

    /// Handwritten JSON string unescaping (no serde_json dependency):
    /// \" \\ \/ \b \f \n \r \t \uXXXX (including surrogate pairs).
    pub fn quoted_decoded(s: &str) -> Ret<String> {
        let s = s.trim();
        let inner = if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
            &s[1..s.len() - 1]
        } else {
            return errf!("json string must be quoted");
        };
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                // Consistent with serde_json: unescaped control characters are invalid
                if (c as u32) < 0x20 {
                    return errf!("json string has unescaped control character");
                }
                out.push(c);
                continue;
            }
            let Some(esc) = chars.next() else {
                return errf!("json string ends with incomplete escape");
            };
            match esc {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let code = json_hex4(&mut chars)?;
                    if (0xD800..0xDC00).contains(&code) {
                        // A high surrogate must be immediately followed by a
                        // \uXXXX low surrogate (serde_json-compatible:
                        // producers like Python's ensure_ascii emit surrogate
                        // pairs for non-BMP characters)
                        if chars.next() != Some('\\') || chars.next() != Some('u') {
                            return errf!("json \\u high surrogate must be followed by a low surrogate");
                        }
                        let low = json_hex4(&mut chars)?;
                        if !(0xDC00..0xE000).contains(&low) {
                            return errf!("json \\u surrogate pair is invalid");
                        }
                        let combined = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        let ch = char::from_u32(combined).ok_or_else(|| {
                            sys::Error::normal(format!("json \\u escape invalid: \\u{code:x}"))
                        })?;
                        out.push(ch);
                    } else if (0xDC00..0xE000).contains(&code) {
                        return errf!("json \\u low surrogate without a high surrogate");
                    } else {
                        let ch = char::from_u32(code).ok_or_else(|| {
                            sys::Error::normal(format!("json \\u escape invalid: \\u{code:x}"))
                        })?;
                        out.push(ch);
                    }
                }
                _ => return errf!("json string has invalid escape \\{}", esc),
            }
        }
        Ok(out)
    }

    /// Read a 4-hex-digit escape (the XXXX part of `\uXXXX`).
    fn json_hex4(chars: &mut std::str::Chars) -> Ret<u32> {
        let mut hex = String::with_capacity(4);
        for _ in 0..4 {
            let Some(h) = chars.next() else {
                return errf!("json \\u escape is truncated");
            };
            hex.push(h);
        }
        u32::from_str_radix(&hex, 16)
            .map_err(|_| sys::Error::normal(format!("json \\u escape invalid: \\u{hex}")))
    }

    /// Split array/object content by top-level commas, returning raw slices.
    fn split(s: &str, start_char: char, end_char: char) -> Ret<Vec<&str>> {
        let s = s.trim();
        if !s.starts_with(start_char) || !s.ends_with(end_char) {
            return errf!(
                "json root must be wrapped by '{}' and '{}'",
                start_char,
                end_char
            );
        }
        let content = &s[1..s.len() - 1];
        let mut items = Vec::new();
        let mut depth = 0i32;
        let mut last = 0usize;
        let mut in_quote = false;
        let mut escaped = false;
        for (i, c) in content.char_indices() {
            if in_quote {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_quote = false;
                }
                continue;
            }
            match c {
                '"' => in_quote = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth < 0 {
                        return errf!("json brackets are unbalanced");
                    }
                }
                ',' if depth == 0 => {
                    items.push(content[last..i].trim());
                    last = i + 1;
                }
                _ => {}
            }
        }
        if in_quote {
            return errf!("json string is not closed");
        }
        if depth != 0 {
            return errf!("json brackets are unbalanced");
        }
        let tail = content[last..].trim();
        if !tail.is_empty() {
            items.push(tail);
        }
        Ok(items)
    }

    pub fn split_array(s: &str) -> Ret<Vec<&str>> {
        split(s, '[', ']')
    }

    pub fn split_object(s: &str) -> Ret<Vec<(&str, &str)>> {
        let mut out = Vec::new();
        for pair in split(s, '{', '}')? {
            let mut in_quote = false;
            let mut escaped = false;
            let mut colon = None;
            for (i, c) in pair.char_indices() {
                if in_quote {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        in_quote = false;
                    }
                    continue;
                }
                if c == '"' {
                    in_quote = true;
                } else if c == ':' {
                    colon = Some(i);
                    break;
                }
            }
            let Some(i) = colon else {
                return errf!("json object member is missing ':'");
            };
            out.push((super::json_unquote(pair[..i].trim()), pair[i + 1..].trim()));
        }
        Ok(out)
    }
}

#[cfg(feature = "json-serde")]
use self::serde_engine as engine;
#[cfg(not(feature = "json-serde"))]
use self::handwritten_engine as engine;

/// Decode a quoted JSON string (escapes + surrogate pairs), returning the
/// decoded content.
pub fn json_expect_quoted_decoded(s: &str) -> Ret<String> {
    engine::quoted_decoded(s)
}

/// Split a JSON array, returning raw slices of its top-level elements.
pub fn json_split_array(s: &str) -> Ret<Vec<&str>> {
    engine::split_array(s)
}

/// Split a JSON object, returning (key, raw value slice) pairs in document
/// order, keeping duplicate keys.
pub fn json_split_object(s: &str) -> Ret<Vec<(&str, &str)>> {
    engine::split_object(s)
}

pub fn json_decode_object(s: &str) -> Ret<HashMap<String, String>> {
    Ok(json_split_object(s)?
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect())
}

pub fn json_decode_array(s: &str) -> Ret<(Vec<String>, usize)> {
    let items = json_split_array(s)?
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let n = items.len();
    Ok((items, n))
}

pub fn json_decode_binary(s: &str) -> Ret<Vec<u8>> {
    let raw = json_expect_quoted_decoded(s)?;
    let trimmed = raw.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return hex::decode(hex).map_err(|e| sys::Error::normal(e.to_string()));
    }
    if let Some(b64) = trimmed
        .strip_prefix("b64:")
        .or_else(|| trimmed.strip_prefix("B64:"))
    {
        use base64::prelude::*;
        return BASE64_STANDARD
            .decode(b64)
            .map_err(|e| sys::Error::normal(e.to_string()));
    }
    Ok(raw.into_bytes())
}

#[cfg(test)]
mod engine_equivalence {
    // Both engines (serde_engine / handwritten_engine) must agree on
    // success/failure and results for the same input. The intentional
    // acceptance differences on malformed input (escaped key names, trailing
    // commas, extra closing brackets, etc. — see the module comment) are not
    // covered here.
    use super::{handwritten_engine, serde_engine};
    use sys::Ret;

    fn compare<T: PartialEq + std::fmt::Debug>(
        kind: &str,
        input: &str,
        hand: Ret<T>,
        serde: Ret<T>,
    ) {
        match (&hand, &serde) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{kind} diverged on {input:?}"),
            (Err(_), Err(_)) => {}
            _ => panic!("{kind} diverged on {input:?}: handwritten={hand:?} serde={serde:?}"),
        }
    }

    #[test]
    fn quoted_decoded_agrees() {
        for s in [
            // Both accept: plain text/unicode/all escapes/surrogate pairs/edge whitespace
            "\"hello\"",
            "\"\"",
            "\"a b\"",
            "\"中文\"",
            "\"a\\\"b\"",
            "\"a\\\\b\"",
            "\"\\\\u1234\"",
            "\"\\/\"",
            "\"\\b\\f\\n\\r\\t\"",
            "\"\\u4e2d\\u6587\"",
            "\"\\uD83D\\uDE00\"",
            "\"\\uD800\\uDC00\"",
            "\"\\u0000\"",
            "\"\\u000A\"",
            "\"\\u007f\"",
            "  \"a\"  ",
            "\"a\" ",
            // Both reject: lone surrogates/truncated escapes/invalid escapes/unclosed/bare values/control characters
            "\"\\uD800\"",
            "\"\\uDC00\"",
            "\"\\uD800x\"",
            "\"\\u12\"",
            "\"\\uZZZZ\"",
            "\"a\\qb\"",
            "\"abc",
            "abc",
            "\"a\"x",
            "'a'",
            "\"",
            "\"\\",
            "\"a\tb\"",
            "\"a\nb\"",
        ] {
            compare(
                "quoted_decoded",
                s,
                handwritten_engine::quoted_decoded(s),
                serde_engine::quoted_decoded(s),
            );
        }
    }

    #[test]
    fn split_object_agrees() {
        for s in [
            // Both accept: nesting/escaped values/duplicate keys/unicode keys/numbers and booleans
            "{}",
            "{ }",
            "{\"a\":1}",
            "{\"a\":1,\"b\":2}",
            "{ \"a\" : 1 , \"b\" : 2 }",
            "{\"a\":1,\"a\":2}",
            "{\"a\":{\"x\":1},\"b\":[1,2]}",
            "{\"a\":\"x,y\",\"b\":\"z\\\"w\"}",
            "{\"a\":\"{x}\",\"b\":1}",
            "{\"a\":\"\"}",
            "{\"a\":[1,{\"b\":2}]}",
            "{\"中文\":1}",
            "{\"a\":-1}",
            "{\"a\":null}",
            "{\"a\":1.5e10}",
            "{\"a\":true}",
            "{\"a\":\"\\u4e2d\"}",
            "{\"a\":\"\\uD83D\\uDE00\"}",
            "{\"a\":\"0x12\",\"b\":\"b64:AA==\"}",
            "{\"a\":[]}",
            "{\"a\":[1,2,3]}",
            "{ \"a\" : { \"b\" : [ 1 , 2 ] } }",
            "{\"a\":  \"x\"  }",
            // Both reject: missing brackets/wrong brackets/trailing garbage
            "{",
            "{\"a\":1",
            "[1,2]",
            "{\"a\":1} trailing",
            "{ } x",
        ] {
            compare(
                "split_object",
                s,
                handwritten_engine::split_object(s),
                serde_engine::split_object(s),
            );
        }
    }

    #[test]
    fn split_array_agrees() {
        for s in [
            // Both accept: mixed types/nesting/escaped strings
            "[]",
            "[1]",
            "[1,2,3]",
            "[\"a\",\"b\"]",
            "[1,\"a\",[2,{\"x\":3}]]",
            "[{}]",
            "[ ]",
            "[1, 2]",
            "[true,false,null]",
            "[1.5]",
            "[\"0x12\",\"b64:AA==\"]",
            "[\"\\u4e2d\"]",
            "[\"\\uD83D\\uDE00\"]",
            "[1,2,3] ",
            // Both reject: missing brackets/wrong brackets/trailing garbage
            "[",
            "[1,2",
            "{\"a\":1}",
            "[]x",
        ] {
            compare(
                "split_array",
                s,
                handwritten_engine::split_array(s),
                serde_engine::split_array(s),
            );
        }
    }

    /// Dynamic binary formats (`0x..`/`b64:..`), unit-qualified amounts and
    /// mixed-case variants: both engines must decode the same quoted string,
    /// and the format layer (`json_decode_binary`) must yield the same bytes
    /// from either engine's output.
    #[test]
    fn dynamic_formats_agree() {
        for s in [
            // hex / base64 binary formats (lower/upper prefixes, mixed case)
            "\"0x12\"",
            "\"0X12\"",
            "\"0xdeadBEEF00\"",
            "\"0x\"",
            "\"b64:AA==\"",
            "\"B64:AA==\"",
            "\"b64:SGVsbG8sIFdvcmxkIQ==\"",
            "\"b64:\"",
            // unit-qualified amount strings (format layer parses the unit part)
            "\"1.5HAC\"",
            "\"1000SAT\"",
            "\"2.34\"",
            // plain text (no recognized prefix stays raw bytes)
            "\"plain text\"",
            // nested dynamic formats survive structure scanning
            "{\"a\":\"0x12\",\"b\":\"b64:AA==\",\"c\":\"1.5HAC\"}",
            "[\"0xdeadBEEF00\",\"b64:SGVsbG8=\",\"0X00\"]",
        ] {
            compare(
                "quoted_decoded (dynamic formats)",
                s,
                handwritten_engine::quoted_decoded(s),
                serde_engine::quoted_decoded(s),
            );
        }
        // The format layer is engine-agnostic, but pin the contract end to
        // end: identical bytes from both engines' decoded strings.
        for s in [
            "\"0xdeadBEEF00\"",
            "\"0X12\"",
            "\"b64:SGVsbG8sIFdvcmxkIQ==\"",
            "\"B64:AA==\"",
            "\"plain text\"",
            "\"1.5HAC\"",
        ] {
            let hand = handwritten_engine::quoted_decoded(s).unwrap();
            let serde = serde_engine::quoted_decoded(s).unwrap();
            let hand_bytes = super::json_decode_binary(&format!("\"{hand}\"")).unwrap();
            let serde_bytes = super::json_decode_binary(&format!("\"{serde}\"")).unwrap();
            assert_eq!(hand_bytes, serde_bytes, "format layer diverged on {s:?}");
        }
    }
}
