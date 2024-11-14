

/* 
* create macro
*/
#[macro_export] 
macro_rules! combi_revenum {
    ($class:ident, $t1:ty, $t2:ty, $swtv: expr) => (

        #[derive(Clone, PartialEq, Eq)]
        pub enum $class {
            Val1($t1),
            Val2($t2),
        }

        impl std::fmt::Debug for $class {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f,"[enum]")
            }
        }

        impl Default for $class {
            fn default() -> Self { Self::Val1(<$t1>::default()) }
        }


        impl Parse for $class {

            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                if buf.len() < 1 {
                    return Err("buf too short".to_owned())
                }
                if buf[0] < $swtv {
                    let (v, sk) = <$t1>::create(buf)?;
                    *self = Self::Val1( v );
                    Ok(sk)
                }else{
                    let mut nbuf = buf[1..].to_vec();
                    nbuf[0] -= $swtv;
                    let (v, sk) = <$t2>::create(&nbuf)?;
                    *self = Self::Val2( v );
                    Ok(sk + 1)
                }
            }
        }

        impl Serialize for $class {

            fn serialize(&self) -> Vec<u8> {
                match self {
                    Self::Val1(v1) => v1.serialize(),
                    Self::Val2(v2) => {
                        let mut b = v2.serialize();
                        let mxv = b[0] as usize + $swtv as usize;
                        if mxv > 255 {
                            panic!("mark value too big")
                        }
                        b[0] = mxv as u8;
                        b
                    },
                }
            }

            fn size(&self) -> usize {
                match self {
                    Self::Val1(v1) => v1.size(),
                    Self::Val2(v2) => v2.size(),
                }
            }

        }

        impl_field_only_new!{$class}


        impl $class {
            

        }




    )
}
