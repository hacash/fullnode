//! HTTP server core.
//!
//! This module hosts transport, routing, and ApiService dispatch only. Concrete
//! API services live in sibling modules and may depend on protocol-specific
//! crates when their routes are chain-specific.
//!
//! Prefer `HttpServer::start_on(Handle, Waiter)` under a shared runtime;
//! `Server::start` keeps a private-runtime fallback that blocks until shutdown.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use base::{
    ApiExecCtx, ApiHandler, ApiHandlerAsync, ApiHandlerKind, ApiMethod, ApiRequest, ApiResponse,
    ApiRoute, ApiService, Node, Server, ServerConfig,
};
use sys::{Rerr, Waiter};

pub struct HttpServer {
    node: Arc<dyn Node>,
    services: Vec<Arc<dyn ApiService>>,
    config: ServerConfig,
    launch_time: u64,
}

impl HttpServer {
    pub fn open(
        node: Arc<dyn Node>,
        services: Vec<Arc<dyn ApiService>>,
        config: ServerConfig,
        launch_time: u64,
    ) -> Self {
        Self {
            node,
            services,
            config,
            launch_time,
        }
    }

    pub fn service_names(&self) -> Vec<&str> {
        self.services.iter().map(|s| s.name()).collect()
    }

    pub fn collect_routes(&self) -> Vec<ApiRoute> {
        let mut all = Vec::new();
        for s in &self.services {
            all.extend(s.routes());
        }
        all
    }

    pub fn make_exec_ctx(&self) -> ApiExecCtx {
        ApiExecCtx {
            engine: self.node.engine(),
            node: self.node.clone(),
            launch_time: self.launch_time,
            // §13.2: default sandbox limiter.  Override by constructing the
            // ApiExecCtx directly if a deployment needs different caps.
            sandbox_limiter: base::SandboxLimiter::default(),
        }
    }
}

impl Server for HttpServer {
    /// Standalone: private multi-thread runtime, blocks until shutdown.
    fn start(&self, waiter: Waiter) -> Rerr {
        if !self.config.enable || self.config.listen_port == 0 {
            return Ok(());
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| sys::Error::fault(format!("tokio runtime build failed: {}", e)))?;
        rt.block_on(self.run_http_async(waiter))
    }

    fn stop(&self) {}
}

impl HttpServer {
    /// Attach HTTP server to an existing tokio runtime (preferred for unified assembly).
    /// Returns immediately after spawning the serve task.
    pub fn start_on(&self, handle: &tokio::runtime::Handle, waiter: Waiter) -> Rerr {
        if !self.config.enable || self.config.listen_port == 0 {
            return Ok(());
        }
        let addr = SocketAddr::new(self.config.listen_ip, self.config.listen_port);
        let listener = bind_http_on(handle, addr)?;
        let app = build_router(
            self.make_exec_ctx(),
            self.collect_routes(),
            self.config.debug_routes,
        );
        handle.spawn(async move {
            if let Err(e) = serve_http(listener, app, waiter).await {
                eprintln!("[Api Server] exited: {}", e);
            }
        });
        Ok(())
    }

    async fn run_http_async(&self, waiter: Waiter) -> Rerr {
        let addr = SocketAddr::new(self.config.listen_ip, self.config.listen_port);
        let app = build_router(
            self.make_exec_ctx(),
            self.collect_routes(),
            self.config.debug_routes,
        );
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| sys::Error::fault(format!("api bind {} failed: {}", addr, e)))?;
        serve_http(listener, app, waiter).await
    }
}

fn bind_http_on(
    handle: &tokio::runtime::Handle,
    addr: std::net::SocketAddr,
) -> sys::Ret<tokio::net::TcpListener> {
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| sys::Error::fault(format!("api bind {} failed: {}", addr, e)))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| sys::Error::fault(format!("api set nonblocking {} failed: {}", addr, e)))?;
    let _runtime = handle.enter();
    tokio::net::TcpListener::from_std(listener)
        .map_err(|e| sys::Error::fault(format!("api adopt listener {} failed: {}", addr, e)))
}

async fn serve_http(listener: tokio::net::TcpListener, app: Router, waiter: Waiter) -> Rerr {
    let addr = listener
        .local_addr()
        .map_err(|e| sys::Error::fault(format!("api local address failed: {}", e)))?;
    println!("[Api Server] listening on http://{}", addr);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        waiter.cancelled().await;
    })
    .await
    .map_err(|e| sys::Error::fault(format!("api server failed: {}", e)))
}

#[derive(Clone)]
struct RouteState {
    ctx: ApiExecCtx,
    handler: ApiHandler,
}

fn build_router(ctx: ApiExecCtx, routes: Vec<ApiRoute>, debug_routes: bool) -> Router {
    let mut app = Router::new().route("/_server_", get(|| async { "Hacash Api Server" }));
    for route in routes {
        if route.debug && !debug_routes {
            continue;
        }
        app = match (&route.method, &route.handler) {
            (ApiMethod::Get, ApiHandlerKind::Sync(handler)) => app.route(
                &route.path,
                get(route_entry_sync).with_state(RouteState {
                    ctx: ctx.clone(),
                    handler: handler.clone(),
                }),
            ),
            (ApiMethod::Post, ApiHandlerKind::Sync(handler)) => app.route(
                &route.path,
                post(route_entry_sync).with_state(RouteState {
                    ctx: ctx.clone(),
                    handler: handler.clone(),
                }),
            ),
            (ApiMethod::Get, ApiHandlerKind::Async(handler)) => app.route(
                &route.path,
                get(route_entry_async).with_state(AsyncRouteState {
                    ctx: ctx.clone(),
                    handler: handler.clone(),
                }),
            ),
            (ApiMethod::Post, ApiHandlerKind::Async(handler)) => app.route(
                &route.path,
                post(route_entry_async).with_state(AsyncRouteState {
                    ctx: ctx.clone(),
                    handler: handler.clone(),
                }),
            ),
        };
    }
    app
}

fn build_api_request(
    method: Method,
    query: std::collections::HashMap<String, String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
    peer_addr: SocketAddr,
) -> ApiRequest {
    let headers = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_owned(), v.to_owned()))
        })
        .collect();
    ApiRequest {
        query,
        headers,
        body: if method == Method::GET {
            Vec::new()
        } else {
            body.to_vec()
        },
        peer_ip: Some(peer_addr.ip()),
    }
}

async fn route_entry_sync(
    State(state): State<RouteState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    method: Method,
    Query(query): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let req = build_api_request(method, query, headers, body, peer_addr);
    // State-query handlers convert storage read failures into 503 themselves
    // (§7.4); no panic boundary exists here anymore.
    let response = (state.handler)(&state.ctx, req);
    api_response_to_axum(response)
}

async fn route_entry_async(
    State(state): State<AsyncRouteState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    method: Method,
    Query(query): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let req = build_api_request(method, query, headers, body, peer_addr);
    // Sync and async handlers share the same state-read error semantics: read
    // failures are converted to 503 inside the handler (§7.4).
    api_response_to_axum((state.handler)(state.ctx.clone(), req).await)
}

#[derive(Clone)]
struct AsyncRouteState {
    ctx: ApiExecCtx,
    handler: ApiHandlerAsync,
}

fn api_response_to_axum(resp: ApiResponse) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    for (k, v) in resp.headers {
        builder = builder.header(k, v);
    }
    builder.body(Body::from(resp.body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("response build failed"))
            .unwrap()
    })
}
