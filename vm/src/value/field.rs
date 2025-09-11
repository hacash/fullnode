

#[derive(Default, Debug, Clone)]
pub struct ValueKey {
    bytes: Vec<u8>
} 

impl Parse for ValueKey {
    fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
        self.bytes = buf.to_vec();
        Ok(buf.len())
    }
}

impl Serialize for ValueKey {
    fn serialize(&self) -> Vec<u8> {
        self.bytes.clone()
    }
    fn size(&self) -> usize {
        self.bytes.len()
    }
}

impl Field for ValueKey {}

impl ValueKey {
    pub fn from(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

/*************************/



impl Parse for Value {
    fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
        let ty = Uint1::build(buf)?;
        let buf = &buf[1..];
        macro_rules! buf_to_uint_unwrap { ($ty:ty, $n:expr) => {
            <$ty>::from_be_bytes(buf_fill_left_zero(buf, $n).try_into().unwrap())
        }}
        *self = match *ty {
            Self::TID_NIL   => Nil,
            Self::TID_BOOL  => Bool(maybe!(buf[0]==0, false, true)),
            Self::TID_U8    => U8(buf[0]),
            Self::TID_U16   => U16(buf_to_uint_unwrap!(u16, 2)),
            Self::TID_U32   => U32(buf_to_uint_unwrap!(u32, 4)),
            Self::TID_U64   => U64(buf_to_uint_unwrap!(u64, 8)),
            Self::TID_U128  => U128(buf_to_uint_unwrap!(u128, 16)),
            // Self::TID_U256 => Bytes(buf.to_vec()),
            Self::TID_BYTES => Bytes(buf.to_vec()),
            _ => unreachable!()
        };
        Ok(buf.len())
    }
}

impl Serialize for Value {
    fn serialize(&self) -> Vec<u8> {
        let ty = self.ty_num();
        let mut buf = self.raw();
        if self.is_uint() { // Uint
            buf = buf_drop_left_zero(&buf, 0)
        }
        iter::once(ty).chain(buf).collect()
    }
    fn size(&self) -> usize {
        self.val_size() + 1 // + ty id
    }
}


impl Field for Value {}

