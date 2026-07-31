//! find_nodes + GETADDR/ADDR discovery.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use sys::Rerr;

use crate::P2PNode;
use crate::p2p::codec::{
    dial_magic_exchange, read_transport_msg as read_v2_msg, write_transport_msg as write_v2_msg,
};
use crate::p2p::legacy::{tcp_check_handshake, write_all, write_transport_msg};
use crate::p2p::msg::V1_MSG_REQUEST_NEAREST_PUBLIC_NODES;
use crate::p2p::msg::v2::{MSG_ADDR, MSG_GETADDR};
use crate::p2p::peer::{ProtocolVersion, RemotePeer};
use crate::topology::{PeerKey, insert_nearest_key};

const V1_ENTRY_SIZE: usize = 4 + 2 + 16;

// ---- v1 MSG 202 wire ----

pub fn serialize_public_nodes(peers: &[Arc<RemotePeer>], _max: usize) -> Vec<u8> {
    let mut body = Vec::new();
    let mut count = 0u8;
    for p in peers {
        if count == 200 {
            break;
        }
        if !p.is_public() || p.addr.ip().is_loopback() {
            continue;
        }
        let IpAddr::V4(ip) = p.addr.ip() else {
            continue;
        };
        body.extend_from_slice(&ip.octets());
        body.extend_from_slice(&p.addr.port().to_be_bytes());
        body.extend_from_slice(&p.node_key);
        count += 1;
    }
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(count);
    out.extend_from_slice(&body);
    out
}

pub fn parse_public_nodes(bts: &[u8]) -> Vec<(PeerKey, SocketAddr)> {
    let num = bts.len() / V1_ENTRY_SIZE;
    let mut out = Vec::with_capacity(num);
    for i in 0..num {
        let one = &bts[i * V1_ENTRY_SIZE..(i + 1) * V1_ENTRY_SIZE];
        let ip: [u8; 4] = one[0..4].try_into().unwrap();
        let port = u16::from_be_bytes([one[4], one[5]]);
        let mut key = [0u8; 16];
        key.copy_from_slice(&one[6..22]);
        let ipaddr = IpAddr::from(ip);
        if ipaddr.is_loopback() {
            continue;
        }
        out.push((key, SocketAddr::new(ipaddr, port)));
    }
    out
}

// ---- v2 GETADDR/ADDR wire (IPv4 + IPv6) ----
// ADDR body:
//   [u16BE count]
//   repeated:
//     [u8 fam] 4 | 6
//     [ip bytes]
//     [u16BE port]
//     [16B key]

pub fn encode_addr_v2(peers: &[Arc<RemotePeer>], max: usize) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut count = 0u16;
    for p in peers {
        if count as usize >= max || count >= 200 {
            break;
        }
        if !p.is_public() || p.addr.ip().is_loopback() {
            continue;
        }
        match p.addr.ip() {
            IpAddr::V4(ip) => {
                entries.push(4u8);
                entries.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                entries.push(6u8);
                entries.extend_from_slice(&ip.octets());
            }
        }
        entries.extend_from_slice(&p.addr.port().to_be_bytes());
        entries.extend_from_slice(&p.node_key);
        count += 1;
    }
    let mut out = Vec::with_capacity(2 + entries.len());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&entries);
    out
}

pub fn decode_addr_v2(body: &[u8]) -> sys::Ret<Vec<(PeerKey, SocketAddr)>> {
    if body.len() < 2 {
        return sys::errf!("ADDR body too short");
    }
    let count = u16::from_be_bytes([body[0], body[1]]) as usize;
    if count > 200 {
        return sys::errf!("ADDR count too large: {}", count);
    }
    let mut off = 2;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if off >= body.len() {
            return sys::errf!("ADDR truncated");
        }
        let fam = body[off];
        off += 1;
        let ip = match fam {
            4 => {
                if off + 4 > body.len() {
                    return sys::errf!("ADDR ipv4 truncated");
                }
                let octets: [u8; 4] = body[off..off + 4].try_into().unwrap();
                off += 4;
                IpAddr::from(octets)
            }
            6 => {
                if off + 16 > body.len() {
                    return sys::errf!("ADDR ipv6 truncated");
                }
                let octets: [u8; 16] = body[off..off + 16].try_into().unwrap();
                off += 16;
                IpAddr::from(octets)
            }
            _ => return sys::errf!("ADDR bad fam {}", fam),
        };
        if off + 2 + 16 > body.len() {
            return sys::errf!("ADDR port/key truncated");
        }
        let port = u16::from_be_bytes([body[off], body[off + 1]]);
        off += 2;
        let mut key = [0u8; 16];
        key.copy_from_slice(&body[off..off + 16]);
        off += 16;
        if ip.is_loopback() {
            continue;
        }
        out.push((key, SocketAddr::new(ip, port)));
    }
    if off != body.len() {
        return sys::errf!("ADDR trailing bytes: {}", body.len() - off);
    }
    Ok(out)
}

async fn request_public_nodes_v2(
    addr: SocketAddr,
    datas: &mut HashMap<PeerKey, SocketAddr>,
) -> Rerr {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| sys::Error::fault(format!("find_nodes connect timeout {}", addr)))?
            .map_err(|e| sys::Error::fault(format!("find_nodes connect {}: {}", addr, e)))?;
    let is_v2 = tokio::time::timeout(Duration::from_secs(5), dial_magic_exchange(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("find_nodes v2 magic timeout".to_owned()))??;
    if !is_v2 {
        return sys::errf!("find_nodes peer {} no longer speaks v2", addr);
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        write_v2_msg(&mut stream, MSG_GETADDR, &[]),
    )
    .await
    .map_err(|_| sys::Error::fault("find_nodes GETADDR write timeout".to_owned()))??;
    let (ty, body) = tokio::time::timeout(Duration::from_secs(5), read_v2_msg(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("find_nodes ADDR read timeout".to_owned()))??;
    if ty != MSG_ADDR {
        return sys::errf!("find_nodes expected ADDR, got {}", ty);
    }
    for (key, found_addr) in decode_addr_v2(&body)? {
        datas.insert(key, found_addr);
    }
    Ok(())
}

async fn request_public_nodes_v1(
    addr: SocketAddr,
    datas: &mut HashMap<PeerKey, SocketAddr>,
) -> Rerr {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| sys::Error::fault(format!("find_nodes connect timeout {}", addr)))?
            .map_err(|e| sys::Error::fault(format!("find_nodes connect {}: {}", addr, e)))?;
    tokio::time::timeout(Duration::from_secs(5), tcp_check_handshake(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("find_nodes v1 magic timeout".to_owned()))??;
    write_transport_msg(&mut stream, V1_MSG_REQUEST_NEAREST_PUBLIC_NODES, &[]).await?;
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        use tokio::io::AsyncReadExt;
        stream.read_to_end(&mut buf).await
    })
    .await
    .map_err(|_| sys::Error::fault("find_nodes read timeout".to_owned()))?
    .map_err(|e| sys::Error::fault(format!("find_nodes read: {}", e)))?;
    if buf.is_empty() {
        return sys::errf!("find_nodes data empty");
    }
    let num = buf[0] as usize;
    if num < 1 {
        return Ok(());
    }
    if num * V1_ENTRY_SIZE != buf.len() - 1 {
        return sys::errf!("find_nodes data size error");
    }
    for (k, a) in parse_public_nodes(&buf[1..]) {
        datas.insert(k, a);
    }
    Ok(())
}

impl P2PNode {
    /// Query backbone peers and connect DHT-nearest results, matching fullnodedev.
    pub async fn find_nodes(self: &Arc<Self>) {
        if !self.config.find_nodes {
            return;
        }
        print!("[P2P] Searching nodes...");
        let backbones = self.peertable.backbones().await;
        if backbones.is_empty() {
            println!("not connected any nodes.");
            return;
        }
        let mut allfind: HashMap<PeerKey, SocketAddr> = HashMap::new();
        let mut excluded = vec![self.config.node_key];
        for p in &backbones {
            excluded.push(p.node_key);
            match p.protocol_version() {
                ProtocolVersion::V2 => {
                    if let Err(e) = request_public_nodes_v2(p.addr, &mut allfind).await {
                        println!("request public nodes error: {}", e);
                    }
                }
                ProtocolVersion::V1 => {
                    if let Err(e) = request_public_nodes_v1(p.addr, &mut allfind).await {
                        println!("request public nodes error: {}", e);
                    }
                }
            }
        }
        for key in &excluded {
            allfind.remove(key);
        }
        if allfind.is_empty() {
            println!("not find any new nodes.");
            return;
        }
        let first = excluded[1];
        let least = *excluded.last().unwrap_or(&self.config.node_key);
        let mut nearest_keys: Vec<PeerKey> = Vec::new();
        let mut nearest_addrs: Vec<SocketAddr> = Vec::new();
        for (k, addr) in &allfind {
            if insert_nearest_key(&mut nearest_keys, &self.config.node_key, &least, k) {
                nearest_addrs.push(*addr);
            }
        }
        println!("find {} new nodes.", allfind.len());
        self.dial_nearest_publics(nearest_addrs, first).await;
    }

    /// Dial DHT-nearest discovered publics (mainnet find_nodes behavior).
    async fn dial_nearest_publics(
        self: &Arc<Self>,
        addrs: Vec<SocketAddr>,
        first_backbone: PeerKey,
    ) {
        if addrs.is_empty() {
            let publen = self.peertable.backbones().await.len();
            println!("connected {} public nodes, not find any nearest.", publen);
            return;
        }
        println!("find {} nearest nodes, try connect...", addrs.len());
        let mut connected = 0usize;
        for addr in addrs {
            match self.connect_addr(addr).await {
                Ok(()) => connected += 1,
                Err(e) => {
                    println!("failed connect to {}, {}.", addr, e);
                    continue;
                }
            }
            if connected >= 16 {
                break;
            }
            let still = self
                .peertable
                .backbones()
                .await
                .iter()
                .any(|p| p.node_key == first_backbone);
            if !still {
                break;
            }
        }
    }

    pub async fn serve_nearest_public_nodes(&self, stream: &mut tokio::net::TcpStream) -> Rerr {
        let publics = self.peertable.publics().await;
        let body = serialize_public_nodes(&publics, 100);
        write_all(stream, &body).await
    }

    pub(crate) fn handle_getaddr(&self, peer: &Arc<RemotePeer>) -> Rerr {
        let publics = self
            .peertable
            .values_snapshot()
            .into_iter()
            .filter(|candidate| candidate.is_public() && !candidate.addr.ip().is_loopback())
            .collect::<Vec<_>>();
        let body = encode_addr_v2(&publics, 100);
        peer.send_v2_transport(MSG_ADDR, &body)
    }

    pub(crate) async fn serve_getaddr_v2(&self, stream: &mut tokio::net::TcpStream) -> Rerr {
        let publics = self.peertable.publics().await;
        let body = encode_addr_v2(&publics, 100);
        write_v2_msg(stream, MSG_ADDR, &body).await
    }
}

#[cfg(test)]
mod tests {
    use super::decode_addr_v2;

    #[test]
    fn addr_rejects_more_than_two_hundred_entries() {
        assert!(decode_addr_v2(&201u16.to_be_bytes()).is_err());
    }

    #[test]
    fn addr_rejects_trailing_bytes() {
        let mut body = 0u16.to_be_bytes().to_vec();
        body.push(0);
        assert!(decode_addr_v2(&body).is_err());
    }
}
