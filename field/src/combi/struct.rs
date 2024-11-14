

#[macro_export]
macro_rules! combi_struct {
    ($class:ident, $( $item:ident : $type:ty )+ ) => (

        #[derive(Default, Debug, Clone, PartialEq, Eq)]
        pub struct $class {
            $(
                pub $item: $type
            ),+
        }

        impl Parse for $class {
            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                let mut mv = 0;
                $(
                    mv += self.$item.parse(&buf[mv..])?;
                )+
                Ok(mv)
            }
        }

        impl Serialize for $class {
            fn serialize(&self) -> Vec<u8> {
                vec![
                    $(
                        self.$item.serialize()
                    ),+
                ].concat()
            }
            fn size(&self) -> usize {
                [ 
                    $(
                        self.$item.size()
                    ),+
                ].iter().sum()
            }
        }

        impl_field_only_new!{$class}


    )
}


// test
combi_struct!{ Test73895763489564,
    aaa: Uint1
}



