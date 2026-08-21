

#[repr(u8)]
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq)]
pub enum ValueTy {
    #[default]
    Nil         = 0,
    Bool        = 1,
    U8          = 2,
    U16         = 3,
    U32         = 4,
    U64         = 5,
    U128        = 6,
    // 7 and 10-12 are reserved (11 formerly HeapSlice)
    Bytes       = 8,
    Address     = 9,
    Tuple       = 13,
    Compo       = 14,
    Handle      = 15
}

impl ValueTy {

    pub fn check_func_argv_type(&self) -> Rerr {
        use ValueTy::*;
        match self {
            Nil | Tuple => errf!("Value Type {:?} cannot be func argv", self),
            _ => Ok(())
        }
    }

    /// Allowed as function return value.
    pub fn check_func_retv_type(&self) -> Rerr {
        use ValueTy::*;
        match self {
            Nil => errf!("Value Type {:?} cannot be func retval", self),
            _ => Ok(())
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ValueTy::Nil       => "nil"       ,
            ValueTy::Bool      => "bool"      ,
            ValueTy::U8        => "u8"        ,
            ValueTy::U16       => "u16"       ,
            ValueTy::U32       => "u32"       ,
            ValueTy::U64       => "u64"       ,
            ValueTy::U128      => "u128"      ,
            ValueTy::Bytes     => "bytes"     ,
            ValueTy::Address   => "address"   ,
            ValueTy::Tuple     => "tuple"     ,
            ValueTy::Compo     => "compo"     ,
            ValueTy::Handle    => "handle"    ,
        }
    }

    pub fn is_uint(&self) -> bool {
        matches!(self, ValueTy::U8 | ValueTy::U16 | ValueTy::U32 | ValueTy::U64 | ValueTy::U128)
    }

    pub fn uint_bits(&self) -> Option<u16> {
        match self {
            ValueTy::U8 => Some(8),
            ValueTy::U16 => Some(16),
            ValueTy::U32 => Some(32),
            ValueTy::U64 => Some(64),
            ValueTy::U128 => Some(128),
            _ => None,
        }
    }

    pub fn from_name(s: &str) -> Ret<Self> {
        use ValueTy::*;
        Ok(match s {
            "nil"       => Nil,
            "bool"      => Bool,
            "u8"        => U8,
            "u16"       => U16,
            "u32"       => U32,
            "u64"       => U64,
            "u128"      => U128,
            "bytes"     => Bytes,
            "address"   => Address,
            "tuple"     => Tuple,
            "compo"     => Compo,
            "handle"    => Handle,
            _ => return errf!("unknown type '{}'", s),
        })
    }

    pub fn build(t: u8) -> Ret<Self> {
        use ValueTy::*;
        Ok(match t {
            0  => Nil       ,
            1  => Bool      ,
            2  => U8        ,
            3  => U16       ,
            4  => U32       ,
            5  => U64       ,
            6  => U128      ,
            8  => Bytes     ,
            9  => Address   ,
            13 => Tuple     ,
            14 => Compo     ,
            15 => Handle    ,
            _ => return errf!("unknown type")
        })
    }



}

pub fn parse_value_ty_param(raw: u8) -> VmrtRes<ValueTy> {
    ValueTy::build(raw).map_ire(ItrErrCode::InstParamsErr)
}

pub fn parse_cto_target_ty_param(raw: u8) -> VmrtRes<ValueTy> {
    use ValueTy::*;
    let ty = parse_value_ty_param(raw)?;
    match ty {
        Bool | U8 | U16 | U32 | U64 | U128 | Bytes | Address => Ok(ty),
        _ => Err(ItrErr::code(ItrErrCode::InstParamsErr)),
    }
}

