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

    /// Bounds-check and advance the cursor without returning the skipped bytes.
    pub fn skip(&mut self, n: usize) -> Ret<()> {
        if self.pos + n > self.buf.len() {
            return normalf!(
                "buffer too short: need {} got {}",
                n,
                self.buf.len() - self.pos
            );
        }
        self.pos += n;
        Ok(())
    }

    pub fn read_bytes(&mut self, n: usize) -> Ret<&'a [u8]> {
        let start = self.pos;
        self.skip(n)?;
        Ok(&self.buf[start..self.pos])
    }

    /// Decode from the remaining slice via `decode`, then advance by the
    /// reported byte count. Dynamic dispatch still takes `&[u8]`.
    pub fn read_with<T, F>(&mut self, decode: F) -> Ret<T>
    where
        F: FnOnce(&[u8]) -> Ret<(T, usize)>,
    {
        let (value, used) = decode(&self.buf[self.pos..])?;
        self.skip(used)?;
        Ok(value)
    }

    pub fn used(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_and_read_with_advance_the_cursor() {
        let buf = [1u8, 2, 3, 4];
        let mut r = Reader::new(&buf);
        r.skip(1).unwrap();
        assert_eq!(r.used(), 1);
        let n: u8 = r
            .read_with(|rest| {
                assert_eq!(rest, &[2, 3, 4]);
                Ok((rest[0], 1usize))
            })
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(r.used(), 2);
        let err = r.skip(8).unwrap_err();
        assert_eq!(err.as_str(), "buffer too short: need 8 got 2");
    }
}
