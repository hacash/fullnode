use std::future::Future;
use std::sync::Arc;

use crate::api::{ApiExecCtx, ApiHandler, ApiHandlerAsync, ApiMethod, ApiRequest, ApiResponse};

#[derive(Clone)]
pub enum ApiHandlerKind {
    Sync(ApiHandler),
    Async(ApiHandlerAsync),
}

#[derive(Clone)]
pub struct ApiRoute {
    pub method: ApiMethod,
    pub path: String,
    pub handler: ApiHandlerKind,
    pub debug: bool,
}

impl ApiRoute {
    pub fn get(
        path: &str,
        handler: impl Fn(&ApiExecCtx, ApiRequest) -> ApiResponse + Send + Sync + 'static,
    ) -> Self {
        Self {
            method: ApiMethod::Get,
            path: path.to_owned(),
            handler: ApiHandlerKind::Sync(Arc::new(handler)),
            debug: false,
        }
    }
    pub fn post(
        path: &str,
        handler: impl Fn(&ApiExecCtx, ApiRequest) -> ApiResponse + Send + Sync + 'static,
    ) -> Self {
        Self {
            method: ApiMethod::Post,
            path: path.to_owned(),
            handler: ApiHandlerKind::Sync(Arc::new(handler)),
            debug: false,
        }
    }
    pub fn get_async<F, Fut>(path: &str, handler: F) -> Self
    where
        F: Fn(ApiExecCtx, ApiRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ApiResponse> + Send + 'static,
    {
        Self {
            method: ApiMethod::Get,
            path: path.to_owned(),
            handler: ApiHandlerKind::Async(Arc::new(move |ctx, req| Box::pin(handler(ctx, req)))),
            debug: false,
        }
    }
    pub fn debug_get(
        path: &str,
        handler: impl Fn(&ApiExecCtx, ApiRequest) -> ApiResponse + Send + Sync + 'static,
    ) -> Self {
        Self {
            method: ApiMethod::Get,
            path: Self::debug_path(path),
            handler: ApiHandlerKind::Sync(Arc::new(handler)),
            debug: true,
        }
    }
    pub fn debug_post(
        path: &str,
        handler: impl Fn(&ApiExecCtx, ApiRequest) -> ApiResponse + Send + Sync + 'static,
    ) -> Self {
        Self {
            method: ApiMethod::Post,
            path: Self::debug_path(path),
            handler: ApiHandlerKind::Sync(Arc::new(handler)),
            debug: true,
        }
    }
    fn debug_path(p: &str) -> String {
        format!("/debug/{}", p.trim_start_matches('/'))
    }
}
