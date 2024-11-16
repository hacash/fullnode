
#[macro_export]
macro_rules! action_define {
    ($class:ident, $kid:expr, $lv:expr, $burn90:expr,
        { $( $item:ident : $ty:ty )* },
        ($pself:ident, $pctx:ident, $pgas:ident $exec:expr)
    ) => {

        #[derive(Default, Debug, Clone, PartialEq, Eq)]
        pub struct $class {
            kind: Uint2,
            $(
                pub $item: $ty,
            )*
        }


        impl Parse for $class {
            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                let mut mv;
                mv = self.kind.parse(&buf)?;
                $(
                    mv += self.$item.parse(&buf[mv..])?;
                )*
                Ok(mv)
            }
        }

        impl Serialize for $class {
            fn serialize(&self) -> Vec<u8> {
                vec![
                    self.kind.serialize(),
                    $(
                        self.$item.serialize()
                    ),*
                ].concat()
            }
            fn size(&self) -> usize {
                [ 
                    self.kind.size(),
                    $(
                        self.$item.size()
                    ),*
                ].iter().sum()
            }
        }


        impl Field for $class {
            fn new() -> Self {
                Self {
                    kind: Uint2::from(Self::KIND),
                    ..Default::default()
                }
            }
        }

        impl ActExec for $class {
            fn execute(&$pself, $pctx: &mut dyn Context) -> Ret<(u32, Vec<u8>)> {
                #[allow(unused_mut)] 
                let mut $pgas: u32 = 0;
                let res: Ret<Vec<u8>> = $exec;
                Ok(($pgas, res?))
            }
        }

        impl Action for $class {
            fn kind(&self) -> u16 { self.kind.to_uint() }
            fn level(&self) -> i8 { $lv }
            fn burn_90(&self) -> bool { $burn90 }
        }

        impl $class {
            pub const KIND: u16 = $kid;
        }

        
    };
}


#[macro_export]
macro_rules! action_register {
    ( $( $kty:ident )+ ) => {
        
        pub fn try_create(kind: u16, buf: &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>> {
            match kind {
                $(<$kty>::KIND => {
                    let (act, sk) = <$kty>::create(buf)?;
                    Ok(Some((Box::new(act), sk)))
                },)+
                _ => Ok(None)
            }
        }
    };
}



//////////////////// TEST  ////////////////////


// test define action
action_define!{Test63856464969364, 9527, 
    ActLv::MAINCALL, // level
    false, // burn 90 fee
    {
        id: Uint1
        addr: Address
    },
    (self, _ctx, gas {
        errf!("never call")
        // Ok(vec![])
    })
}

