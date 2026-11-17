use std::fmt::{Debug, Display, Formatter, Result};

use sys::ToHex;

use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::*;

macro_rules! read_be_value {
    ($bytes:expr, $len:expr, $ty:ty, $ctor:ident) => {{
        let mut buf = [0u8; $len];
        buf.copy_from_slice($bytes);
        Value::$ctor(<$ty>::from_be_bytes(buf))
    }};
}

#[allow(dead_code)]
#[derive(Default)]
pub struct Heap {
    // bsgas: i64,   // 1 2 4 8 16 32 64 128 256 512 1024 2048 4096 8192 segln: usize, // 256
    limit: usize, // 64 seg
    datas: Vec<u8>,
}

impl Display for Heap {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "0x{}", self.datas.to_hex())
    }
}

impl Debug for Heap {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "heap({}):0x{}", self.datas.len(), self.datas.to_hex())
    }
}

impl Heap {
    pub const SEGLEN: usize = 256;

    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }

    pub fn reset(&mut self, limit: usize) {
        self.limit = limit;
        self.datas.clear()
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

impl Heap {
    #[inline(always)]
    fn checked_right(&self, start: usize, len: usize, tip: &'static str) -> VmrtRes<usize> {
        let right = start
            .checked_add(len)
            .ok_or_else(|| ItrErr::new(HeapError, tip))?;
        if right > self.datas.len() {
            return Err(ItrErr::new(HeapError, tip));
        }
        Ok(right)
    }

    #[inline(always)]
    fn read_uint(&self, uty: u16, seg: u16) -> VmrtRes<Value> {
        let len = match uty {
            0 => 1usize,
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            _ => return itr_err_fmt!(HeapError, "uint type {} not supported", uty),
        };
        let idx = len
            .checked_mul(seg as usize)
            .ok_or_else(|| ItrErr::new(HeapError, "read overflow"))?;
        let right = self.checked_right(idx, len, "read overflow")?;
        let bytes = &self.datas[idx..right];
        Ok(match uty {
            0 => Value::U8(bytes[0]),
            1 => read_be_value!(bytes, 2, u16, U16),
            2 => read_be_value!(bytes, 4, u32, U32),
            3 => read_be_value!(bytes, 8, u64, U64),
            4 => read_be_value!(bytes, 16, u128, U128),
            _ => return itr_err_fmt!(HeapError, "uint type {} not supported", uty),
        })
    }

    /// `seg_max`: the per-grow segment cap (`SpaceCap::heap_grow_segment` —
    /// task 7 moved the literal 16 out of here; the cumulative budget is
    /// still `limit` through `heap_grow_gas` below).
    pub fn grow(&mut self, seg: u8, seg_max: usize, gas_extra: &GasExtra) -> VmrtRes<i64> {
        let seg = seg as usize;
        if seg < 1 {
            return itr_err_fmt!(HeapError, "heap grow cannot be empty");
        }
        if seg > seg_max {
            return itr_err_fmt!(HeapError, "heap grow cannot exceed {}", seg_max);
        }
        let oldseg = self.datas.len() / Self::SEGLEN;
        let gas = gas_extra.heap_grow_gas(oldseg, seg, self.limit)?;
        let newsz = self
            .datas
            .len()
            .checked_add(seg * Self::SEGLEN)
            .ok_or_else(|| ItrErr::new(HeapError, "heap size overflow"))?;
        self.datas.resize(newsz, 0u8);
        Ok(gas)
    }

    fn do_write(&mut self, start: usize, v: Value) -> VmrtErr {
        let data = v
            .extract_bytes()
            .map_err(|ItrErr(_, msg)| ItrErr::new(HeapError, &msg))?;
        let right = self.checked_right(start, data.len(), "write overflow")?;
        self.datas[start..right].copy_from_slice(&data);
        Ok(())
    }

    pub fn write(&mut self, start: usize, v: Value) -> VmrtErr {
        self.do_write(start, v)
    }

    pub fn do_read(&self, start: usize, len: usize) -> VmrtRes<Value> {
        let max = self.checked_right(start, len, "read overflow")?;
        let data = &self.datas[start..max];
        Ok(Value::Bytes(data.to_vec()))
    }

    // return Value::bytes
    pub fn read(&self, i: &Value, n: Value) -> VmrtRes<Value> {
        let start = i.extract_u32()? as usize;
        let length = n.extract_u16()? as usize;
        self.do_read(start, length)
    }

    /* 2 bit = u8 u16 u32 u64; remaining 6 bits encode the segment (u8: bytes 0..=63, u16: 0..=127, u32: 0..=255, u64: 0..=511) */
    pub fn read_u(&self, mark: u8) -> VmrtRes<Value> {
        let uty = mark >> 6;
        let seg = mark & 0b00111111;
        self.read_uint(uty as u16, seg as u16)
    }

    /* 3 bit = u8 u16 u32 u64 u128; remaining 13 bits encode the segment */
    pub fn read_ul(&self, mark: u16) -> VmrtRes<Value> {
        // upper 3 bits indicate uint type; remaining 13 bits indicate segment shift by 13 (5+8) explicitly to avoid precedence ambiguity
        let uty = mark >> 13;
        let seg = mark & 0b0001111111111111;
        self.read_uint(uty, seg)
    }
}

#[cfg(test)]
mod heap_tests {
    use super::*;
    use crate::rt::SpaceCap;

    fn gst() -> GasExtra {
        GasExtra::new(0, &base::VmExecutionParams::default())
    }

    /// Task 7: the per-grow segment cap is the `SpaceCap::heap_grow_segment`
    /// field (default 16 — the former literal inside `grow`), so every
    /// existing success/failure is unchanged.
    #[test]
    fn grow_default_cap_faces_unchanged() {
        let cap = SpaceCap::new(0);
        assert_eq!(cap.heap_grow_segment, 16);
        let mut h = Heap::new(cap.heap_segment);
        // seg == 0 stays HeapError
        let e = h.grow(0, cap.heap_grow_segment, &gst()).unwrap_err();
        assert_eq!(e.0, ItrErrCode::HeapError);
        // 17 segments exceed the default field
        let e = h.grow(17, cap.heap_grow_segment, &gst()).unwrap_err();
        assert_eq!(e.0, ItrErrCode::HeapError);
        assert!(e.1.contains("cannot exceed 16"), "msg: {}", e.1);
        // 16 segments grow fine
        h.grow(16, cap.heap_grow_segment, &gst()).unwrap();
        assert_eq!(h.do_read(0, 16 * Heap::SEGLEN).unwrap().val_size(), 16 * Heap::SEGLEN);
    }

    /// The field raised to its u8 parameter ceiling (255): the per-grow
    /// check passes for huge segments, but the CUMULATIVE budget stays
    /// `heap_segment` (64) through `heap_grow_gas` — OutOfHeap(34), the
    /// existing gas semantics, untouched.
    #[test]
    fn grow_raised_field_meets_cumulative_limit() {
        let mut cap = SpaceCap::new(0);
        cap.heap_grow_segment = 255;
        let mut h = Heap::new(cap.heap_segment);
        // a single 255-segment grow passes the field check but hits the
        // cumulative 64: OutOfHeap, not HeapError
        let e = h.grow(255, cap.heap_grow_segment, &gst()).unwrap_err();
        assert_eq!(e.0, ItrErrCode::OutOfHeap);
        // 64 segments fit the cumulative budget, then one more does not
        h.grow(64, cap.heap_grow_segment, &gst()).unwrap();
        let e = h.grow(1, cap.heap_grow_segment, &gst()).unwrap_err();
        assert_eq!(e.0, ItrErrCode::OutOfHeap);
    }

    /// The field lowered: the per-grow check rejects at the field value.
    #[test]
    fn grow_lowered_field_rejects_at_field() {
        let mut cap = SpaceCap::new(0);
        cap.heap_grow_segment = 4;
        let mut h = Heap::new(cap.heap_segment);
        let e = h.grow(5, cap.heap_grow_segment, &gst()).unwrap_err();
        assert_eq!(e.0, ItrErrCode::HeapError);
        assert!(e.1.contains("cannot exceed 4"), "msg: {}", e.1);
        h.grow(4, cap.heap_grow_segment, &gst()).unwrap();
        assert_eq!(h.do_read(0, 4 * Heap::SEGLEN).unwrap().val_size(), 4 * Heap::SEGLEN);
    }
}
