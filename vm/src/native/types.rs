







macro_rules! native_call_define {
    ( $( $name:ident = $v:expr, $gas:expr, $rty: expr)+ ) => {
        
#[allow(non_camel_case_types)]
#[repr(u8)]
#[derive(Default, PartialEq, Debug, Clone, Copy)]
pub enum NativeCall {
    #[default] Null   = 0u8,
    $(
        $name = $v,
    )+
}

impl NativeCall {
    $(

    concat_idents::concat_idents!{ const_name = idx_, $name {
    #[allow(non_upper_case_globals)]
    pub const const_name: u8 = $v;
    }}
    )+
    pub fn call(hei: u64, idx: u8, v: &[u8]) -> VmrtRes<(Value, i64)> {
        let cty: NativeCall = std_mem_transmute!(idx);
        match cty {
            $(
                Self::$name => $name(hei, v).map(|r|{
                    assert_eq!($rty, r.ty());
                    // assert_eq!($vsz, r.val_size());
                    (r, $gas)
                }),
            )+
            _ => return itr_err_fmt!(NativeCallError, "notfind native call func idx {}", idx),
        }
    }

    pub fn name(&self) -> &'static str {
        use NativeCall::*;
        match self {
            $(
                $name => stringify!($name),
            )+
            _ => unreachable!()
        }
    }

    pub fn from_name(name: &str) -> Option<(u8, NativeCall)> {
        Some(match name {
            $(
                stringify!($name) => (Self::$name as u8, Self::$name),
            )+
            _ => return None
        })
    }


}


    };
}



