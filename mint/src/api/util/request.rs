use base::{ApiExecCtx, ApiRequest, ApiResponse, OptimisticState};
use std::fmt::Write;
use sys::{ToBase64, ToHex};

pub(crate) const UNIT_238: u128 = 100_0000_0000;

/// Optimistic session acquisition with the unavailable case expressed
/// distinctly: a fatal/stopping engine fails the request through the unified
/// error mapping, a busy engine keeps the plain "state changed" response
/// (§5/§8.2 of the error-system design).
pub(crate) fn optimistic_snapshot(ctx: &ApiExecCtx) -> Result<OptimisticState, ApiResponse> {
    match ctx.engine.optimistic_canonical() {
        Ok(Some(snapshot)) => Ok(snapshot),
        Ok(None) => Err(api_error("state changed")),
        Err(e) => Err(api_state_read_error(&e)),
    }
}

pub(crate) fn hac238_to_unit(v: u128) -> f64 {
    v as f64 / UNIT_238 as f64
}

pub(crate) fn json_string(v: &str) -> String {
    let mut encoded = String::with_capacity(v.len() + 2);
    encoded.push('"');
    for ch in v.chars() {
        match ch {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0C}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            c if c <= '\u{1F}' => write!(&mut encoded, "\\u{:04x}", c as u32).unwrap(),
            c => encoded.push(c),
        }
    }
    encoded.push('"');
    encoded
}

pub(crate) fn q_string(req: &ApiRequest, key: &str, default: &str) -> String {
    req.query(key)
        .map_or_else(|| default.to_owned(), |s| s.to_owned())
}

pub(crate) fn q_bool(req: &ApiRequest, key: &str, default: bool) -> bool {
    let Some(v) = req.query(key) else {
        return default;
    };
    !matches!(
        v,
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

pub(crate) fn q_i64(req: &ApiRequest, key: &str, default: i64) -> i64 {
    req.query(key)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

pub(crate) fn q_u32(req: &ApiRequest, key: &str, default: u32) -> u32 {
    req.query(key)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(default)
}

pub(crate) fn q_f64(req: &ApiRequest, key: &str, default: f64) -> f64 {
    req.query(key)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default)
}

pub(crate) fn q_coinkind_hsd(req: &ApiRequest) -> sys::Ret<(bool, bool, bool)> {
    let raw = q_string(req, "coinkind", "hsd");
    let mut s = raw.to_lowercase();
    s.retain(|c| !c.is_whitespace() && c != ',' && c != ';' && c != '|');
    if s.is_empty() || s == "all" || s == "hsda" {
        return Ok((true, true, true));
    }
    if !s
        .chars()
        .all(|c| c == 'h' || c == 's' || c == 'd' || c == 'a')
    {
        return sys::errf!("coinkind format invalid");
    }
    Ok((s.contains('h'), s.contains('s'), s.contains('d')))
}

pub(crate) fn api_error(errmsg: &str) -> ApiResponse {
    ApiResponse::json(format!("{{\"ret\":1,\"err\":{}}}", json_string(errmsg)))
}

/// §8.2 of the error-system design: the single API mapping entry lives in
/// `base` and is shared by every query handler and the VM sandbox API.
pub(crate) use base::api_state_read_error;

pub(crate) fn api_ok(fields: Vec<(&str, String)>) -> ApiResponse {
    let fields = fields
        .into_iter()
        .map(|(k, v)| format!("\"{}\":{}", k, v))
        .collect::<Vec<_>>()
        .join(",");
    ApiResponse::json(format!("{{\"ret\":0,{}}}", fields))
}

pub(crate) fn api_bytes(data: Vec<u8>, content_type: &str) -> ApiResponse {
    ApiResponse {
        status: 200,
        headers: vec![("content-type".to_owned(), content_type.to_owned())],
        body: data,
    }
}

pub(crate) fn api_html(s: String) -> ApiResponse {
    api_bytes(s.into_bytes(), "text/html; charset=utf-8")
}

pub(crate) fn api_data_list_field(name: &str, latest: i64, list: Vec<String>) -> ApiResponse {
    ApiResponse::json(format!(
        "{{\"ret\":0,\"{}\":{},\"list\":[{}]}}",
        name,
        latest,
        list.join(",")
    ))
}

pub(crate) fn encode_miner_bytes(v: &[u8], is_base64: bool) -> String {
    if is_base64 { v.to_base64() } else { v.to_hex() }
}

pub(crate) fn get_id_range(max: i64, page: i64, limit: i64, instart: i64, desc: bool) -> Vec<i64> {
    if max < 1 || page < 1 || limit < 1 {
        return vec![];
    }
    let limit = limit.min(200);
    let mut start = 1;
    if instart != i64::MAX {
        start = instart;
    }
    if desc && instart == i64::MAX {
        start = max;
    }
    if page > 1 {
        let Some(offset) = (page - 1).checked_mul(limit) else {
            return vec![];
        };
        if desc {
            let Some(next) = start.checked_sub(offset) else {
                return vec![];
            };
            start = next;
        } else {
            let Some(next) = start.checked_add(offset) else {
                return vec![];
            };
            start = next;
        }
    }
    let mut rng = Vec::with_capacity(limit as usize);
    for offset in 0..limit {
        let id = if desc {
            start.checked_sub(offset)
        } else {
            start.checked_add(offset)
        };
        let Some(id) = id else {
            break;
        };
        if id >= 1 && id <= max {
            rng.push(id);
        }
    }
    rng
}

pub(crate) fn body_data_may_hex(req: &ApiRequest) -> sys::Ret<Vec<u8>> {
    if !q_bool(req, "hexbody", false) {
        return Ok(req.body.clone());
    }
    hex::decode(&req.body).map_err(|_| sys::Error::fault("hex format invalid"))
}

#[cfg(test)]
mod tests {
    use super::{api_state_read_error, get_id_range, json_string};

    #[test]
    fn json_string_escapes_control_characters() {
        assert_eq!(
            json_string("a\"\\\n\t\u{0001}"),
            "\"a\\\"\\\\\\n\\t\\u0001\""
        );
    }

    #[test]
    fn id_range_rejects_invalid_and_out_of_bounds_pages() {
        assert_eq!(get_id_range(10, 1, 3, i64::MAX, true), vec![10, 9, 8]);
        assert_eq!(get_id_range(10, 2, 3, i64::MAX, true), vec![7, 6, 5]);
        assert_eq!(get_id_range(10, 1, 3, i64::MAX, false), vec![1, 2, 3]);
        assert!(get_id_range(10, 999_999_999, 3, i64::MAX, true).is_empty());
        assert!(get_id_range(10, 0, 3, i64::MAX, true).is_empty());
        assert!(get_id_range(10, 1, 0, i64::MAX, true).is_empty());
    }

    /// Test 11 of §10.2: canonical state read failures map to HTTP 503
    /// through the single API mapping entry, while business decode/revert
    /// errors stay ordinary business errors (§8.2).
    #[test]
    fn api_mapping_503_only_for_canonical_state_failures() {
        let read =
            sys::Error::abort("backend down").with_code(base::STATE_READ_FAILED_CODE);
        assert_eq!(api_state_read_error(&read).status, 503);
        let decode =
            sys::Error::abort("bad bytes").with_code(base::STATE_DECODE_FAILED_CODE);
        assert_eq!(api_state_read_error(&decode).status, 503);
        let unavailable =
            sys::Error::abort("engine stopping").with_code("engine_unavailable");
        assert_eq!(api_state_read_error(&unavailable).status, 503);

        let revert = sys::Error::revert("user revert");
        assert_ne!(api_state_read_error(&revert).status, 503);
        let fault = sys::Error::fault("business error");
        assert_ne!(api_state_read_error(&fault).status, 503);
        let decode_err = sys::Error::normal("bad input");
        assert_ne!(api_state_read_error(&decode_err).status, 503);
    }
}
