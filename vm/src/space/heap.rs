
#[allow(dead_code)]
#[derive(Default, Debug)]
pub struct Heap {
    // bsgas: i64,   // 1 2 4 8 16 32 64 128 256 512 1024 2048 4096 8192 
    // segln: usize, // 256
    limit: usize, // 64 seg
    datas: Vec<u8>,
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

}


use crate::VmrtRes;


impl Heap {

    pub fn grow(&mut self, seg: u8) -> VmrtRes<i64> {
        if seg < 1 {
            return itr_err_fmt!(HeapError, "heap grow cannot empty")
        }
        if seg > 16 {
            return itr_err_fmt!(HeapError, "heap grow cannot more than 16")
        }
        let seg = seg as usize;
        let oldseg = self.datas.len() / Self::SEGLEN;
        if oldseg + seg > self.limit {
            return itr_err_code!(OutOfHeap)
        }
        let mut gas = 2u64.pow(oldseg as u32) as i64;
        let mut adgs = gas;
        for _ in 0..seg-1 {
            gas += adgs;
            adgs *= 2;
        } 
        self.datas.reserve(seg * Self::SEGLEN);
        Ok(gas)
    }

    fn do_write(&mut self, start: usize, v: Value) -> VmrtErr {
        let data = v.canbe_bytes_ec(HeapError)?;
        let right = start + data.len();
        if right > self.datas.len() {
            return itr_err_fmt!(HeapError, "write overflow")
        }
        let (_, right) = self.datas.split_at_mut(start);
        let (left, _) = right.split_at_mut(data.len());
        left.copy_from_slice(&data);
        Ok(())
    }

    pub fn write(&mut self, k: Value, v: Value) -> VmrtErr {
        let start = k.checked_u32()? as usize;
        self.do_write(start, v)
    }

    pub fn write_x(&mut self, start: u8, v: Value) -> VmrtErr {
        self.do_write(start as usize, v)
    }

    pub fn write_xl(&mut self, start: u16, v: Value) -> VmrtErr {
        self.do_write(start as usize, v)
    }

    pub fn do_read(&self, start: usize, len: usize) -> VmrtRes<Value> {
        let max = start + len;
        if max > self.datas.len() {
            return itr_err_fmt!(HeapError, "read overflow")
        }
        let data = &self.datas[start..start+len];
        Ok(Value::Bytes(data.to_vec()))
    }

    // return Value::bytes
    pub fn read(&self, k: Value, n: &Value) -> VmrtRes<Value> {
        let start  = k.checked_u32()? as usize;
        let length = n.checked_u16()? as usize;
        self.do_read(start, length)
    }

    pub fn slice(&self, s: Value, l: &Value) -> VmrtRes<Value> {
        let start  = s.checked_u32()?;
        let length = l.checked_u32()?;
        let max = start + length;
        if max as usize > self.datas.len() {
            return itr_err_fmt!(HeapError, "create slice overflow")
        }
        Ok(Value::HeapSlice((start, length)))
    }

    /*
        2 bit = u8 u16 u32 u64
        6 bit = seg max 64 (u8:64, u16:128, u32:256, u64:512)
    */
    pub fn read_u(&self, mark: u8) -> VmrtRes<Value> {
        let uty = mark >> 6;
        let seg = mark & 0b00111111;
        let len = [1,2,4,8][uty as usize] as usize;
        let idx = len * seg as usize;
        let mut val = self.do_read(idx, len)?;
        match uty {
            0 => val.cast_u8(),
            1 => val.cast_u16(),
            2 => val.cast_u32(),
            3 => val.cast_u64(),
            _ => unreachable!()
        }?;
        Ok(val)
    }

    /*
        3   bit = u8 u16 u32 u64 u128 u256
        5+8 bit = seg max 64 (u8:64, u16:128, u32:256, u64:512)
    */
    pub fn read_ul(&self, mark: u16) -> VmrtRes<Value> {
        let uty = mark >> 6+8;
        if uty > 4 {
            return itr_err_fmt!(HeapError, "uint type {} not support", uty)
        }
        let seg = mark & 0b0001111111111111;
        let len = [1,2,4,8,16][uty as usize] as usize;
        let idx = len * seg as usize;
        let mut val = self.do_read(idx, len)?;
        match uty {
            0 => val.cast_u8(),
            1 => val.cast_u16(),
            2 => val.cast_u32(),
            3 => val.cast_u64(),
            4 => val.cast_u128(),
            // 5 => val.cast_256(),
            _ => unreachable!()
        }?;
        Ok(val)
    }






}