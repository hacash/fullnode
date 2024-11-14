



macro_rules! fixed_define {
    ($class:ident, $size: expr) => {

        #[derive(Debug, Hash, Copy, Clone, PartialEq, Eq)]
        pub struct $class {
            bytes: [u8; $size],
        }

        impl Default for $class {
            fn default() -> Self {
                $class {
                    bytes: [0u8; $size],
                }
            }
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
            type Target = [u8; $size];
            fn deref(&self) -> &[u8; $size] {
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
                let bts = bufeat(buf, $size)?;
                self.bytes = bts.try_into().unwrap();
                Ok($size)
            }
        }

        impl Serialize for $class {
            fn serialize(&self) -> Vec<u8> {
                self.to_vec()
            }
            fn size(&self) -> usize {
                $size
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

            pub const SIZE: usize = $size as usize;
            pub const DEFAULT: Self = Self{ bytes: [0u8; $size] };

            pub fn to_vec(&self) -> Vec<u8> {
                self.bytes.to_vec()
            }

            pub fn from_vec(v: Vec<u8>) -> Self where Self: Sized {
                Self::from(v.try_into().unwrap())
            }

            pub fn from(v: [u8; $size]) -> Self where Self: Sized {
                Self{
                    bytes: v
                }
            }


        }


    }
}



fixed_define!{Fixed1,  1}
fixed_define!{Fixed2,  2}
fixed_define!{Fixed3,  3}
fixed_define!{Fixed4,  4}
fixed_define!{Fixed5,  5}
fixed_define!{Fixed6,  6}
fixed_define!{Fixed7,  7}
fixed_define!{Fixed8,  8}
fixed_define!{Fixed9,  9}
fixed_define!{Fixed10, 10}
fixed_define!{Fixed12, 12}
fixed_define!{Fixed15, 15}
fixed_define!{Fixed16, 16}
fixed_define!{Fixed18, 18}
fixed_define!{Fixed20, 20}
fixed_define!{Fixed21, 21}
fixed_define!{Fixed32, 32}
fixed_define!{Fixed33, 33}
fixed_define!{Fixed64, 64}


/*
* Bool
*/
pub type Bool = Fixed1;

impl Bool {

    pub fn check(&self) -> bool {
        self[0] != 0
    }

    pub fn new(v: bool) -> Self where Self: Sized {
        let v = match v {
            true => 1u8,
            false => 0u8,
        };
        Self{
            bytes: [v]
        }
    }

}

