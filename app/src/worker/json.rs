//! Minimal JSON object access for the standalone mining workers (no serde): parses remote
//! fullnode responses with field's hand-written engine — flat objects, one level of nesting.

use field::{
    json_expect_quoted_decoded, json_expect_unquoted, json_split_array, json_split_object,
};
use sys::Ret;

pub(crate) struct JsonObj {
    pairs: Vec<(String, String)>,
}

impl JsonObj {
    pub(crate) fn parse(text: &str) -> Ret<Self> {
        let pairs = json_split_object(text)
            .map_err(|e| sys::Error::fault(format!("invalid json response: {}", e)))?
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        Ok(Self { pairs })
    }

    /// Nested object value (e.g. `born` inside `{"born":{"hash":...}}`).
    pub(crate) fn obj(&self, key: &str) -> Ret<Self> {
        let raw = self
            .raw(key)
            .ok_or_else(|| sys::Error::fault(format!("missing json object field {}", key)))?;
        Self::parse(raw)
    }

    fn raw(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Decoded string value (`"..."` on the wire).
    pub(crate) fn get_str(&self, key: &str) -> Ret<String> {
        let raw = self
            .raw(key)
            .ok_or_else(|| sys::Error::fault(format!("missing json string field {}", key)))?;
        json_expect_quoted_decoded(raw)
            .map_err(|e| sys::Error::fault(format!("invalid json string field {}: {}", key, e)))
    }

    fn get_int<T>(&self, key: &str) -> Ret<T>
    where
        T: std::str::FromStr,
    {
        let raw = self
            .raw(key)
            .ok_or_else(|| sys::Error::fault(format!("missing json number field {}", key)))?;
        json_expect_unquoted(raw)
            .map_err(|e| sys::Error::fault(format!("invalid json number field {}: {}", key, e)))?
            .parse()
            .map_err(|_| sys::Error::fault(format!("invalid json number field {}", key)))
    }

    pub(crate) fn get_u64(&self, key: &str) -> Ret<u64> {
        self.get_int(key)
    }

    pub(crate) fn get_i64(&self, key: &str) -> Ret<i64> {
        self.get_int(key)
    }

    /// Array of decoded strings (e.g. `mkrl_modify_list`).
    pub(crate) fn get_str_array(&self, key: &str) -> Ret<Vec<String>> {
        let raw = self
            .raw(key)
            .ok_or_else(|| sys::Error::fault(format!("missing json array field {}", key)))?;
        let items = json_split_array(raw)
            .map_err(|e| sys::Error::fault(format!("invalid json array field {}: {}", key, e)))?;
        items
            .iter()
            .map(|v| {
                json_expect_quoted_decoded(v).map_err(|e| {
                    sys::Error::fault(format!("invalid json array item in {}: {}", key, e))
                })
            })
            .collect()
    }

    /// Compact re-serialization for log lines.
    pub(crate) fn display(&self) -> String {
        let fields = self
            .pairs
            .iter()
            .map(|(k, v)| format!("{}:{}", field::json_escape(k), v))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{{}}}", fields)
    }
}
