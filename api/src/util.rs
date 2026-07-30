//! Shared JSON / hex helpers for built-in API services.

use base::ApiResponse;

pub(crate) fn json_string(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn api_json_error(err: &str) -> ApiResponse {
    ApiResponse::json(format!("{{\"ret\":1,\"err\":{}}}", json_string(err)))
}

pub(crate) fn hex_short(b: &[u8]) -> String {
    hex_bytes(&b.iter().take(4).copied().collect::<Vec<_>>())
}

pub(crate) fn hex_bytes(b: &[u8]) -> String {
    b.iter().map(|c| format!("{:02x}", c)).collect()
}
