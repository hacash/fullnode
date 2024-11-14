

macro_rules! uint_define {
    ($class:ident, $size: expr, $numlen: expr, $vty: ty ) => {
                
        #[derive(Default, Debug, Hash, Copy, Clone, PartialEq, Eq)]
        pub struct $class {
            value: $vty,
        }


        impl Display for $class {
            fn fmt(&self,f: &mut std::fmt::Formatter) -> std::fmt::Result{
                write!(f,"{}", self.value)
            }
        }

        impl Deref for $class {
            type Target = $vty;
            fn deref(&self) -> &$vty {
                &self.value
            }
        }

        impl AsRef<$vty> for $class {
            fn as_ref(&self) -> &$vty {
                &self.value
            }
        }

        
        ord_impl!{$class, value}
        compute_impl!{$class, value, $vty}
        from_uint_all!{$class, value, $vty}


        impl Parse for $class {
            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                let mut bts = bufeat(buf, $size)?;
                let pdn = $numlen - $size; // left zero
                if pdn > 0 {
                    bts = vec![ vec![0u8; pdn], bts ].concat();
                }
                self.value = <$vty>::from_be_bytes(bts.try_into().unwrap());
                Ok($size)
            }
        }

        impl Serialize for $class {
            fn serialize(&self) -> Vec<u8> {
                self.to_bytes().to_vec()
            }
            fn size(&self) -> usize {
                $size
            }
        }

        impl_field_only_new!{$class}

        impl Uint for $class {
            concat_idents!{to_ty = to_, $vty {
                fn to_ty(&self) -> $vty {
                    self.value
                }
            }}
            concat_idents!{from_ty = from_, $vty {
                fn from_ty(v: $vty) -> Self {
                    Self{ value: v }
                }
            }}
        }


        impl $class {

            pub const MAX: $vty = <$vty>::MAX;
            pub const SIZE: usize = $size as usize;

            pub const fn from(v: $vty) -> Self {
                Self{ value: v }
            }

            pub fn to_uint(&self) -> $vty {
                self.value
            }   

            pub fn as_uint(&self) -> &$vty {
                &self.value
            }   

            pub fn to_bytes(&self) -> [u8; $size] {
                let mut real = [0u8; $size];
                let bts = <$vty>::to_be_bytes(self.value);
                for x in 1 ..= $size {
                    real[$size-x] = bts[$numlen-x];
                }
                // println!("Uint to_bytes size {} bts {} real {}", $size, hex::encode(bts), hex::encode(real));
                real
            }
            

        }


    };
}


/*
* define
*/
uint_define!{Uint1, 1, 1, u8}
uint_define!{Uint2, 2, 2, u16}
uint_define!{Uint3, 3, 4, u32}
uint_define!{Uint4, 4, 4, u32}
uint_define!{Uint5, 5, 8, u64}
uint_define!{Uint6, 6, 8, u64}
uint_define!{Uint7, 7, 8, u64}
uint_define!{Uint8, 8, 8, u64}




/************************ test ************************/





#[cfg(test)]
mod uint_tests {
    use super::*;


    macro_rules! uint_test_one {
        ($ty: ty, $v: expr) => { {
            let u1 = <$ty>::from($v);
            let mut u1f = <$ty>::from(0);
            let _ = u1f.parse(&u1.serialize());
            assert_eq!(u1, u1f);
        } }
    }


    #[test]
    fn test2() {
        
        uint_test_one!(Uint1, 0);
        uint_test_one!(Uint1, 1);
        uint_test_one!(Uint1, 2);
        uint_test_one!(Uint1, 3);
        uint_test_one!(Uint1, 77);
        uint_test_one!(Uint1, 100);
        uint_test_one!(Uint1, 150);
        uint_test_one!(Uint1, 254);
        uint_test_one!(Uint1, 255);

        uint_test_one!(Uint2, 0);
        uint_test_one!(Uint2, 1);
        uint_test_one!(Uint2, 2);
        uint_test_one!(Uint2, 3);
        uint_test_one!(Uint2, 65534);
        uint_test_one!(Uint2, 65535);

        let m3: u32 = 256*256*256 - 1;
        uint_test_one!(Uint3, 0);
        uint_test_one!(Uint3, 1);
        uint_test_one!(Uint3, 1000);
        uint_test_one!(Uint3, m3-2);
        uint_test_one!(Uint3, m3-1);
        uint_test_one!(Uint3, m3);

        let m4: u32 = u32::MAX;
        uint_test_one!(Uint4, 0);
        uint_test_one!(Uint4, 1);
        uint_test_one!(Uint4, 74563000);
        uint_test_one!(Uint4, m4-2);
        uint_test_one!(Uint4, m4-1);
        uint_test_one!(Uint4, m4);

        let m5: u64 = 256*256*256*256*256 - 1;
        uint_test_one!(Uint5, 0);
        uint_test_one!(Uint5, 1);
        uint_test_one!(Uint5, 740345600);
        uint_test_one!(Uint5, m5-2);
        uint_test_one!(Uint5, m5-1);
        uint_test_one!(Uint5, m5);

        let m6: u64 = 256*256*256*256*256*256 - 1;
        uint_test_one!(Uint6, 0);
        uint_test_one!(Uint6, 1);
        uint_test_one!(Uint6, 7404534566600);
        uint_test_one!(Uint6, m6-2);
        uint_test_one!(Uint6, m6-1);
        uint_test_one!(Uint6, m6);

        let m7: u64 = 256*256*256*256*256*256*256 - 1;
        uint_test_one!(Uint7, 0);
        uint_test_one!(Uint7, 1);
        uint_test_one!(Uint7, 7434505674564500);
        uint_test_one!(Uint7, m7-2);
        uint_test_one!(Uint7, m7-1);
        uint_test_one!(Uint7, m7);

        let m8: u64 = u64::MAX;
        uint_test_one!(Uint8, 0);
        uint_test_one!(Uint8, 1);
        uint_test_one!(Uint8, 7408487745635624600);
        uint_test_one!(Uint8, m8-2);
        uint_test_one!(Uint8, m8-1);
        uint_test_one!(Uint8, m8);

    }


}





