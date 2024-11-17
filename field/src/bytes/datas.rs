
macro_rules! datas_define {
    ($class:ident, $sty: ty, $lty: ty) => {

        #[derive(Default, Debug, Hash, Clone, PartialEq, Eq)]
        pub struct $class {
            count: $sty,
            bytes: Vec<u8>,
        }


        impl Display for $class {
            fn fmt(&self,f: &mut Formatter) -> Result {
                write!(f,"{}",hex::encode(&self.bytes))
            }
        }

        impl Index<usize> for $class {
            type Output = u8;
            fn index(&self, idx: usize) -> &Self::Output {
                &self.bytes[idx]
            }
        }

        impl IndexMut<usize> for $class {
            fn index_mut(&mut self, idx: usize) -> &mut Self::Output{
                &mut self.bytes[idx]
            }
        }

        impl Deref for $class {
            type Target = Vec<u8>;
            fn deref(&self) -> &Vec<u8> {
                &self.bytes
            }
        }

        impl AsRef<[u8]> for $class {
            fn as_ref(&self) -> &[u8] {
                self.bytes.as_slice()
            }
        }

        impl Parse for $class {
            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                self.count.parse(buf)?;
                let sz = self.count.to_uint() as usize;
                let bts = bufeat(buf, sz)?;
                *self = Self::from(bts)?;
                Ok(<$sty>::SIZE + sz)
            }
        }

        impl Serialize for $class {
            fn serialize(&self) -> Vec<u8> {
                vec![
                    self.count.serialize(),
                    self.bytes.to_vec()
                ].concat()
            }
            fn size(&self) -> usize {
                <$sty>::SIZE + self.count.to_uint() as usize
            }
        }

        impl_field_only_new!{$class}

        impl Hex for $class {
            fn to_hex(&self) -> String {
                hex::encode(&self.bytes)
            }
        }

        impl Base64 for $class {
            fn to_base64(&self) -> String {
                BASE64_STANDARD.encode(self)
            }
        }

        
        impl $class {

            pub fn into_vec(self) -> Vec<u8> {
                self.bytes
            }

            pub fn to_vec(&self) -> Vec<u8> {
                self.bytes.clone()
            }

            pub fn as_vec(&self) -> &Vec<u8> {
                &self.bytes
            }

            pub fn from(buf: Vec<u8>) -> Ret<Self> {
                if buf.len() > <$sty>::MAX as usize {
                    return Err(s!("datas length too long"))
                }
                Ok(Self {
                    count: <$sty>::from(buf.len() as $lty),
                    bytes: buf,
                })
            }

            pub fn length(&self) -> usize {
                self.count.to_usize()
            }
        
            pub fn push(&mut self, a: u8) -> Rerr {
                if self.bytes.len() + 1 > <$sty>::MAX as usize {
                    return errf!("append size overflow")
                }
                self.count += 1u8;
                self.bytes.push(a);
                Ok(())
            }
        
            pub fn append(&mut self, tar: &mut Vec<u8>) -> Rerr {
                self.count += tar.len() as u64;
                self.bytes.append(tar);
                Ok(())
            }

        }












    }
}



/*
* define 
*/
datas_define!{BytesW1, Uint1, u8}
datas_define!{BytesW2, Uint2, u16}
datas_define!{BytesW4, Uint4, u32}
