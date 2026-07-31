use std::net::{IpAddr, Ipv4Addr};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct P2PConfig {
    pub listen_ip: IpAddr,
    pub block_queue_cap: usize,
    pub boot_nodes: Vec<String>,
    #[serde(skip)]
    pub node_key: [u8; 16],
    pub node_name: String,
    pub listen_port: u16,
    #[serde(skip)]
    pub data_dir: String,
    pub find_nodes: bool,
    pub accept_nodes: bool,
    pub use_stable_nodes: bool,
    pub backbone_peers: usize,
    pub offshoot_peers: usize,
}

impl Default for P2PConfig {
    fn default() -> Self {
        Self {
            listen_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            block_queue_cap: 8,
            boot_nodes: Vec::new(),
            node_key: [0; 16],
            node_name: String::new(),
            listen_port: 3337,
            data_dir: String::new(),
            find_nodes: true,
            accept_nodes: true,
            use_stable_nodes: true,
            backbone_peers: 4,
            offshoot_peers: 200,
        }
    }
}
