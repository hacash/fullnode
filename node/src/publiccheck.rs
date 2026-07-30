//! Public-reachability probe (v1 MSG 201 / v2 MSG_CHECK_PUBLIC).

use std::net::SocketAddr;
use std::time::Duration;

use sys::Ret;

use crate::p2p::codec::{
    dial_magic_exchange, read_transport_msg, write_transport_msg as v2_write, write_v2_magic,
};
use crate::p2p::legacy::write_all;
use crate::p2p::legacy::{read_exact, tcp_check_handshake, write_transport_msg as v1_write};
use crate::p2p::msg::v2::MSG_CHECK_PUBLIC;
use crate::p2p::msg::{P2P_MAGIC_V1, PEER_KEY_SIZE, V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK};

/// Dial `{ip}:{listen_port}` with v1 handshake; Ok(true) if key matches.
pub async fn probe_public_id_v1(addr: SocketAddr, expect_key: &[u8; 16]) -> Ret<bool> {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| sys::Error::fault(format!("public-check connect timeout {}", addr)))?
            .map_err(|e| sys::Error::fault(format!("public-check connect {}: {}", addr, e)))?;
    tcp_check_handshake(&mut stream).await?;
    v1_write(&mut stream, V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK, &[]).await?;
    let key = read_exact(&mut stream, PEER_KEY_SIZE).await?;
    if key.as_slice() != expect_key.as_slice() {
        return Err(sys::Error::fault("peer id not match".to_owned()));
    }
    Ok(true)
}

/// v2 short-connection public probe via MSG_CHECK_PUBLIC.
pub async fn probe_public_id_v2(addr: SocketAddr, expect_key: &[u8; 16]) -> Ret<bool> {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| sys::Error::fault(format!("v2 public-check connect timeout {}", addr)))?
            .map_err(|e| sys::Error::fault(format!("v2 public-check connect {}: {}", addr, e)))?;
    // Prefer v2 magic; if peer is v1, fall back.
    let magic = crate::p2p::msg::P2P_MAGIC_V2.to_be_bytes();
    crate::p2p::codec::write_all(&mut stream, &magic).await?;
    let got = crate::p2p::codec::read_exact(&mut stream, 4).await?;
    if got == magic {
        v2_write(&mut stream, MSG_CHECK_PUBLIC, &[]).await?;
        let (ty, body) =
            tokio::time::timeout(Duration::from_secs(3), read_transport_msg(&mut stream))
                .await
                .map_err(|_| sys::Error::fault("v2 public-check read timeout".to_owned()))??;
        if ty != MSG_CHECK_PUBLIC || body.len() != PEER_KEY_SIZE {
            return Err(sys::Error::fault("v2 public-check bad response".to_owned()));
        }
        if body.as_slice() != expect_key.as_slice() {
            return Err(sys::Error::fault("peer id not match".to_owned()));
        }
        return Ok(true);
    }
    if got == P2P_MAGIC_V1.to_be_bytes() {
        // Peer is v1 — complete v1 magic and probe with 201.
        write_all(&mut stream, &P2P_MAGIC_V1.to_be_bytes()).await?;
        v1_write(&mut stream, V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK, &[]).await?;
        let key = read_exact(&mut stream, PEER_KEY_SIZE).await?;
        if key.as_slice() != expect_key.as_slice() {
            return Err(sys::Error::fault("peer id not match".to_owned()));
        }
        return Ok(true);
    }
    Err(sys::Error::fault(
        "public-check unexpected magic".to_owned(),
    ))
}

pub async fn probe_public_id(addr: SocketAddr, expect_key: &[u8; 16]) -> Ret<bool> {
    // Try v2 first (next peers), then v1.
    match probe_public_id_v2(addr, expect_key).await {
        Ok(v) => Ok(v),
        Err(_) => probe_public_id_v1(addr, expect_key).await,
    }
}

/// After receiving REPORT_PEER on inbound: optional callback probe.
pub async fn maybe_mark_public_from_report(
    peer_addr: SocketAddr,
    listen_port: u16,
    peer_key: &[u8; 16],
) -> (bool, SocketAddr) {
    let mut addr = peer_addr;
    let mut is_public = false;
    if listen_port > 0 && !peer_addr.ip().is_loopback() {
        let mut probe = peer_addr;
        probe.set_port(listen_port);
        if let Ok(true) = probe_public_id(probe, peer_key).await {
            is_public = true;
            addr.set_port(listen_port);
        }
    }
    (is_public, addr)
}

/// After v2 VERSION claiming NODE_PUBLIC: verify with probe when listen_port set.
pub async fn maybe_mark_public_from_version(
    peer_addr: SocketAddr,
    listen_port: u16,
    peer_key: &[u8; 16],
    claims_public: bool,
) -> (bool, SocketAddr) {
    if !claims_public || peer_addr.ip().is_loopback() {
        return (false, peer_addr);
    }
    if listen_port == 0 {
        // No listen port to probe — treat as non-public for backbone purposes.
        return (false, peer_addr);
    }
    let mut probe = peer_addr;
    probe.set_port(listen_port);
    if let Ok(true) = probe_public_id(probe, peer_key).await {
        let mut addr = peer_addr;
        addr.set_port(listen_port);
        (true, addr)
    } else {
        (false, peer_addr)
    }
}

/// ANSWER_PEER path (we dialed them): public iff not loopback.
pub fn public_from_answer(peer_addr: SocketAddr) -> bool {
    !peer_addr.ip().is_loopback()
}

/// Serve one-shot MSG_CHECK_PUBLIC on an already magic-exchanged v2 stream.
pub async fn serve_check_public_v2(
    stream: &mut tokio::net::TcpStream,
    node_key: &[u8; 16],
) -> sys::Rerr {
    v2_write(stream, MSG_CHECK_PUBLIC, node_key).await
}

#[allow(dead_code)]
pub async fn write_v2_magic_reply(stream: &mut tokio::net::TcpStream) -> sys::Rerr {
    write_v2_magic(stream).await
}

#[allow(dead_code)]
pub async fn _dial_magic(stream: &mut tokio::net::TcpStream) -> Ret<bool> {
    dial_magic_exchange(stream).await
}
