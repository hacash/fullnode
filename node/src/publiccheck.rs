//! Public-reachability probe (v1 MSG 201 / v2 MSG_CHECK_PUBLIC).

use std::net::SocketAddr;
use std::time::Duration;

use sys::Ret;

use crate::p2p::codec::{
    dial_magic_exchange, read_transport_msg, write_transport_msg as v2_write, write_v2_magic,
};
use crate::p2p::legacy::{read_exact, tcp_check_handshake, write_transport_msg as v1_write};
use crate::p2p::msg::v2::MSG_CHECK_PUBLIC;
use crate::p2p::msg::{PEER_KEY_SIZE, V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK};

async fn probe_public_id_v1(addr: SocketAddr, expect_key: &[u8; 16]) -> Ret<bool> {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| sys::Error::fault(format!("public-check connect timeout {}", addr)))?
            .map_err(|e| sys::Error::fault(format!("public-check connect {}: {}", addr, e)))?;
    tokio::time::timeout(Duration::from_secs(3), tcp_check_handshake(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("public-check v1 magic timeout".to_owned()))??;
    tokio::time::timeout(
        Duration::from_secs(3),
        v1_write(&mut stream, V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK, &[]),
    )
    .await
    .map_err(|_| sys::Error::fault("public-check v1 request timeout".to_owned()))??;
    let key = tokio::time::timeout(
        Duration::from_secs(3),
        read_exact(&mut stream, PEER_KEY_SIZE),
    )
    .await
    .map_err(|_| sys::Error::fault("public-check v1 response timeout".to_owned()))??;
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
    let is_v2 = tokio::time::timeout(Duration::from_secs(3), dial_magic_exchange(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("public-check magic timeout".to_owned()))??;
    if !is_v2 {
        // A dev v1 acceptor has already rejected the v2 magic it read. Start
        // a fresh connection only after this explicit v1 identification.
        drop(stream);
        return probe_public_id_v1(addr, expect_key).await;
    }
    tokio::time::timeout(
        Duration::from_secs(3),
        v2_write(&mut stream, MSG_CHECK_PUBLIC, &[]),
    )
    .await
    .map_err(|_| sys::Error::fault("v2 public-check request timeout".to_owned()))??;
    let (ty, body) = tokio::time::timeout(Duration::from_secs(3), read_transport_msg(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("v2 public-check read timeout".to_owned()))??;
    if ty != MSG_CHECK_PUBLIC || body.len() != PEER_KEY_SIZE {
        return Err(sys::Error::fault("v2 public-check bad response".to_owned()));
    }
    if body.as_slice() != expect_key.as_slice() {
        return Err(sys::Error::fault("peer id not match".to_owned()));
    }
    Ok(true)
}

pub async fn probe_public_id(addr: SocketAddr, expect_key: &[u8; 16]) -> Ret<bool> {
    // The v2 probe already handles an explicit v1 magic reply on the same
    // connection. Other v2 errors are real failures and must not trigger an
    // unsolicited second connection with a different protocol.
    probe_public_id_v2(addr, expect_key).await
}

/// After receiving REPORT_PEER on inbound: optional callback probe.
pub async fn maybe_mark_public_from_report(
    peer_addr: SocketAddr,
    listen_port: u16,
    peer_key: &[u8; 16],
) -> (bool, SocketAddr) {
    let mut addr = peer_addr;
    let mut is_public = false;
    if listen_port > 0 {
        let mut probe = peer_addr;
        probe.set_port(listen_port);
        if let Ok(true) = probe_public_id_v1(probe, peer_key).await
            && !peer_addr.ip().is_loopback()
        {
            is_public = true;
            addr.set_port(listen_port);
        }
    }
    (is_public, addr)
}

/// After inbound v2 VERSION: probe every announced listen port, matching dev REPORT_PEER.
pub async fn maybe_mark_public_from_version(
    peer_addr: SocketAddr,
    listen_port: u16,
    peer_key: &[u8; 16],
) -> (bool, SocketAddr) {
    if listen_port == 0 {
        // No listen port to probe — treat as non-public for backbone purposes.
        return (false, peer_addr);
    }
    let mut probe = peer_addr;
    probe.set_port(listen_port);
    if let Ok(true) = probe_public_id(probe, peer_key).await
        && !peer_addr.ip().is_loopback()
    {
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
