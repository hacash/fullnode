//! v2 P2P transport frame codec.
//!
//! Wire format (big-endian throughout):
//! ```text
//! [u32BE length = body.len()][u8 ty][u32BE crc32c(ty+body)][body]
//! ```
//! - 9-byte header (vs v1's 4+1+2 nested).
//! - `crc32c` covers `[ty][body]` (not length, not itself).
//! - `length` counts only `body` (unlike v1 where size includes the type byte).

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use sys::{Rerr, Ret, errf};

use super::msg::{P2P_MAGIC_V1, P2P_MAGIC_V2, P2P_MSG_DATA_MAX_SIZE, V2_FRAME_HEADER_SIZE};

// ===================================================================
// crc32c (Castagnoli)
// ===================================================================

const fn build_crc32c_table() -> [u32; 256] {
    let poly: u32 = 0x82F6_3B78;
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ poly;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

const CRC32C_TABLE: [u32; 256] = build_crc32c_table();

#[inline]
fn crc32c_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc = CRC32C_TABLE[((crc as u8) ^ b) as usize] ^ (crc >> 8);
    }
    crc
}

/// CRC-32C over `[ty BE][body]` without heap allocation.
pub fn crc32c_ty_body(ty: u8, body: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF;
    crc = crc32c_update(crc, &[ty]);
    crc = crc32c_update(crc, body);
    crc ^ 0xFFFF_FFFF
}

// ===================================================================
// I/O helpers
// ===================================================================

pub async fn write_all(conn: &mut (impl AsyncWrite + Unpin), data: &[u8]) -> Rerr {
    conn.write_all(data)
        .await
        .map_err(|e| sys::Error::fault(format!("v2 p2p write failed: {}", e)))
}

pub async fn read_exact(conn: &mut (impl AsyncRead + Unpin), len: usize) -> Ret<Vec<u8>> {
    let mut buf = vec![0u8; len];
    conn.read_exact(&mut buf)
        .await
        .map_err(|e| sys::Error::fault(format!("v2 p2p read failed: {}", e)))?;
    Ok(buf)
}

// ===================================================================
// v2 frame encode / decode
// ===================================================================

pub fn create_transport_frame(ty: u8, body: &[u8]) -> Ret<Vec<u8>> {
    if body.len() > P2P_MSG_DATA_MAX_SIZE {
        return errf!("v2 message {} too large: {}", ty, body.len());
    }
    let mut out = Vec::with_capacity(V2_FRAME_HEADER_SIZE + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.push(ty);
    let crc = crc32c_ty_body(ty, body);
    out.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

pub async fn read_transport_msg(conn: &mut (impl AsyncRead + Unpin)) -> Ret<(u8, Vec<u8>)> {
    let header = read_exact(conn, V2_FRAME_HEADER_SIZE).await?;
    let length = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
    let ty = header[4];
    let expected_crc = u32::from_be_bytes(header[5..9].try_into().unwrap());
    if length > P2P_MSG_DATA_MAX_SIZE {
        return errf!("v2 frame too large: ty={} len={}", ty, length);
    }
    let body = read_exact(conn, length).await?;
    let actual_crc = crc32c_ty_body(ty, &body);
    if actual_crc != expected_crc {
        return errf!(
            "v2 checksum mismatch: ty={} expected={:#010x} got={:#010x}",
            ty,
            expected_crc,
            actual_crc
        );
    }
    Ok((ty, body))
}

pub async fn write_transport_msg(
    conn: &mut (impl AsyncWrite + Unpin),
    ty: u8,
    body: &[u8],
) -> Rerr {
    let frame = create_transport_frame(ty, body)?;
    write_all(conn, &frame).await
}

// ===================================================================
// v2 magic handshake
// ===================================================================

/// Dialer: write v2 magic, read peer reply. `true` = v2 peer, `false` = v1.
pub async fn dial_magic_exchange(conn: &mut (impl AsyncRead + AsyncWrite + Unpin)) -> Ret<bool> {
    let magic = P2P_MAGIC_V2.to_be_bytes();
    write_all(conn, &magic).await?;
    let got = read_exact(conn, 4).await?;
    if got == magic {
        Ok(true)
    } else if got == P2P_MAGIC_V1.to_be_bytes() {
        Ok(false)
    } else {
        errf!("dial magic exchange: unexpected peer response")
    }
}

/// Accept: read 4 bytes first. `true` = v2, `false` = v1.
pub async fn accept_read_magic(conn: &mut (impl AsyncRead + Unpin)) -> Ret<bool> {
    let got = read_exact(conn, 4).await?;
    if got == P2P_MAGIC_V2.to_be_bytes() {
        Ok(true)
    } else if got == P2P_MAGIC_V1.to_be_bytes() {
        Ok(false)
    } else {
        errf!("accept magic: unknown value")
    }
}

pub async fn write_v2_magic(conn: &mut (impl AsyncWrite + Unpin)) -> Rerr {
    write_all(conn, &P2P_MAGIC_V2.to_be_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::accept_read_magic;
    use crate::p2p::msg::{P2P_MAGIC_V1, P2P_MAGIC_V2};

    #[tokio::test]
    async fn accept_magic_distinguishes_v1_and_v2() {
        let v1_bytes = P2P_MAGIC_V1.to_be_bytes();
        let v2_bytes = P2P_MAGIC_V2.to_be_bytes();
        let mut v1 = v1_bytes.as_slice();
        let mut v2 = v2_bytes.as_slice();

        assert!(!accept_read_magic(&mut v1).await.unwrap());
        assert!(accept_read_magic(&mut v2).await.unwrap());
    }

    #[tokio::test]
    async fn accept_magic_rejects_non_p2p_prefixes() {
        let mut http = b"GET ".as_slice();
        assert!(accept_read_magic(&mut http).await.is_err());
    }
}
