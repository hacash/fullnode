//! Legacy v1 P2P protocol (mainnet-compatible with `fullnodedev`).
//!
//! Wire format:
//! - TCP magic handshake: 4-byte `P2P_MAGIC_V1` exchanged both directions.
//! - Transport frame: `[u32BE size=1+body.len][u8 ty][body]`.
//! - Two-tier dispatch: `MSG_CUSTOMER(0xFF)` wraps `[u16BE app_ty][body]`.
//! - Identity handshake: REPORT_PEER / ANSWER_PEER.
//!
//! This module is the self-contained v1 codepath. It is kept alongside v2
//! during the dual-protocol transition (Phase 0/1) and will be deleted in
//! Phase 3 once the network has migrated.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use sys::{Rerr, Ret, errf};

use super::msg::{P2P_MAGIC_V1, P2P_MSG_DATA_MAX_SIZE, PEER_KEY_SIZE};

// ===================================================================
// v1 transport-layer message kinds (u8)
// ===================================================================

pub const MSG_REPORT_PEER: u8 = 1;
pub const MSG_ANSWER_PEER: u8 = 2;
pub const MSG_PING: u8 = 3;
pub const MSG_PONG: u8 = 4;
pub const MSG_REMIND_ME_IS_PUBLIC: u8 = 151;
pub const MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK: u8 = 201;
pub const MSG_REQUEST_NEAREST_PUBLIC_NODES: u8 = 202;
pub const MSG_CLOSE: u8 = 254;
pub const MSG_CUSTOMER: u8 = 255;

// Application-layer kinds (u16 inside MSG_CUSTOMER) are the shared core
// constants in `super::msg` (MSG_REQ_STATUS … MSG_BLOCK_DISCOVER). Do not
// duplicate them here — callers always use the shared names.

/// Encode a v1 transport frame: `[u32BE (1+body.len())][u8 ty][body]`.
pub fn create_transport_frame(ty: u8, body: &[u8]) -> Ret<Vec<u8>> {
    if body.len() > P2P_MSG_DATA_MAX_SIZE.saturating_sub(1) {
        return errf!("v1 p2p message {} too large: {}", ty, body.len());
    }
    let size = (1 + body.len()) as u32;
    let mut out = Vec::with_capacity(4 + size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.push(ty);
    out.extend_from_slice(body);
    Ok(out)
}

/// Wrap an application message for `MSG_CUSTOMER`: `[u16BE app_ty][body]`.
pub fn encode_customer(app_ty: u16, body: &[u8]) -> Ret<Vec<u8>> {
    if body.len() + 2 > P2P_MSG_DATA_MAX_SIZE.saturating_sub(1) {
        return errf!("v1 customer message {} too large: {}", app_ty, body.len());
    }
    let mut inner = Vec::with_capacity(2 + body.len());
    inner.extend_from_slice(&app_ty.to_be_bytes());
    inner.extend_from_slice(body);
    create_transport_frame(MSG_CUSTOMER, &inner)
}

/// Parse CUSTOMER body into `(app_ty, app_body)`.
pub fn decode_customer(body: &[u8]) -> Ret<(u16, Vec<u8>)> {
    if body.len() < 2 {
        return errf!("v1 customer body too short: {}", body.len());
    }
    let app_ty = u16::from_be_bytes([body[0], body[1]]);
    Ok((app_ty, body[2..].to_vec()))
}

pub async fn write_all(conn: &mut (impl AsyncWrite + Unpin), data: &[u8]) -> Rerr {
    conn.write_all(data)
        .await
        .map_err(|e| sys::Error::fault(format!("v1 p2p write failed: {}", e)))
}

pub async fn write_transport_msg(
    conn: &mut (impl AsyncWrite + Unpin),
    ty: u8,
    body: &[u8],
) -> Rerr {
    let frame = create_transport_frame(ty, body)?;
    write_all(conn, &frame).await
}

pub async fn read_exact(conn: &mut (impl AsyncRead + Unpin), len: usize) -> Ret<Vec<u8>> {
    let mut buf = vec![0u8; len];
    conn.read_exact(&mut buf)
        .await
        .map_err(|e| sys::Error::fault(format!("v1 p2p read failed: {}", e)))?;
    Ok(buf)
}

/// Read one v1 transport frame; returns `(ty, body)`.
pub async fn read_transport_msg(conn: &mut (impl AsyncRead + Unpin)) -> Ret<(u8, Vec<u8>)> {
    let size_bytes = read_exact(conn, 4).await?;
    let size = u32::from_be_bytes(size_bytes.try_into().unwrap());
    if size < 1 || size as usize > P2P_MSG_DATA_MAX_SIZE {
        return errf!("v1 transport size invalid: {}", size);
    }
    let ty_body = read_exact(conn, size as usize).await?;
    Ok((ty_body[0], ty_body[1..].to_vec()))
}

/// Bidirectional v1 magic handshake (write then read).
pub async fn tcp_check_handshake(conn: &mut (impl AsyncRead + AsyncWrite + Unpin)) -> Rerr {
    let magic = P2P_MAGIC_V1.to_be_bytes();
    write_all(conn, &magic).await?;
    let got = read_exact(conn, 4).await?;
    if got != magic {
        return errf!("v1 magic mismatch");
    }
    Ok(())
}

// ===================================================================
// v1 identity handshake (REPORT_PEER / ANSWER_PEER)
// ===================================================================

/// Build the 36-byte nodeinfo blob used in REPORT_PEER (and stripped for ANSWER).
///
/// Layout: `[0..2)=0, [2..4)=listen_port BE, [4..20)=node_key, [20..36)=name`.
pub fn build_node_info(node_key: &[u8; 16], node_name: &str, listen_port: u16) -> Vec<u8> {
    let mut info = vec![0u8; 2 + 2 + PEER_KEY_SIZE * 2];
    info[2..4].copy_from_slice(&listen_port.to_be_bytes());
    info[4..20].copy_from_slice(node_key);
    let mut name = node_name.as_bytes().to_vec();
    name.resize(PEER_KEY_SIZE, b' ');
    if name.len() > PEER_KEY_SIZE {
        name.truncate(PEER_KEY_SIZE);
    }
    info[20..36].copy_from_slice(&name);
    info
}

/// Key+name (32B) for ANSWER_PEER body - strips the 4-byte prefix from full nodeinfo.
pub fn node_info_key_name(node_info: &[u8]) -> Vec<u8> {
    if node_info.len() > PEER_KEY_SIZE * 2 {
        node_info[4..].to_vec()
    } else {
        node_info.to_vec()
    }
}

#[derive(Clone, Debug)]
pub struct PeerIdentity {
    pub key: [u8; 16],
    pub name: String,
    pub listen_port: u16,
}

/// Parse REPORT_PEER body (expects >= 36 bytes with port prefix).
pub fn parse_report_peer(body: &[u8]) -> sys::Ret<PeerIdentity> {
    if body.len() < 4 + PEER_KEY_SIZE * 2 {
        return sys::errf!("REPORT_PEER body too short: {}", body.len());
    }
    let listen_port = u16::from_be_bytes([body[2], body[3]]);
    parse_key_name(&body[4..], listen_port)
}

/// Parse ANSWER_PEER body (expects >= 32 bytes key+name).
pub fn parse_answer_peer(body: &[u8]) -> sys::Ret<PeerIdentity> {
    parse_key_name(body, 0)
}

fn parse_key_name(idname: &[u8], listen_port: u16) -> sys::Ret<PeerIdentity> {
    if idname.len() < PEER_KEY_SIZE * 2 {
        return sys::errf!("peer identity too short: {}", idname.len());
    }
    let mut key = [0u8; 16];
    key.copy_from_slice(&idname[..PEER_KEY_SIZE]);
    let name_raw = &idname[PEER_KEY_SIZE..PEER_KEY_SIZE * 2];
    let name = String::from_utf8_lossy(name_raw)
        .trim_end_matches(' ')
        .to_string();
    Ok(PeerIdentity {
        key,
        name,
        listen_port,
    })
}
