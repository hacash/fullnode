

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
            <$ty>::from_be_bytes(buf[0..$l].try_into().unwrap())
        }}}
        let sz: usize;
        (sz, *self) = match ty {
            ValueTy::Nil     => (0, Nil),
            ValueTy::Bool    => (1, Bool(maybe!(buf[0]==0, false, true))),
            ValueTy::U8      => (1, U8(buf[0])),
            ValueTy::U16     => (2,   U16(buf_to_uint!(u16,  buf,  2))),
            ValueTy::U32     => (4,   U32(buf_to_uint!(u32,  buf,  4))),
            ValueTy::U64     => (8,   U64(buf_to_uint!(u64,  buf,  8))),
            ValueTy::U128    => (16, U128(buf_to_uint!(u128, buf, 16))),
            ValueTy::Bytes   => {
                let l = buf_to_uint!(u16,  buf,  2) as usize;
                buf = &buf[2..];
                if buf.len() < l {
                    return err
                }
                (2 + l as usize, Bytes(buf[0..l].to_vec()))
            },
            ValueTy::Address => (field::Address::SIZE, Address(field::Address::from_bytes(&buf)?)),
            _ => panic!("Compo or slice value item cannot be parse"),
        };
        Ok(sz + 1)
    }
}

impl Serialize for Value {
    fn serialize(&self) -> Vec<u8> {
        let ty = self.ty_num();
        let mut buf = self.raw();
        if self.is_bytes() { // Uint
            buf = [(buf.len() as u16).to_be_bytes().to_vec(), buf].concat()
        }
        iter::once(ty).chain(buf).collect()
    }

    fn size(&self) -> usize {
        1 + self.can_get_size().unwrap() as usize // + ty id
    }
}


impl Field for Value {}

