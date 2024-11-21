

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
                $( mv += self.$item.parse(&buf[mv..])?; )+
                Ok(mv)
            }
        }

        impl Serialize for $class {
            fn serialize(&self) -> Vec<u8> {
                vec![ $( self.$item.serialize() ),+ ].concat()
            }
            fn size(&self) -> usize {
                [ $( self.$item.size() ),+ ].iter().sum()
            }
        }

        impl_field_only_new!{$class}



    )
}


// test
combi_struct!{ Test73895763489564,
    aaa: Uint1
}









#[macro_export]
macro_rules! combi_struct_with_parse_serialize {
    ($class:ident, ( $this:ident, $buf:ident, $parse:expr, $serialize:expr, $size:expr ), $( $item:ident : $type:ty )+ ) => (

        #[derive(Default, Debug, Clone, PartialEq, Eq)]
        pub struct $class {
            $(
                pub $item: $type
            ),+
        }

        impl Parse for $class {
            fn parse(&mut $this, $buf: &[u8]) -> Ret<usize> {
                $parse
            }
        }

        impl Serialize for $class {
            fn serialize(&$this) -> Vec<u8> {
                $serialize
            }
            fn size(&$this) -> usize {
                $size
            }
        }

        impl_field_only_new!{$class}

    )
}


#[macro_export]
macro_rules! combi_struct_with_parse {
    ($class:ident, ( $this:ident, $buf:ident, $parse:expr ), $( $item:ident : $type:ty )+ ) => (

        combi_struct_with_parse_serialize!{$class, (
            $this, $buf, $parse,
            vec![ $( $this.$item.serialize() ),+ ].concat(),
            [ $( $this.$item.size() ),+ ].iter().sum()
        ), $( $item : $type )+ }

    )
}






#[macro_export]
macro_rules! combi_struct_field_more_than_condition {
    ($class:ident, { $( $item:ident : $type:ty )+ }, $mrn:ident, $mrv:ty, $cdn:ident, $cdv:expr ) => (

        #[derive(Default, Debug, Clone, PartialEq, Eq)]
        pub struct $class {
            $(
                pub $item: $type,
            )+
            pub $mrn: $mrv
        }

        impl Parse for $class {

            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                let mut mv = 0;
                $( mv += self.$item.parse(&buf[mv..])?; )+
                if self.$cdn.uint() > $cdv {
                    mv += self.$mrn.parse(&buf[mv..])?;
                }
                Ok(mv)
            }

        }

        impl Serialize for $class {

            fn serialize(&self) -> Vec<u8> {
                let mut sers = vec![ $( self.$item.serialize() ),+ ];
                if self.$cdn.uint() > $cdv {
                    sers.push(self.$mrn.serialize());
                }
                sers.concat()
            }

            fn size(&self) -> usize {
                let mut sz = [ $( self.$item.size() ),+ ].iter().sum();
                if self.$cdn.uint() > $cdv {
                    sz += self.$mrn.size();
                }
                sz
            }

        }

        impl_field_only_new!{$class}

    )
}

