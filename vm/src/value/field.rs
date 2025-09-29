

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

// just for storage

impl Parse for Value {
    fn parse(&mut self, mut buf: &[u8]) -> Ret<usize>{
        let err = errf!("value buf too short");
        let bl = buf.len();
        if bl < 1 {
            return err
        }
        let ty = ValueTy::build(buf[0])?;
        buf = &buf[1..];
        macro_rules! buf_to_uint { ($ty:ty, $buf:expr, $l:expr) => {{
            if buf.len() < $l {
                return err
            }
            <$ty>::from_be_bytes(buf_fill_left_zero(buf, $l).try_into().unwrap())
        }}}
        *self = match ty {
            ValueTy::Nil       => Nil,
            ValueTy::Bool      => Bool(maybe!(buf[0]==0, false, true)),
            ValueTy::U8        => U8(buf[0]),
            ValueTy::U16       => U16(buf_to_uint!(u16, buf, 2)),
            ValueTy::U32       => U32(buf_to_uint!(u32, buf, 4)),
            ValueTy::U64       => U64(buf_to_uint!(u64, buf, 8)),
            ValueTy::U128      => U128(buf_to_uint!(u128, buf, 16)),
            ValueTy::Bytes     => Bytes(buf.to_vec()),
            ValueTy::Addr      => Addr(Address::from_bytes(&buf)?),
            _ => panic!("Compo value item cannot be parse"),
        };
        Ok(bl)
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
        1 + self.can_get_size().unwrap() as usize // + ty id
    }
}


impl Field for Value {}

