use std::sync::Arc;
use std::{future::Future, pin::Pin};

use crate::api::SandboxLimiter;
use crate::chain::Engine;
use crate::node::Node;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiMethod {
    Get,
    Post,
}

#[derive(Clone, Debug, Default)]
pub struct ApiRequest {
    pub query: std::collections::HashMap<String, String>,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Vec<u8>,
    /// TCP peer reported by the HTTP server.  Security-sensitive rate limits
    /// must use this value rather than client-controlled forwarding headers.
    pub peer_ip: Option<std::net::IpAddr>,
}

impl ApiRequest {
    pub fn query(&self, key: &str) -> Option<&str> {
        self.query.get(key).map(|s| s.as_str())
    }
    pub fn query_u64(&self, key: &str) -> Option<u64> {
        self.query(key).and_then(|s| s.parse().ok())
    }
    pub fn query_usize(&self, key: &str) -> Option<usize> {
        self.query(key).and_then(|s| s.parse().ok())
    }
}

#[derive(Clone, Debug)]
pub struct ApiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ApiResponse {
    pub fn json(body: String) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: body.into_bytes(),
        }
    }
    pub fn text(body: String) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            body: body.into_bytes(),
        }
    }
    pub fn err(status: u16, msg: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            body: msg.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone)]
pub struct ApiExecCtx {
    pub engine: Arc<dyn Engine>,
    pub node: Arc<dyn Node>,
    pub launch_time: u64,
    /// §13.2 shared concurrency + wall-clock limiter for VM sandbox calls.
    /// Routes that invoke `contract_sandbox_call` / `debug_contract_storage`
    /// acquire a permit from this limiter before running the sandbox so that
    /// untrusted contract bytecodes cannot starve root writers or exhaust the
    /// HTTP worker pool.  Routes that do not touch the VM sandbox simply
    /// ignore this field.
    pub sandbox_limiter: SandboxLimiter,
}

pub type ApiHandler = Arc<dyn Fn(&ApiExecCtx, ApiRequest) -> ApiResponse + Send + Sync>;

pub type ApiHandlerAsync = Arc<
    dyn Fn(ApiExecCtx, ApiRequest) -> Pin<Box<dyn Future<Output = ApiResponse> + Send>>
        + Send
        + Sync,
>;

/// §8.2 of the error-system design: the single API mapping entry for state
/// read errors. Canonical state failures (`Abort` — StorageRead, StateDecode,
/// EngineUnavailable) map to HTTP 503; business errors keep the plain JSON
/// error body.
pub fn api_state_read_error(e: &sys::Error) -> ApiResponse {
    if e.is_abort() {
        ApiResponse::err(503, &format!("state read failed: {}", e))
    } else {
        let mut encoded = String::with_capacity(e.to_string().len() + 16);
        encoded.push('"');
        for ch in format!("state read failed: {}", e).chars() {
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
        ApiResponse::json(format!("{{\"ret\":1,\"err\":{}}}", encoded))
    }
}
