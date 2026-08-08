//! Pipelined sync wire codecs (GET_BLOCKS / BLOCKS).

use sys::{Ret, errf};

/// Default download window (in-flight GET_BLOCKS).
pub const SYNC_WINDOW: usize = 3;
/// Default max blocks per response.
pub const DEFAULT_MAX_BLOCKS: u32 = 2_000;
/// Default max payload bytes per response (~4 MiB).
pub const DEFAULT_MAX_BYTES: u32 = 4 * 1024 * 1024;
/// BLOCKS fixed header size.
pub const BLOCKS_HEADER_SIZE: usize = 44;
pub const BLOCKS_HDR_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetBlocks {
    pub request_id: u64,
    pub start_height: u64,
    pub max_blocks: u32,
    pub max_bytes: u32,
}

impl GetBlocks {
    pub fn new(request_id: u64, start_height: u64) -> Self {
        Self {
            request_id,
            start_height,
            max_blocks: DEFAULT_MAX_BLOCKS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        out.extend_from_slice(&self.request_id.to_be_bytes());
        out.extend_from_slice(&self.start_height.to_be_bytes());
        out.extend_from_slice(&self.max_blocks.to_be_bytes());
        out.extend_from_slice(&self.max_bytes.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Ret<Self> {
        if body.len() != 24 {
            return errf!("GET_BLOCKS body length invalid: {}", body.len());
        }
        Ok(Self {
            request_id: u64::from_be_bytes(body[0..8].try_into().unwrap()),
            start_height: u64::from_be_bytes(body[8..16].try_into().unwrap()),
            max_blocks: u32::from_be_bytes(body[16..20].try_into().unwrap()),
            max_bytes: u32::from_be_bytes(body[20..24].try_into().unwrap()),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlocksHeader {
    pub remote_tip: u64,
    pub start_height: u64,
    pub end_height: u64,
    pub count: u64,
    pub request_id: u64,
    pub more: bool,
    pub flags: u8,
    pub hdr_version: u16,
}

impl BlocksHeader {
    pub fn encode(&self) -> [u8; BLOCKS_HEADER_SIZE] {
        let mut out = [0u8; BLOCKS_HEADER_SIZE];
        out[0..8].copy_from_slice(&self.remote_tip.to_be_bytes());
        out[8..16].copy_from_slice(&self.start_height.to_be_bytes());
        out[16..24].copy_from_slice(&self.end_height.to_be_bytes());
        out[24..32].copy_from_slice(&self.count.to_be_bytes());
        out[32..40].copy_from_slice(&self.request_id.to_be_bytes());
        out[40] = if self.more { 1 } else { 0 };
        out[41] = self.flags;
        out[42..44].copy_from_slice(&self.hdr_version.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Ret<(Self, &[u8])> {
        if body.len() < BLOCKS_HEADER_SIZE {
            return errf!("BLOCKS body too short: {}", body.len());
        }
        let hdr = Self {
            remote_tip: u64::from_be_bytes(body[0..8].try_into().unwrap()),
            start_height: u64::from_be_bytes(body[8..16].try_into().unwrap()),
            end_height: u64::from_be_bytes(body[16..24].try_into().unwrap()),
            count: u64::from_be_bytes(body[24..32].try_into().unwrap()),
            request_id: u64::from_be_bytes(body[32..40].try_into().unwrap()),
            more: body[40] != 0,
            flags: body[41],
            hdr_version: u16::from_be_bytes([body[42], body[43]]),
        };
        if hdr.hdr_version != BLOCKS_HDR_VERSION {
            return errf!("BLOCKS hdr_version unsupported: {}", hdr.hdr_version);
        }
        if hdr.start_height == 0 || hdr.end_height < hdr.start_height {
            return errf!(
                "BLOCKS invalid span {}..{}",
                hdr.start_height,
                hdr.end_height
            );
        }
        let expected = hdr.end_height - hdr.start_height + 1;
        if hdr.count == 0 || hdr.count > 10_000 || hdr.count != expected {
            return errf!(
                "BLOCKS count invalid: declared {} span {}",
                hdr.count,
                expected
            );
        }
        Ok((hdr, &body[BLOCKS_HEADER_SIZE..]))
    }
}
