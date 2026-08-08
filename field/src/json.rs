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
#[macro_export]
macro_rules! impl_struct_to_json {
    ($class:ty { $($field:ident),* $(,)? } optional $optional:ident when $condition:ident) => {
        impl $crate::ToJSON for $class {
            fn to_json_fmt(&self, fmt: &$crate::JSONFormater) -> String {
                let mut fields = Vec::new();
                $(
                    fields.push(format!(
                        "\"{}\":{}",
                        stringify!($field),
                        $crate::ToJSON::to_json_fmt(&self.$field, fmt)
                    ));
                )*
                if self.$condition() {
                    fields.push(format!(
                        "\"{}\":{}",
                        stringify!($optional),
                        $crate::ToJSON::to_json_fmt(&self.$optional, fmt)
                    ));
                }
                format!("{{{}}}", fields.join(","))
            }
        }
    };
    ($class:ty { $($field:ident),* $(,)? }) => {
        impl $crate::ToJSON for $class {
            fn to_json_fmt(&self, fmt: &$crate::JSONFormater) -> String {
                let mut fields = Vec::new();
                $(
                    fields.push(format!(
                        "\"{}\":{}",
                        stringify!($field),
                        $crate::ToJSON::to_json_fmt(&self.$field, fmt)
                    ));
                )*
                format!("{{{}}}", fields.join(","))
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

pub fn json_expect_quoted_decoded(s: &str) -> Ret<String> {
    serde_json::from_str(s.trim()).map_err(|e| sys::Error::decode(e.to_string()))
}

pub fn json_split(s: &str, start_char: char, end_char: char) -> Ret<Vec<&str>> {
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
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                items.push(content[last..i].trim());
                last = i + 1;
            }
            _ => {}
        }
    }
    let tail = content[last..].trim();
    if !tail.is_empty() {
        items.push(tail);
    }
    Ok(items)
}

pub fn json_split_array(s: &str) -> Ret<Vec<&str>> {
    json_split(s, '[', ']')
}

pub fn json_split_object(s: &str) -> Ret<Vec<(&str, &str)>> {
    Ok(json_split(s, '{', '}')?
        .into_iter()
        .filter_map(|pair| {
            let mut in_quote = false;
            let mut escaped = false;
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
                    return Some((json_unquote(pair[..i].trim()), pair[i + 1..].trim()));
                }
            }
            None
        })
        .collect())
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
        return hex::decode(hex).map_err(|e| sys::Error::decode(e.to_string()));
    }
    if let Some(b64) = trimmed
        .strip_prefix("b64:")
        .or_else(|| trimmed.strip_prefix("B64:"))
    {
        use base64::prelude::*;
        return BASE64_STANDARD
            .decode(b64)
            .map_err(|e| sys::Error::decode(e.to_string()));
    }
    Ok(raw.into_bytes())
}
