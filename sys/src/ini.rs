use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Ret, errf};

pub type IniObj = HashMap<String, HashMap<String, Option<String>>>;
pub type IniSec = HashMap<String, Option<String>>;

// ================================ typed INI value helpers ================================
//
// These replace the former serde Deserializer layer with explicit per-key
// decoders. Semantics preserved from the old `deserialize_ini`:
//
//   * `key =` (empty value) is absent configuration — the field default
//     applies (`ini_str` returns None, typed helpers return their default);
//   * `key = ""` is an explicit empty string;
//   * bool accepts exactly "true"/"1" and "false"/"0" (anything else errors);
//   * unsigned integers parse strictly or error;
//   * comma-separated values decode as sequences (trimmed, empty items
//     skipped);
//   * `deny_unknown_fields` is expressed by calling `ini_deny_unknown` with
//     the known key list (keys with absent values are tolerated, mirroring
//     the old section iterator which skipped them).

/// Present non-empty value of `key`, or `None` when the key is missing or
/// written as bare `key =` (the old section iterator skipped those).
pub fn ini_str<'a>(sec: &'a IniSec, key: &str) -> Option<&'a str> {
    sec.get(key).and_then(|v| v.as_deref())
}

/// String value with default; `key = ""` yields the explicit empty string.
pub fn ini_str_or(sec: &IniSec, key: &str, def: &str) -> String {
    ini_str(sec, key).unwrap_or(def).to_owned()
}

/// Strict serde-compatible bool: "true"/"1"/"false"/"0", anything else errors
/// (old `deserialize_bool` behavior, not the lenient `ini_must_bool`).
pub fn ini_bool(sec: &IniSec, key: &str, def: bool) -> Ret<bool> {
    let Some(v) = ini_str(sec, key) else {
        return Ok(def);
    };
    match v {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => errf!("config key {key} has invalid bool value {other:?}"),
    }
}

/// Strict integer parse (the old `deserialize_u64`/`u32`/`u16`/`usize`
/// behavior): invalid values error rather than falling back.
pub fn ini_u64(sec: &IniSec, key: &str, def: u64) -> Ret<u64> {
    match ini_str(sec, key) {
        None => Ok(def),
        Some(v) => v.parse().map_err(|_| {
            crate::Error::fault(format!("config key {key} has invalid u64 value {v:?}"))
        }),
    }
}

pub fn ini_u32(sec: &IniSec, key: &str, def: u32) -> Ret<u32> {
    match ini_str(sec, key) {
        None => Ok(def),
        Some(v) => v.parse().map_err(|_| {
            crate::Error::fault(format!("config key {key} has invalid u32 value {v:?}"))
        }),
    }
}

pub fn ini_u16(sec: &IniSec, key: &str, def: u16) -> Ret<u16> {
    match ini_str(sec, key) {
        None => Ok(def),
        Some(v) => v.parse().map_err(|_| {
            crate::Error::fault(format!("config key {key} has invalid u16 value {v:?}"))
        }),
    }
}

pub fn ini_u8(sec: &IniSec, key: &str, def: u8) -> Ret<u8> {
    match ini_str(sec, key) {
        None => Ok(def),
        Some(v) => v.parse().map_err(|_| {
            crate::Error::fault(format!("config key {key} has invalid u8 value {v:?}"))
        }),
    }
}

pub fn ini_usize(sec: &IniSec, key: &str, def: usize) -> Ret<usize> {
    match ini_str(sec, key) {
        None => Ok(def),
        Some(v) => v.parse().map_err(|_| {
            crate::Error::fault(format!("config key {key} has invalid usize value {v:?}"))
        }),
    }
}

/// Comma-separated sequence (trimmed, empty items skipped — the old
/// `deserialize_seq` behavior). `None` when the key is absent or `key =`.
pub fn ini_seq(sec: &IniSec, key: &str) -> Option<Vec<String>> {
    let v = ini_str(sec, key)?;
    Some(
        v.split(',')
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// `deny_unknown_fields` equivalent: any key with a present value outside
/// `allowed` errors; bare `key =` (absent values) are tolerated.
pub fn ini_deny_unknown(sec: &IniSec, section: &str, allowed: &[&str]) -> Ret<()> {
    for (key, value) in sec {
        if value.is_some() && !allowed.contains(&key.as_str()) {
            return errf!("config section [{section}] has unknown field {key}");
        }
    }
    Ok(())
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

/// Parse an INI document from text (used by `load_config` and by callers
/// that hold the text directly, e.g. tests).
pub fn load_ini_text(text: &str) -> Ret<IniObj> {
    parse_ini(text)
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

    fn sec(pairs: &[(&str, Option<&str>)]) -> IniSec {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_owned)))
            .collect()
    }

    #[test]
    fn typed_helpers_decode_like_the_old_serde_layer() {
        let s = sec(&[
            ("listen_port", Some("3337")),
            ("protocol", Some("2")),
            ("empty_key", None),
            ("explicit_empty", Some("")),
            ("maxs", Some("2000, 100, , 5")),
            ("flag", Some("true")),
            ("zero_one", Some("1")),
            ("big_port", Some("70000")),
        ]);
        assert_eq!(ini_u16(&s, "listen_port", 0).unwrap(), 3337);
        assert_eq!(ini_u8(&s, "protocol", 0).unwrap(), 2);
        // `key =` is absent configuration: the default applies.
        assert_eq!(ini_u64(&s, "empty_key", 7).unwrap(), 7);
        // `key = ""` is an explicit empty string.
        assert_eq!(ini_str_or(&s, "explicit_empty", "def"), "");
        assert_eq!(ini_str_or(&s, "missing", "def"), "def");
        // Comma-separated sequences: trimmed, empty items skipped.
        assert_eq!(
            ini_seq(&s, "maxs"),
            Some(vec!["2000".into(), "100".into(), "5".into()])
        );
        assert_eq!(ini_seq(&s, "missing"), None);
        assert_eq!(ini_bool(&s, "flag", false).unwrap(), true);
        assert_eq!(ini_bool(&s, "zero_one", false).unwrap(), true);
        assert!(ini_bool(&s, "listen_port", false).is_err());
        assert!(ini_u64(&s, "flag", 0).is_err());
        assert!(ini_u16(&s, "big_port", 0).is_err()); // out of range
    }

    #[test]
    fn deny_unknown_tolerates_bare_empty_keys() {
        let s = sec(&[("known", Some("1")), ("ghost", None)]);
        assert!(ini_deny_unknown(&s, "t", &["known"]).is_ok());
        let s = sec(&[("known", Some("1")), ("ghost", Some("x"))]);
        assert!(ini_deny_unknown(&s, "t", &["known"]).is_err());
    }

    #[test]
    fn parse_ini_empty_value_is_none_explicit_empty_is_some() {
        let ini = parse_ini("[s]\na =\nb = \"\"\nc = 1\n").unwrap();
        let s = ini_section(&ini, "s");
        assert_eq!(s.get("a"), Some(&None));
        assert_eq!(s.get("b"), Some(&Some(String::new())));
        assert_eq!(s.get("c"), Some(&Some("1".to_owned())));
    }
}
