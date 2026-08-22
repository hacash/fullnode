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

/// Visit a JSON object under the canonical codec rules: duplicated and
/// unknown fields are rejected before the caller decodes any value.
/// The visitor is `dyn` rather than generic so the walk framework is compiled
/// once instead of once per decoder call site (wasm size); object field lists
/// are short, so duplicate detection is a linear scan over a `Vec`, not a hash set.
pub fn json_object_fields<'a>(
    json: &'a str,
    allowed: &[&str],
    visit: &mut dyn FnMut(&'a str, &'a str) -> Ret<()>,
) -> Ret<()> {
    let mut seen: Vec<&str> = Vec::new();
    for (key, value) in json_split_object(json)? {
        if seen.contains(&key) {
            return errf!("JSON field {} is duplicated", key);
        }
        seen.push(key);
        if !allowed.contains(&key) {
            return errf!("JSON field {} is unknown", key);
        }
        visit(key, value)?;
    }
    Ok(())
}

/// Split a JSON object while enforcing the codec-wide duplicate-key rule.
/// Dynamic counterpart to [`json_object_fields`] for second-stage registry decoders (no allow-list).
pub fn json_object_entries<'a>(json: &'a str) -> Ret<Vec<(&'a str, &'a str)>> {
    let mut seen: Vec<&str> = Vec::new();
    let mut entries = Vec::new();
    for (key, value) in json_split_object(json)? {
        if seen.contains(&key) {
            return errf!("JSON field {} is duplicated", key);
        }
        seen.push(key);
        entries.push((key, value));
    }
    Ok(entries)
}

/// Construct a JSON value through the field's existing mutable decoder.
/// Keeps object decoders transactional: parse fields into temporaries, assign only after all succeed.
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
                let mut seen: Vec<&str> = Vec::new();
                $crate::json_object_fields(json, &[$(stringify!($field)),*, stringify!($optional)], &mut |key, value| {
                    seen.push(key);
                    match key {
                        $(stringify!($field) => next.$field.from_json(value)?,)*
                        stringify!($optional) => next.$optional.from_json(value)?,
                        _ => return sys::errf!("{} JSON field {} is unknown", stringify!($class), key),
                    }
                    Ok(())
                })?;
                $(
                    if !seen.contains(&stringify!($field)) {
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
                let mut seen: Vec<&str> = Vec::new();
                $crate::json_object_fields(json, &[$(stringify!($field)),*], &mut |key, value| {
                    seen.push(key);
                    match key {
                        $(stringify!($field) => next.$field.from_json(value)?,)*
                        _ => return sys::errf!("{} JSON field {} is unknown", stringify!($class), key),
                    }
                    Ok(())
                })?;
                $(
                    if !seen.contains(&stringify!($field)) {
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

/// Generate the wire-action JSON object: numeric `kind` plus action body fields.
/// Irregular actions share this without depending on the protocol-facing `base` crate.
#[macro_export]
macro_rules! impl_action_json {
    ($class:ty { $($field:ident),* $(,)? }) => {
        impl $crate::ToJSON for $class {
            fn to_json_fmt(&self, fmt: &$crate::JSONFormater) -> String {
                let mut s = String::new();
                s.push_str("{\"kind\":");
                s.push_str(&$crate::ToJSON::to_json_fmt(&self.kind, fmt));
                $(
                    s.push(',');
                    s.push('"');
                    s.push_str(stringify!($field));
                    s.push_str("\":");
                    s.push_str(&$crate::ToJSON::to_json_fmt(&self.$field, fmt));
                )*
                s.push('}');
                s
            }
        }
    };
}

/// Generate the standard object JSON representation for a field struct.
/// Uses direct string pushes instead of `format!` so generated bodies carry no fmt machinery (wasm size).
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

/// Escape `s` as a JSON string literal, matching serde_json's default output.
/// The single shared escaper — mint's API and the VM sandbox API delegate here.
pub fn json_escape(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() + 2);
    encoded.push('"');
    for ch in s.chars() {
        match ch {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0C}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            c if c <= '\u{1F}' => {
                use std::fmt::Write;
                let _ = write!(&mut encoded, "\\u{:04x}", c as u32);
            }
            c => encoded.push(c),
        }
    }
    encoded.push('"');
    encoded
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

// ─── JSON engine (single implementation) ───────────────────────────────
// One hand-written engine for SDK/wasm and fullnode — serde_json-equivalent on the wire contract (serde is only a dev test oracle).

/// serde_json oracle engine. Compiled under cfg(test) only, for the
/// equivalence tests and the differential fuzz.
#[cfg(test)]
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
    /// keeping duplicate keys one by one (wire field names are unescaped ASCII).
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

/// Handwritten JSON engine — the single production parser for every build.
/// Strictly serde_json-equivalent on the shared contract (pinned by tests).
mod handwritten_engine {
    use sys::{Ret, errf};

    /// serde_json's default recursion limit; documents nested deeper than
    /// this are rejected by both engines.
    const MAX_DEPTH: i32 = 128;

    /// Handwritten JSON string unescaping (no serde_json dependency):
    /// \" \\ \/ \b \f \n \r \t \uXXXX (incl. surrogate pairs); input must be exactly one quoted string, closing quote must end it.
    pub fn quoted_decoded(s: &str) -> Ret<String> {
        let s = s.trim();
        if !s.starts_with('"') {
            return errf!("json string must be quoted");
        }
        let mut out = String::with_capacity(s.len());
        let mut chars = s[1..].chars();
        while let Some(c) = chars.next() {
            if c == '"' {
                // An unescaped quote is the closing quote: it must end the input.
                if chars.as_str().trim().is_empty() {
                    return Ok(out);
                }
                return errf!("json string has trailing characters after closing quote");
            }
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
                        // A high surrogate must be immediately followed by a \uXXXX low surrogate
                        // (serde_json-compatible — Python's ensure_ascii emits surrogate pairs).
                        if chars.next() != Some('\\') || chars.next() != Some('u') {
                            return errf!(
                                "json \\u high surrogate must be followed by a low surrogate"
                            );
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
        errf!("json string is not closed")
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

    /// Validate that `s` is exactly one quoted string (escape syntax checked
    /// by `quoted_decoded`, which also validates surrogate pairs).
    fn validate_string(s: &str) -> Ret<()> {
        let mut escaped = false;
        let mut end = None;
        for (i, c) in s.char_indices().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '"' {
                end = Some(i);
                break;
            }
            if (c as u32) < 0x20 {
                return errf!("json string has unescaped control character");
            }
        }
        let Some(end) = end else {
            return errf!("json string is not closed");
        };
        quoted_decoded(&s[..=end])?;
        if !s[end + 1..].trim().is_empty() {
            return errf!("json string has trailing characters after closing quote");
        }
        Ok(())
    }

    /// Strict JSON number grammar: -?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?
    fn validate_number(s: &str) -> Ret<()> {
        let mut chars = s.chars().peekable();
        if chars.peek() == Some(&'-') {
            chars.next();
        }
        match chars.next() {
            Some('0') => {
                if matches!(chars.peek(), Some('0'..='9')) {
                    return errf!("json number has leading zeros");
                }
            }
            Some('1'..='9') => {
                while matches!(chars.peek(), Some('0'..='9')) {
                    chars.next();
                }
            }
            _ => return errf!("json number is invalid"),
        }
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut frac_digits = 0;
            while matches!(chars.peek(), Some('0'..='9')) {
                chars.next();
                frac_digits += 1;
            }
            if frac_digits == 0 {
                return errf!("json number fraction is incomplete");
            }
        }
        if matches!(chars.peek(), Some('e') | Some('E')) {
            chars.next();
            if matches!(chars.peek(), Some('+') | Some('-')) {
                chars.next();
            }
            let mut exp_digits = 0;
            while matches!(chars.peek(), Some('0'..='9')) {
                chars.next();
                exp_digits += 1;
            }
            if exp_digits == 0 {
                return errf!("json number exponent is incomplete");
            }
        }
        if chars.peek().is_some() {
            return errf!("json number has trailing characters");
        }
        Ok(())
    }

    fn expect_literal(s: &str, lit: &str) -> Ret<()> {
        if s == lit {
            Ok(())
        } else {
            errf!("json literal is invalid")
        }
    }

    /// Validate that `s` is exactly one complete JSON value, recursion bounded by the
    /// `MAX_DEPTH` check inside `split`. Nested objects use lenient keys; top-level keys stay unescaped.
    fn validate_value(s: &str) -> Ret<()> {
        let s = s.trim();
        let Some(c) = s.chars().next() else {
            return errf!("json value is empty");
        };
        match c {
            '"' => validate_string(s),
            '{' => {
                split_object_impl(s, false)?;
                Ok(())
            }
            '[' => {
                split_array(s)?;
                Ok(())
            }
            't' => expect_literal(s, "true"),
            'f' => expect_literal(s, "false"),
            'n' => expect_literal(s, "null"),
            '-' | '0'..='9' => validate_number(s),
            _ => errf!("json value is invalid"),
        }
    }

    /// Split array/object content by top-level commas, returning raw slices.
    /// Strict: empty items, trailing commas, unterminated quotes and over-deep nesting are rejected.
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
        let mut saw_top_comma = false;
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
                '{' | '[' => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return errf!("json nesting is too deep");
                    }
                }
                '}' | ']' => {
                    depth -= 1;
                    if depth < 0 {
                        return errf!("json brackets are unbalanced");
                    }
                }
                ',' if depth == 0 => {
                    let item = content[last..i].trim();
                    if item.is_empty() {
                        return errf!("json has an empty item");
                    }
                    items.push(item);
                    last = i + 1;
                    saw_top_comma = true;
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
        } else if saw_top_comma {
            return errf!("json has a trailing comma");
        }
        Ok(items)
    }

    pub fn split_array(s: &str) -> Ret<Vec<&str>> {
        let items = split(s, '[', ']')?;
        for item in &items {
            validate_value(item)?;
        }
        Ok(items)
    }

    pub fn split_object(s: &str) -> Ret<Vec<(&str, &str)>> {
        split_object_impl(s, true)
    }

    /// Shared object splitter. `strict_keys` (public entry) requires plain unescaped
    /// keys; lenient keys (nested values) are validated as full JSON strings, escapes allowed.
    fn split_object_impl(s: &str, strict_keys: bool) -> Ret<Vec<(&str, &str)>> {
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
            let key_raw = pair[..i].trim();
            if !(key_raw.starts_with('"') && key_raw.ends_with('"') && key_raw.len() >= 2) {
                return errf!("json object key must be a quoted string");
            }
            if strict_keys {
                if key_raw.contains('\\') {
                    return errf!("json object key must not be escaped");
                }
            } else {
                validate_string(key_raw)?;
            }
            let value = pair[i + 1..].trim();
            validate_value(value)?;
            out.push((super::json_unquote(key_raw), value));
        }
        Ok(out)
    }
}

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

/// Decode an object into an owned map after applying the canonical duplicate key rule.
/// Use [`json_object_entries`] when field order or raw slices must be preserved.
pub fn json_decode_object(s: &str) -> Ret<HashMap<String, String>> {
    Ok(json_object_entries(s)?
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
    // Both engines must agree on success/failure and results for the same input;
    // the former "intentional acceptance differences" are now strict-equivalence cases.
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
            // Strict-equivalence cases (previously tolerated by handwritten):
            // extra quotes before/after the value, quote before end of input
            "\"\"\"\"",
            "\"a\" \"b\"",
            "\"a\"\"",
            "\"\"a\"",
            "\"a\\\"\"",
            "\"a\"\n",
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
            "{\"\":1}",
            "{\"a\":-1}",
            "{\"a\":-0}",
            "{\"a\":0}",
            "{\"a\":null}",
            "{\"a\":1.5e10}",
            "{\"a\":1E5}",
            "{\"a\":0.5e-2}",
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
            "{\"a\":1}{",
            // Strict-equivalence cases (previously tolerated): escaped/unquoted keys,
            // trailing commas, empty items, bad/duplicated colons, empty values, invalid numbers, missing commas
            "{\"\\u0061\":1}",
            "{\"a\\\\b\":1}",
            "{a:1}",
            "{\"a\":1,}",
            "{\"a\":1,,}",
            "{\"a\":1,,\"b\":2}",
            "{\"a\":1,\"b\":2,}",
            "{\"a\":}",
            "{\"a\" 1}",
            "{\"a\"::1}",
            "{\"a\"::1,\"b\":2}",
            "{\"a\":01}",
            "{\"a\":-01}",
            "{\"a\":1.}",
            "{\"a\":.5}",
            "{\"a\":+1}",
            "{\"a\":1e}",
            "{\"a\":1e+}",
            "{\"a\":1 2}",
            "{\"a\":1 \"b\":2}",
            "{\"a\":tru}",
            "{\"a\":True}",
            "{\"a\":NaN}",
            "{\"a\":Infinity}",
            "{\"a\":\"1\" \"2\"}",
            "{\"a\":[1,2]}",
            "{\"a\":[1,2}",
            "{\"a\":{\"b\":1}}",
            "{\"a\":{\"b\":1}",
            // Nested-content cases: the raw-value path allows escaped keys inside
            // nested structures, while root map keys stay unescaped-only.
            "{\"a\":{\"\\u0061\":1}}",
            "{\"a\":{\"a\\\\b\":1}}",
            "{\"a\":{\"b\":[1,2]}}",
            "{\"a\":{\"b\":1 2}}",
            "{\"a\":[1,,2]}",
            "{\"a\":{\"b\":01}}",
            "{\"a\":{\"b\":tru}}",
            "{\"a\":[1,]}",
            "{\"a\":{\"b\" 1}}",
            "{\"a\":{b:1}}",
            "{\"a\":[truex]}",
            "{\"a\":\"abc}",
            "{\"a\":{\"b\":\"abc}",
            "{\"a\":[1 2]}",
            "{\"a\":{\"b\":1}{\"c\":2}}",
            "{\"a\":{\"b\":1,\"c\":2,\"d\":}}",
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
            // Strict-equivalence cases (previously tolerated): trailing commas,
            // empty items, missing commas, invalid numbers, unterminated strings
            "[1,]",
            "[1,,2]",
            "[1 2]",
            "[\"a\" \"b\"]",
            "[01]",
            "[1.]",
            "[+1]",
            "[tru]",
            "[\"abc]",
            "[1,2]x",
            "[[]",
            // Nested-content cases (see split_object): escaped keys inside
            // nested objects are allowed on the raw-value path.
            "[{\"\\u0061\":1}]",
            "[{\"b\":1 2}]",
            "[[1,,2]]",
            "[{\"a\":1},]",
            "[{\"b\":\"abc}",
        ] {
            compare(
                "split_array",
                s,
                handwritten_engine::split_array(s),
                serde_engine::split_array(s),
            );
        }
    }

    /// Dynamic binary formats (`0x..`/`b64:..`), unit-qualified amounts, mixed case:
    /// both engines decode the same string; `json_decode_binary` yields the same bytes.
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

    /// Nesting depth: the handwritten engine caps recursion at `MAX_DEPTH` (DoS guard),
    /// the one intentional divergence — serde's raw-value oracle has no recursion limit.
    #[test]
    fn recursion_depth_boundary_agrees() {
        for n in [100usize, 126, 127, 128, 129] {
            let arr = format!("{}1{}", "[".repeat(n), "]".repeat(n));
            assert!(
                handwritten_engine::split_array(&arr).is_ok(),
                "handwritten must accept {n}-deep arrays (below MAX_DEPTH)"
            );
            let obj = format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n));
            assert!(
                handwritten_engine::split_object(&obj).is_ok(),
                "handwritten must accept {n}-deep objects (below MAX_DEPTH)"
            );
        }
        // At and beyond the cap the handwritten engine rejects (DoS guard);
        // serde's raw-value oracle still accepts (documented divergence).
        for n in [130usize, 200] {
            let arr = format!("{}1{}", "[".repeat(n), "]".repeat(n));
            assert!(handwritten_engine::split_array(&arr).is_err());
            assert!(serde_engine::split_array(&arr).is_ok());
            let obj = format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n));
            assert!(handwritten_engine::split_object(&obj).is_err());
            assert!(serde_engine::split_object(&obj).is_ok());
        }
    }

    /// The shared string escaper must match serde_json's output exactly
    /// (replaces the former `serde_json::to_string` uses in API and VM layers).
    #[test]
    fn json_escape_matches_serde() {
        for s in [
            "",
            "plain",
            "a\"b",
            "a\\b",
            "tab\there",
            "newline\nhere",
            "cr\rhere",
            "back\u{8}space",
            "form\u{C}feed",
            "\u{01}control",
            "中文 emoji 😀",
            "mix \"\\\t\n\r\u{1F}\u{7f}end",
        ] {
            assert_eq!(
                super::json_escape(s),
                serde_json::to_string(s).unwrap(),
                "json_escape diverged on {s:?}"
            );
            // And it round-trips through the engine's own decoder.
            assert_eq!(
                super::json_expect_quoted_decoded(&super::json_escape(s)).unwrap(),
                s
            );
        }
    }

    /// Deterministic differential fuzz: both engines must agree on every input
    /// (valid/malformed/garbage). Fixed seeds — the corpus is reproducible.
    #[test]
    fn differential_fuzz_agrees() {
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.0
            }
            fn below(&mut self, n: u64) -> usize {
                (self.next() % n) as usize
            }
        }

        // Grammar-driven generator: produces well-formed docs, near-miss
        // malformed docs and token garbage.
        fn gen_doc(rng: &mut Lcg, depth: usize, sb: &mut String) {
            match rng.below(100) {
                // bare tokens (garbage mix)
                0..=24 => {
                    sb.push(
                        [
                            '{', '}', '[', ']', '"', '\\', ':', ',', ' ', '\t', '\n', '0', '1',
                            '9', '-', '+', '.', 'e', 'E', 't', 'f', 'n', 'u', 'a', 'x',
                        ][rng.below(25)],
                    );
                    return;
                }
                // numbers
                25..=34 => {
                    let mut num = String::new();
                    if rng.below(2) == 0 {
                        num.push('-');
                    }
                    if rng.below(3) == 0 {
                        num.push('0');
                    } else {
                        for _ in 0..1 + rng.below(3) {
                            num.push((b'1' + rng.below(9) as u8) as char);
                        }
                    }
                    if rng.below(2) == 0 {
                        num.push('.');
                        for _ in 0..1 + rng.below(2) {
                            num.push((b'0' + rng.below(10) as u8) as char);
                        }
                    }
                    if rng.below(2) == 0 {
                        num.push('e');
                        if rng.below(2) == 0 {
                            num.push(if rng.below(2) == 0 { '+' } else { '-' });
                        }
                        num.push((b'0' + rng.below(10) as u8) as char);
                    }
                    sb.push_str(&num);
                    return;
                }
                // strings (valid escapes)
                35..=54 => {
                    sb.push('"');
                    let n = rng.below(4);
                    for _ in 0..n {
                        match rng.below(6) {
                            0 => sb.push_str("\\\""),
                            1 => sb.push_str("\\\\"),
                            2 => sb.push_str("\\u4e2d"),
                            3 => sb.push_str("\\uD83D\\uDE00"),
                            4 => sb.push_str("\\n"),
                            _ => sb.push((b'a' + rng.below(26) as u8) as char),
                        }
                    }
                    sb.push('"');
                    return;
                }
                55..=58 => sb.push_str(["true", "false", "null", "\"str\""][rng.below(4)]),
                // structured nesting
                59..=79 if depth < 4 => {
                    if rng.below(2) == 0 {
                        sb.push('[');
                        let n = rng.below(4);
                        for i in 0..n {
                            if i > 0 {
                                sb.push(',');
                            }
                            gen_doc(rng, depth + 1, sb);
                        }
                        sb.push(']');
                    } else {
                        sb.push('{');
                        let n = rng.below(3);
                        for i in 0..n {
                            if i > 0 {
                                sb.push(',');
                            }
                            sb.push('"');
                            sb.push((b'a' + rng.below(4) as u8) as char);
                            sb.push('"');
                            sb.push(':');
                            gen_doc(rng, depth + 1, sb);
                        }
                        sb.push('}');
                    }
                    return;
                }
                // near-miss malformed: trailing commas, empty items, escaped
                // keys, unquoted keys, missing colons, bad numbers, unclosed
                80..=88 => {
                    let pick = rng.below(8);
                    let base = match pick {
                        0 => "[1,2,",
                        1 => "{\"a\":1,,}",
                        2 => "{\"\\u0061\":1}",
                        3 => "{a:1}",
                        4 => "{\"a\" 1}",
                        5 => "{\"a\":01}",
                        6 => "[\"abc",
                        7 => "{\"a\":1 \"b\":2}",
                        _ => "{\"a\":}",
                    };
                    sb.push_str(base);
                    if rng.below(2) == 0 {
                        sb.push('}');
                    }
                    return;
                }
                _ => {
                    sb.push(match rng.below(3) {
                        0 => '{',
                        1 => '[',
                        _ => '"',
                    });
                    return;
                }
            };
        }

        let mut rng = Lcg(0x9E3779B97F4A7C15);
        for round in 0..3000 {
            let mut doc = String::new();
            gen_doc(&mut rng, 0, &mut doc);
            let label = format!("fuzz#{round}");
            compare(
                &label,
                &doc,
                handwritten_engine::quoted_decoded(&doc),
                serde_engine::quoted_decoded(&doc),
            );
            compare(
                &label,
                &doc,
                handwritten_engine::split_array(&doc),
                serde_engine::split_array(&doc),
            );
            compare(
                &label,
                &doc,
                handwritten_engine::split_object(&doc),
                serde_engine::split_object(&doc),
            );
        }
    }

    #[test]
    fn object_entries_reject_duplicate_keys_without_filtering_dynamic_fields() {
        let entries = super::json_object_entries(r#"{"kind":1,"amount":2}"#).unwrap();
        assert_eq!(entries, vec![("kind", "1"), ("amount", "2")]);
        assert!(super::json_object_entries(r#"{"kind":1,"kind":2}"#).is_err());
    }
}
