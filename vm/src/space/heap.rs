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

    pub fn grow(&mut self, seg: u8, gas_extra: &GasExtra) -> VmrtRes<i64> {
        let seg = seg as usize;
        if seg < 1 {
            return itr_err_fmt!(HeapError, "heap grow cannot be empty");
        }
        if seg > 16 {
            return itr_err_fmt!(HeapError, "heap grow cannot exceed 16");
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
