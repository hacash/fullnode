//! HTTP transport: routing and ApiService dispatch only. Chain-specific routes live in the `api` crate.

mod http;

pub use http::HttpServer;
