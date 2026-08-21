//! VERSION/VERACK identity handshake after magic exchange; the shared STATUS
//! exchange validates genesis and starts sync.
//!
//! VERSION wire layout (fixed prefix **86** bytes + user_agent; no timestamp field):
//! ```text
//! [u16BE protocol_version]   = 2
//! [u64BE services]
//! [16B    node_key]
//! [16B    node_name]          (space-padded)
//! [u16BE  listen_port]
//! [32B    genesis_hash]
//! [u64BE  start_height]
//! [u8     relay]
//! [u8     user_agent_len]
//! [N B    user_agent]
//! [u8     custom_type_count]
//! [N B    custom_types]        (each in 101..=255, sorted, unique)
//! ```

use std::time::Duration;

use sys::{Ret, errf};

use super::codec::{read_transport_msg, write_transport_msg};
use super::msg::{MSG_VERACK, MSG_VERSION, PEER_KEY_SIZE, PROTOCOL_VERSION};
use field::Hash;

/// Whole handshake budget (VERSION+VERACK both ways).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct VersionMessage {
    pub protocol_version: u16,
    pub services: u64,
    pub node_key: [u8; 16],
    pub node_name: String,
    pub listen_port: u16,
    pub genesis_hash: [u8; 32],
    pub start_height: u64,
    pub relay: bool,
    pub user_agent: String,
    pub custom_types: Vec<u8>,
}

impl VersionMessage {
    pub fn build(
        node_key: &[u8; 16],
        node_name: &str,
        listen_port: u16,
        services_bits: u64,
        genesis_hash: &Hash,
        start_height: u64,
        user_agent: &str,
        custom_types: &[u8],
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            services: services_bits,
            node_key: *node_key,
            node_name: node_name.to_string(),
            listen_port,
            genesis_hash: genesis_hash.into_array(),
            start_height,
            relay: true,
            user_agent: user_agent.to_string(),
            custom_types: canonical_custom_types(custom_types),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let name_bytes = self.node_name.as_bytes();
        let ua_bytes = self.user_agent.as_bytes();
        // Fixed prefix: 2+8+16+16+2+32+8+1+1 = 86 bytes (no timestamp).
        let custom_types = canonical_custom_types(&self.custom_types);
        let mut out = Vec::with_capacity(87 + ua_bytes.len() + custom_types.len());
        out.extend_from_slice(&self.protocol_version.to_be_bytes());
        out.extend_from_slice(&self.services.to_be_bytes());
        out.extend_from_slice(&self.node_key);
        let mut name = name_bytes.to_vec();
        name.resize(PEER_KEY_SIZE, b' ');
        if name.len() > PEER_KEY_SIZE {
            name.truncate(PEER_KEY_SIZE);
        }
        out.extend_from_slice(&name);
        out.extend_from_slice(&self.listen_port.to_be_bytes());
        out.extend_from_slice(&self.genesis_hash);
        out.extend_from_slice(&self.start_height.to_be_bytes());
        out.push(if self.relay { 1 } else { 0 });
        let ua_len = ua_bytes.len().min(255) as u8;
        out.push(ua_len);
        out.extend_from_slice(&ua_bytes[..ua_len as usize]);
        out.push(custom_types.len() as u8);
        out.extend_from_slice(&custom_types);
        out
    }

    pub fn decode(body: &[u8]) -> Ret<Self> {
        // Fixed prefix is 86 bytes, followed by the mandatory custom count.
        if body.len() < 87 {
            return errf!("VERSION body too short: {}", body.len());
        }
        let mut off = 0;
        let protocol_version = u16::from_be_bytes([body[off], body[off + 1]]);
        off += 2;
        let services = u64::from_be_bytes(body[off..off + 8].try_into().unwrap());
        off += 8;
        let mut node_key = [0u8; 16];
        node_key.copy_from_slice(&body[off..off + 16]);
        off += 16;
        let name_raw = &body[off..off + PEER_KEY_SIZE];
        off += PEER_KEY_SIZE;
        let node_name = String::from_utf8_lossy(name_raw)
            .trim_end_matches(' ')
            .to_string();
        let listen_port = u16::from_be_bytes([body[off], body[off + 1]]);
        off += 2;
        let mut genesis_hash = [0u8; 32];
        genesis_hash.copy_from_slice(&body[off..off + 32]);
        off += 32;
        let start_height = u64::from_be_bytes(body[off..off + 8].try_into().unwrap());
        off += 8;
        let relay = body[off] != 0;
        off += 1;
        let ua_len = body[off] as usize;
        off += 1;
        if body.len() < off + ua_len + 1 {
            return errf!("VERSION user_agent truncated");
        }
        let user_agent = String::from_utf8_lossy(&body[off..off + ua_len]).to_string();
        off += ua_len;
        let custom_count = body[off] as usize;
        off += 1;
        if body.len() != off + custom_count {
            return errf!("VERSION custom type list malformed");
        }
        let custom_types = canonical_custom_types(&body[off..]);
        if custom_types.as_slice() != &body[off..] {
            return errf!("VERSION custom types must be sorted, unique, and in 101..=255");
        }
        Ok(Self {
            protocol_version,
            services,
            node_key,
            node_name,
            listen_port,
            genesis_hash,
            start_height,
            relay,
            user_agent,
            custom_types,
        })
    }

    pub fn validate_as_peer(&self) -> Ret<()> {
        if self.protocol_version != PROTOCOL_VERSION {
            return errf!(
                "unexpected protocol_version {} (want {})",
                self.protocol_version,
                PROTOCOL_VERSION
            );
        }
        Ok(())
    }

    pub fn wants_relay(&self) -> bool {
        self.relay
    }
}

#[derive(Clone, Debug)]
pub struct PeerIdentity {
    pub key: [u8; 16],
    pub name: String,
    pub listen_port: u16,
    pub services: u64,
    pub relay: bool,
    pub custom_types: Vec<u8>,
}

impl PeerIdentity {
    pub fn from_version(v: &VersionMessage) -> Self {
        Self {
            key: v.node_key,
            name: v.node_name.clone(),
            listen_port: v.listen_port,
            services: v.services,
            relay: v.wants_relay(),
            custom_types: v.custom_types.clone(),
        }
    }
}

fn canonical_custom_types(types: &[u8]) -> Vec<u8> {
    let mut out = types.to_vec();
    out.sort_unstable();
    out.dedup();
    out.retain(|ty| *ty > 100);
    out
}

pub async fn send_version(
    conn: &mut (impl tokio::io::AsyncWrite + Unpin),
    msg: &VersionMessage,
) -> sys::Rerr {
    write_transport_msg(conn, MSG_VERSION, &msg.encode()).await
}

pub async fn send_verack(conn: &mut (impl tokio::io::AsyncWrite + Unpin)) -> sys::Rerr {
    write_transport_msg(conn, MSG_VERACK, &[]).await
}

pub async fn read_version(conn: &mut (impl tokio::io::AsyncRead + Unpin)) -> Ret<VersionMessage> {
    let (ty, body) = read_transport_msg(conn).await?;
    if ty != MSG_VERSION {
        return errf!("expected VERSION, got ty={}", ty);
    }
    let msg = VersionMessage::decode(&body)?;
    msg.validate_as_peer()?;
    Ok(msg)
}

pub async fn read_verack(conn: &mut (impl tokio::io::AsyncRead + Unpin)) -> sys::Rerr {
    let (ty, body) = read_transport_msg(conn).await?;
    if ty != MSG_VERACK {
        return errf!("expected VERACK, got ty={}", ty);
    }
    if !body.is_empty() {
        return errf!("VERACK body should be empty, got {} bytes", body.len());
    }
    Ok(())
}

/// Full VERSION↔VERSION + VERACK↔VERACK exchange under a timeout.
pub async fn exchange_handshake(
    conn: &mut (impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin),
    local: &VersionMessage,
) -> Ret<VersionMessage> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        send_version(conn, local).await?;
        let peer = read_version(conn).await?;
        send_verack(conn).await?;
        read_verack(conn).await?;
        Ok(peer)
    })
    .await
    .map_err(|_| sys::Error::fault("handshake timeout".to_owned()))?
}
