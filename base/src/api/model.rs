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

/// One block-submit admission check. `Some` denies the broadcast; `None` lets the
/// next registered gate run.
pub type BlockSubmitGate = Arc<dyn Fn(&ApiExecCtx) -> Option<ApiResponse> + Send + Sync>;

/// Gates run in registration order. The first denial is the HTTP response.
#[derive(Clone, Default)]
pub struct BlockSubmitGates {
    gates: Vec<BlockSubmitGate>,
}

impl BlockSubmitGates {
    pub fn push(&mut self, gate: BlockSubmitGate) {
        self.gates.push(gate);
    }

    fn snapshot(&self) -> Vec<BlockSubmitGate> {
        self.gates.clone()
    }
}

#[derive(Clone)]
pub struct ApiExecCtx {
    pub engine: Arc<dyn Engine>,
    pub node: Arc<dyn Node>,
    pub launch_time: u64,
    /// §13.2 shared concurrency + wall-clock limiter for VM sandbox calls.
    /// Sandbox routes acquire a permit before running; non-sandbox routes ignore it.
    pub sandbox_limiter: SandboxLimiter,
    pub block_submit_gates: BlockSubmitGates,
}

impl ApiExecCtx {
    /// Ask each registered gate, in order. The first denial is returned.
    pub fn reject_block_submit(&self) -> Option<ApiResponse> {
        let gates = self.block_submit_gates.snapshot();
        for gate in gates {
            if let Some(resp) = gate(self) {
                return Some(resp);
            }
        }
        None
    }
}

pub type ApiHandler = Arc<dyn Fn(&ApiExecCtx, ApiRequest) -> ApiResponse + Send + Sync>;

pub type ApiHandlerAsync = Arc<
    dyn Fn(ApiExecCtx, ApiRequest) -> Pin<Box<dyn Future<Output = ApiResponse> + Send>>
        + Send
        + Sync,
>;

/// §8.2: canonical `Abort` state-read failures (StorageRead, StateDecode,
/// EngineUnavailable) map to HTTP 503; business errors keep the plain JSON body.
pub fn api_state_read_error(e: &sys::Error) -> ApiResponse {
    if e.is_abort() {
        ApiResponse::err(503, &format!("state read failed: {}", e))
    } else {
        ApiResponse::json(format!(
            "{{\"ret\":1,\"err\":{}}}",
            field::json_escape(&format!("state read failed: {}", e))
        ))
    }
}
