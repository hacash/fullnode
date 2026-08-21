use std::net::{IpAddr, Ipv4Addr};

/// HTTP API server configuration (`[api]` section). Decoded hand-written in
/// the app composition layer (no serde).
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub enable: bool,
    pub listen_ip: IpAddr,
    pub listen_port: u16,
    pub debug_routes: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enable: false,
            listen_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            listen_port: 8082,
            debug_routes: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServerConfig;

    #[test]
    fn api_server_is_disabled_by_default() {
        assert!(!ServerConfig::default().enable);
    }
}
