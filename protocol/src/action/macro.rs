
#[macro_export]
macro_rules! action_define {
    ($class:ident, $kid:expr, $lv:expr, $burn90:expr, $reqsign:expr, 
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
                if !$pctx.env().chain.fast_sync {
                    check_action_level($pctx.depth(), $pself, $pctx.tx().actions())?;
                }
                #[allow(unused_mut)] 
                let mut $pgas: u32 = $pself.size() as u32; // act size is base gas use
                let res: Ret<Vec<u8>> = $exec;
                unsafe {
                    ACTION_HOOK_FUNC($pself.kind(), $pself as &dyn Any, $pctx, &mut $pgas)?;
                }
                Ok(($pgas, res?))
            }
        }

        impl Action for $class {
            fn kind(&self) -> u16 { *self.kind }
            fn level(&self) -> i8 { $lv }
            fn burn_90(&$pself) -> bool { $burn90 }
            fn req_sign(&$pself) -> Vec<AddrOrPtr> { $reqsign.to_vec() } // request_need_sign_addresses
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



// check action level
pub fn check_action_level(depth: i8, act: &dyn Action, actions: &Vec<Box<dyn Action>>) -> Rerr {
        if depth > 8 {
            return errf!("action depth cannot over {}", 8)
        }
        let actlen = actions.len();
        if actlen < 1 || actlen > 200 {
            return errf!("one transaction max actions is 200")
        }
        let kid = act.kind();
        let alv = act.level();
        if alv == ActLv::TOP_ONLY {
            if actlen > 1 {
                return errf!("action {} just can execute on TOP_ONLY", kid)
            }
        } else if alv == ActLv::TOP_UNIQUE {
            let mut smalv = 0;
            for act in actions {
                if act.kind() == kid {
                    smalv += 1;
                }
            }
            if smalv > 1 {
                return errf!("action {} just can execute on level TOP_UNIQUE", kid)
            }
        } else if alv == ActLv::TOP {
            if depth >= 0 {
                return errf!("action just can execute on level TOP")
            }
        } else if alv == ActLv::AST {
            if depth >= 0 {
                return errf!("action just can execute on level AST")
            }
        } else if depth > alv {
            return errf!("action just can execute on depth {} but call in {}", alv, depth)
        }
        // ok
        Ok(())
}









//////////////////// TEST  ////////////////////


// test define action
action_define!{Test63856464969364, 9527, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [],
    {
        id: Uint1
        addr: Address
    },
    (self, _ctx, gas {
        errf!("never call")
        // Ok(vec![])
    })
}

