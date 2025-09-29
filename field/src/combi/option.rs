

/* 
* create macro
*/
#[macro_export] 
macro_rules! combi_option {
    ($class:ident, $t1:ty, $t2:ty) => (

        #[derive(Clone, PartialEq, Eq)]
        pub enum $class {
            Val1($t1),
            Val2($t2),
        }

        impl std::fmt::Debug for $class {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f,"[option]")
            }
        }

        impl Default for $class {
            fn default() -> Self { Self::Val1(<$t1>::default()) }
        }


        impl Parse for $class {

            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                let swt = Uint1::build(buf)?;
                let buf = &buf[1..];
                Ok(1 + match *swt == 0 {
                    true => {
                        let (v, sk) = <$t1>::create(buf)?;
                        *self = Self::Val1( v );
                        sk
                    },
                    false => {
                        let (v, sk) = <$t2>::create(buf)?;
                        *self = Self::Val2( v );
                        sk
                    }
                })
            }
        }

        impl Serialize for $class {

            fn serialize(&self) -> Vec<u8> {
                let bts = match self {
                    Self::Val1(v1) => (0u8, v1.serialize()),
                    Self::Val2(v2) => (1u8, v2.serialize()),
                };
                vec![vec![bts.0], bts.1].concat()
            }

            fn size(&self) -> usize {
                1 + match self {
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
