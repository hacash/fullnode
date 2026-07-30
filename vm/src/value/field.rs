

#[derive(Default, Debug, Clone)]
pub struct ValueKey {
    bytes: Vec<u8>
} 

impl Decode for ValueKey {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let used = buf.len();
        Ok((Self { bytes: buf.to_vec() }, used))
    }
}

impl Encode for ValueKey {
    fn encode(&self) -> Vec<u8> {
        self.bytes.clone()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.bytes);
    }
    fn size(&self) -> usize {
        self.bytes.len()
    }
}

impl ToJSON for ValueKey {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        format!("\"0x{}\"", hex::encode(&self.bytes))
    }
}
impl FromJSON for ValueKey {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        self.bytes = field::json_decode_binary(json)?;
        Ok(())
    }
}

impl ValueKey {
    pub fn from(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}



/*************************/

// just for storage

impl Decode for Value {
    fn decode(mut buf: &[u8]) -> Ret<(Self, usize)>{
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
        let out: Value;
        (sz, out) = match ty {
            ValueTy::Nil     => (0, Nil),
            ValueTy::Bool    => {
                let b = buf_to_uint!(u8, buf, 1);
                let value = Value::type_from(ValueTy::Bool, vec![b])
                    .map_err(|_| "value bool invalid".to_owned())?;
                (1, value)
            },
            ValueTy::U8      => (1, U8(buf_to_uint!(u8, buf, 1))),
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
            ValueTy::Address => {
                let (adr, sz) = field::Address::decode(buf)?;
                (sz, Address(adr))
            },
            _ => return errf!("Tuple, handle, compo or slice value item cannot be parsed"),
        };
        Ok((out, sz + 1))
    }
}

impl Encode for Value {
    fn encode(&self) -> Vec<u8> {
        match self {
            // Runtime-only variants are intentionally excluded from field serialization.
            // Parse also rejects them, so serialize must keep the same type boundary.
            Tuple(..) | Handle(..) | Compo(..) => {
                panic!("Value::serialize does not support Tuple/Handle/Compo")
            }
            Bytes(buf) => {
                assert!(
                    buf.len() < u16::MAX as usize,
                    "Value::serialize bytes length {} exceeds u16 field limit",
                    buf.len()
                );
                let mut out = Vec::with_capacity(1 + 2 + buf.len());
                out.push(self.ty_num());
                out.extend_from_slice(&(buf.len() as u16).to_be_bytes());
                out.extend_from_slice(buf);
                out
            }
            _ => {
                let buf = self.scalar_bytes().expect("non-scalar values are rejected above");
                let mut out = Vec::with_capacity(1 + buf.len());
                out.push(self.ty_num());
                out.extend_from_slice(&buf);
                out
            }
        }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.encode());
    }

    fn size(&self) -> usize {
        match self {
            Tuple(..) | Handle(..) | Compo(..) => {
                panic!("Value::size does not support Tuple/Handle/Compo")
            }
            Bytes(buf) => {
                assert!(
                    buf.len() < u16::MAX as usize,
                    "Value::size bytes length {} exceeds u16 field limit",
                    buf.len()
                );
                1 + 2 + buf.len()
            }
            _ => 1 + self.scalar_bytes().expect("non-scalar values are rejected above").len(),
        }
    }
}

impl ToJSON for Value {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        Value::to_json(self)
    }
}
impl FromJSON for Value {
    fn from_json(&mut self, _json: &str) -> Ret<()> {
        errf!("Value FromJSON not implemented")
    }
}

