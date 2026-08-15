use sys::{Ret, normalf};

pub trait Encode {
    fn size(&self) -> usize;
    fn encode_to(&self, out: &mut Vec<u8>);
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size());
        self.encode_to(&mut out);
        out
    }
}

pub trait Decode: Sized {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)>;
}

/// Decode a value whose length prefix has already been consumed by its caller.
pub trait ParsePrefix: Sized {
    fn create_with_prefix(prefix: &[u8], rest: &[u8]) -> Ret<(Self, usize)>;
}

pub trait Field: Encode + Decode + Clone + std::fmt::Debug {}

impl<T: Encode + Decode + Clone + std::fmt::Debug> Field for T {}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn read<T: Decode>(&mut self) -> Ret<T> {
        let (v, used) = T::decode(&self.buf[self.pos..])?;
        self.pos += used;
        Ok(v)
    }

    pub fn read_bytes(&mut self, n: usize) -> Ret<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return normalf!(
                "buffer too short: need {} got {}",
                n,
                self.buf.len() - self.pos
            );
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn used(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
}
