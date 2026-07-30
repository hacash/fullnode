//! `Bytes` — `Arc<Vec<u8>>` + `[start..end)` range, cheap clone / slice.
//!
//! The fundamental byte buffer type used across all crates for state values,
//! block data, and KV storage.  Lives in `sys` because it is as foundational
//! as `Hash` — zero domain semantics, used by every layer above.

use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Bytes {
    data: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl Bytes {
    pub fn from_vec(v: Vec<u8>) -> Self {
        let end = v.len();
        Self {
            data: Arc::new(v),
            start: 0,
            end,
        }
    }

    pub fn from_slice(s: &[u8]) -> Self {
        Self::from_vec(s.to_vec())
    }

    pub fn from_arc_vec_slice(data: Arc<Vec<u8>>, start: usize, len: usize) -> Self {
        let end = start + len;
        assert!(
            start <= end && end <= data.len(),
            "Bytes::from_arc_vec_slice out of bounds"
        );
        Self { data, start, end }
    }

    pub fn slice(&self, start: usize, len: usize) -> Self {
        let abs_start = self.start + start;
        let abs_end = abs_start + len;
        assert!(
            abs_start <= abs_end && abs_end <= self.end,
            "Bytes::slice out of bounds"
        );
        Self {
            data: self.data.clone(),
            start: abs_start,
            end: abs_end,
        }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn into_vec(self) -> Vec<u8> {
        let Self { data, start, end } = self;
        if start == 0 && end == data.len() {
            return Arc::try_unwrap(data).unwrap_or_else(|data| data.as_ref().clone());
        }
        data[start..end].to_vec()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.data[self.start..self.end]
    }
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.data, &other.data)
            && self.start == other.start
            && self.end == other.end
        {
            return true;
        }
        self.as_ref() == other.as_ref()
    }
}
impl Eq for Bytes {}

impl PartialOrd for Bytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Bytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl std::hash::Hash for Bytes {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state)
    }
}

impl std::borrow::Borrow<[u8]> for Bytes {
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

// ============================================================
// Byte-slice parsing helpers
// ============================================================

/// Parse first 8 bytes of `b` as big-endian u64.
/// Panics if `b.len() < 8` — a short value means data corruption.
#[inline]
pub fn parse_u64_be(b: &[u8]) -> u64 {
    let bytes: &[u8; 8] = b[..8]
        .try_into()
        .unwrap_or_else(|_| panic!("parse_u64_be: expected 8 bytes, got {}", b.len()));
    u64::from_be_bytes(*bytes)
}
