//! Public-reachability probe via MSG_CHECK_PUBLIC.

use std::net::SocketAddr;
use std::time::Duration;

use sys::Ret;

use crate::p2p::codec::{dial_magic_exchange, read_transport_msg, write_transport_msg};
use crate::p2p::msg::MSG_CHECK_PUBLIC;
use crate::p2p::msg::PEER_KEY_SIZE;

/// Short-connection public probe via MSG_CHECK_PUBLIC.
pub async fn probe_public_id(addr: SocketAddr, expect_key: &[u8; 16]) -> Ret<bool> {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| sys::Error::fault(format!("public-check connect timeout {}", addr)))?
            .map_err(|e| sys::Error::fault(format!("public-check connect {}: {}", addr, e)))?;
    tokio::time::timeout(Duration::from_secs(3), dial_magic_exchange(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("public-check magic timeout".to_owned()))??;
    tokio::time::timeout(
        Duration::from_secs(3),
        write_transport_msg(&mut stream, MSG_CHECK_PUBLIC, &[]),
    )
    .await
    .map_err(|_| sys::Error::fault("public-check request timeout".to_owned()))??;
    let (ty, body) = tokio::time::timeout(Duration::from_secs(3), read_transport_msg(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("public-check read timeout".to_owned()))??;
    if ty != MSG_CHECK_PUBLIC || body.len() != PEER_KEY_SIZE {
        return sys::errf!("public-check bad response");
    }
    if body.as_slice() != expect_key.as_slice() {
        return sys::errf!("peer id not match");
    }
    Ok(true)
}

/// After inbound VERSION: probe every announced listen port.
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

/// Serve one-shot MSG_CHECK_PUBLIC on an already magic-exchanged stream.
pub async fn serve_check_public(
    stream: &mut tokio::net::TcpStream,
    node_key: &[u8; 16],
) -> sys::Rerr {
    write_transport_msg(stream, MSG_CHECK_PUBLIC, node_key).await
}
