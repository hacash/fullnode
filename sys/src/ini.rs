use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Ret, errf};
use serde::de::{self, DeserializeOwned, IntoDeserializer, MapAccess, SeqAccess, Visitor};

pub type IniObj = HashMap<String, HashMap<String, Option<String>>>;

/// Deserialize an INI document whose sections map to nested Rust structs.
/// Scalar values are interpreted by the destination field type; comma-separated
/// values deserialize as sequences. This keeps file and runtime config shapes
/// identical without a field-by-field copying layer. The outer config type must
/// allow unknown fields when independently-owned sections (such as `[hascan]`)
/// are present; known section structs can still use `deny_unknown_fields`.
pub fn deserialize_ini<T: DeserializeOwned>(ini: &IniObj) -> Ret<T> {
    T::deserialize(IniRoot { ini }).map_err(|e| crate::Error::fault(format!("config decode: {e}")))
}

struct IniRoot<'a> {
    ini: &'a IniObj,
}
struct IniSection<'a> {
    values: &'a HashMap<String, Option<String>>,
}
struct IniValue<'a> {
    value: Option<&'a str>,
}

type DeError = de::value::Error;

struct RootMap<'a> {
    iter: std::collections::hash_map::Iter<'a, String, HashMap<String, Option<String>>>,
    value: Option<&'a HashMap<String, Option<String>>>,
}
struct SectionMap<'a> {
    iter: std::collections::hash_map::Iter<'a, String, Option<String>>,
    value: Option<Option<&'a str>>,
}

impl<'de> MapAccess<'de> for RootMap<'de> {
    type Error = DeError;
    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        let Some((key, value)) = self.iter.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(key.as_str().into_deserializer()).map(Some)
    }
    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        seed.deserialize(IniSection {
            values: self
                .value
                .take()
                .ok_or_else(|| de::Error::custom("INI section missing"))?,
        })
    }
}

impl<'de> MapAccess<'de> for SectionMap<'de> {
    type Error = DeError;
    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        loop {
            let Some((key, value)) = self.iter.next() else {
                return Ok(None);
            };
            // `key =` is absent configuration, so serde can apply the field
            // default. `key = ""` remains an explicit empty string.
            let Some(value) = value.as_deref() else {
                continue;
            };
            self.value = Some(Some(value));
            return seed.deserialize(key.as_str().into_deserializer()).map(Some);
        }
    }
    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        seed.deserialize(IniValue {
            value: self
                .value
                .take()
                .ok_or_else(|| de::Error::custom("INI value missing"))?,
        })
    }
}

impl<'de> de::Deserializer<'de> for IniRoot<'de> {
    type Error = DeError;
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(RootMap {
            iter: self.ini.iter(),
            value: None,
        })
    }
    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
    fn deserialize_struct<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
    serde::forward_to_deserialize_any! { bool i8 i16 i32 i64 u8 u16 u32 u64 u128 i128 f32 f64 char str string bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct enum identifier ignored_any }
}
impl<'de> de::Deserializer<'de> for IniSection<'de> {
    type Error = DeError;
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(SectionMap {
            iter: self.values.iter(),
            value: None,
        })
    }
    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
    fn deserialize_struct<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
    serde::forward_to_deserialize_any! { bool i8 i16 i32 i64 u8 u16 u32 u64 u128 i128 f32 f64 char str string bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct enum identifier ignored_any }
}

impl<'de> de::Deserializer<'de> for IniValue<'de> {
    type Error = DeError;
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value.unwrap_or(""))
    }
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Some(_) => visitor.visit_some(self),
            None => visitor.visit_none(),
        }
    }
    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value.unwrap_or("") {
            "true" | "1" => visitor.visit_bool(true),
            "false" | "0" => visitor.visit_bool(false),
            v => Err(de::Error::custom(format!("invalid bool {v:?}"))),
        }
    }
    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u64(
            self.value
                .unwrap_or("")
                .parse()
                .map_err(de::Error::custom)?,
        )
    }
    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u32(
            self.value
                .unwrap_or("")
                .parse()
                .map_err(de::Error::custom)?,
        )
    }
    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u16(
            self.value
                .unwrap_or("")
                .parse()
                .map_err(de::Error::custom)?,
        )
    }
    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u8(
            self.value
                .unwrap_or("")
                .parse()
                .map_err(de::Error::custom)?,
        )
    }
    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_string(self.value.unwrap_or("").to_owned())
    }
    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value.unwrap_or(""))
    }
    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(IniSeq {
            iter: self.value.unwrap_or("").split(','),
        })
    }
    serde::forward_to_deserialize_any! { i8 i16 i32 i64 i128 u128 f32 f64 char bytes byte_buf unit unit_struct newtype_struct tuple tuple_struct map struct enum identifier ignored_any }
}
struct IniSeq<'a> {
    iter: std::str::Split<'a, char>,
}
impl<'de> SeqAccess<'de> for IniSeq<'de> {
    type Error = DeError;
    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.iter.next().map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => seed.deserialize(IniValue { value: Some(v) }).map(Some),
            None => Ok(None),
        }
    }
}

pub fn join_path(a: &Path, b: &str) -> PathBuf {
    let mut a = a.to_path_buf();
    a.push(b);
    a
}

pub fn get_current_exe_absolute_dir(dir: &str) -> PathBuf {
    let path = PathBuf::from(dir);
    if path.is_absolute() {
        return path;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(path)
}

pub fn get_mainnet_data_dir(ini: &IniObj) -> PathBuf {
    let sec = ini_section(ini, "default");
    let data_dir = ini_must(sec, "data_dir", "hacash_mainnet_data");
    get_current_exe_absolute_dir(&data_dir)
}

pub fn ini_section<'a>(ini: &'a IniObj, key: &str) -> &'a HashMap<String, Option<String>> {
    static EMPTY: std::sync::OnceLock<HashMap<String, Option<String>>> = std::sync::OnceLock::new();
    ini.get(key)
        .unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
}

pub fn ini_must(sec: &HashMap<String, Option<String>>, key: &str, def: &str) -> String {
    ini_must_maxlen(sec, key, def, 0)
}

pub fn ini_must_maxlen(
    sec: &HashMap<String, Option<String>>,
    key: &str,
    def: &str,
    ml: usize,
) -> String {
    let mut val = sec
        .get(key)
        .and_then(|v| v.as_deref())
        .unwrap_or(def)
        .to_owned();
    if ml > 0 && val.len() > ml {
        let mut cut = ml;
        while cut > 0 && !val.is_char_boundary(cut) {
            cut -= 1;
        }
        val.truncate(cut);
    }
    val
}

pub fn ini_must_u64(sec: &HashMap<String, Option<String>>, key: &str, dv: u64) -> u64 {
    ini_must(sec, key, &dv.to_string()).parse().unwrap_or(dv)
}

pub fn ini_must_f64(sec: &HashMap<String, Option<String>>, key: &str, dv: f64) -> f64 {
    ini_must(sec, key, &dv.to_string()).parse().unwrap_or(dv)
}

pub fn ini_must_bool(sec: &HashMap<String, Option<String>>, key: &str, dv: bool) -> bool {
    let def = if dv { "true" } else { "false" };
    let val = ini_must(sec, key, def);
    !matches!(
        val.as_str(),
        "false"
            | "False"
            | "FALSE"
            | "none"
            | "None"
            | "NONE"
            | "null"
            | "Null"
            | "NULL"
            | "0"
            | "_"
            | ""
    )
}

pub fn load_config(path: impl AsRef<Path>) -> Ret<IniObj> {
    let requested = path.as_ref();
    let config_path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join(requested)
    };
    if !config_path.exists() {
        return errf!("cannot find config file {}", config_path.display());
    }
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| crate::Error::fault(format!("read config failed: {}", e)))?;
    parse_ini(&text)
}

fn parse_ini(text: &str) -> Ret<IniObj> {
    let mut out = IniObj::new();
    let mut section = "default".to_owned();
    out.entry(section.clone()).or_default();
    for (idx, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return errf!("config line {} section format invalid", idx + 1);
            }
            section = line[1..line.len() - 1].trim().to_owned();
            out.entry(section.clone()).or_default();
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            return errf!("config line {} key value format invalid", idx + 1);
        };
        let key = key.trim();
        if key.is_empty() {
            return errf!("config line {} key is empty", idx + 1);
        }
        let val = val.trim();
        out.entry(section.clone()).or_default().insert(
            key.to_owned(),
            (!val.is_empty()).then(|| unquote_ini_value(val)),
        );
    }
    Ok(out)
}

fn strip_comment(line: &str) -> &str {
    let mut in_dq = false; // double quote "
    let mut in_sq = false; // single quote '
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' if !in_sq => in_dq = !in_dq,
            '\'' if !in_dq => in_sq = !in_sq,
            '#' | ';' if !in_dq && !in_sq => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn unquote_ini_value(val: &str) -> String {
    if val.len() >= 2
        && ((val.starts_with('"') && val.ends_with('"'))
            || (val.starts_with('\'') && val.ends_with('\'')))
    {
        val[1..val.len() - 1].to_owned()
    } else {
        val.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Config {
        p2p: P2P,
        txpool: TxPool,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct P2P {
        listen_port: u16,
        protocol: u8,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct TxPool {
        maxs: Vec<usize>,
    }

    #[test]
    fn typed_unsigned_values_and_lists_decode() {
        let ini =
            parse_ini("[p2p]\nlisten_port = 3337\nprotocol = 2\n[txpool]\nmaxs = 2000, 100\n")
                .unwrap();
        let got: Config = deserialize_ini(&ini).unwrap();
        assert_eq!(
            got,
            Config {
                p2p: P2P {
                    listen_port: 3337,
                    protocol: 2,
                },
                txpool: TxPool {
                    maxs: vec![2000, 100],
                },
            }
        );
    }
}
