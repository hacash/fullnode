







macro_rules! native_call_define {
    ( $( $name:ident = $v:expr, $gas:expr )+ ) => {
        
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

    pub fn call(idx: u8, v: &[u8]) -> VmrtRes<(Value, i64)> {
        let cty: NativeCall = std_mem_transmute!(idx);
        match cty {
            $(
                Self::$name => $name(v).map(|r|(r,$gas)),
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



