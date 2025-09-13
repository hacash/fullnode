
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum Value {
    #[default] Nil,   // type_id = 0
    Bool(bool),       //           1
    U8(u8),           //           2
    U16(u16),         //           3
    U32(u32),         //           4
    U64(u64),         //           5
    U128(u128),       //           6
    /*U256(u256),*/   //           7
    /*U512(u512),*/   //           8
    /*U1024(u1024),*/ //           9
    Bytes(Vec<u8>),   //           10
    // Addr(Address),    //          11
    /*Array(),*/      //           21
    /*Struct(),*/     //           22
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}


use Value::*;

impl Value {

    pub const TID_NIL:   u8 =  0;
    pub const TID_BOOL:  u8 =  1;
    pub const TID_U8:    u8 =  2;
    pub const TID_U16:   u8 =  3;
    pub const TID_U32:   u8 =  4;
    pub const TID_U64:   u8 =  5;
    pub const TID_U128:  u8 =  6;
    // pub const TID_U256:  u8 = 7;
    // pub const TID_U512:  u8 = 8;
    // pub const TID_U1024: u8 = 9;
    pub const TID_BYTES: u8 = 10;
    // pub const TID_ADDR:  u8 = 11;

    pub fn nil() -> Self {
        Nil
    }

    pub fn bool(b: bool) -> Self {
        Bool(b)
    }

    pub fn bool_true() -> Self {
        Bool(true)
    }

    pub fn bool_false() -> Self {
        Bool(false)
    }

    pub fn u8(n: u8) -> Self {
        U8(n)
    }

    pub fn empty_bytes() -> Self {
        Bytes(vec![])
    }
    
    pub fn bytes(b: Vec<u8>) -> Self {
        Bytes(b)
    }

    pub fn is_nil(&self) -> bool {
        match self {
            Nil => true,
            _ => false,
        }
    }
    
    pub fn is_bool(&self) -> bool {
        match self {
            Bool(..) => true,
            _ => false,
        }
    }


    pub fn is_uint(&self) -> bool {
        match self {
            U8(..) | 
            U16(..) | 
            U32(..) | 
            U64(..) | 
            U128(..) 
            /*| U256(_)*/ => true,
            _ => false,
        }
    }

    pub fn is_bytes(&self) -> bool {
        match self {
            Bytes(..) => true,
            _ => false,
        }
    }

    pub fn check_false(&self) -> bool {
        ! self.check_true()
    }
    
    pub fn check_true(&self) -> bool {
        match self {
            Nil     => false,
            Bool(b) => *b,
            U8(n)   => *n!=0,
            U16(n)  => *n!=0,
            U32(n)  => *n!=0,
            U64(n)  => *n!=0,
            U128(n) => *n!=0,
            // U256(n)   => *n!=0,
            Bytes(b)=> buf_not_zero(b),
        }
    }

    pub fn raw(&self) -> Vec<u8> {
        match &self {
            Nil => vec![],
            Bool(n) => vec![maybe!(n, 1, 0)],
            U8(n) =>   n.to_be_bytes().into(),
            U16(n) =>  n.to_be_bytes().into(),
            U32(n) =>  n.to_be_bytes().into(),
            U64(n) =>  n.to_be_bytes().into(),
            U128(n) => n.to_be_bytes().into(),
            Bytes(buf) => buf.clone(),
        }
    }

    pub fn ty_num(&self) -> u8 {
        match self {
            Nil          => Self::TID_NIL,
            Bool(..)     => Self::TID_BOOL,
            U8(..)       => Self::TID_U8,
            U16(..)      => Self::TID_U16,
            U32(..)      => Self::TID_U32,
            U64(..)      => Self::TID_U64,
            U128(..)     => Self::TID_U128,
            // U256()..)   => Self::TID_U256,
            Bytes(..)    => Self::TID_BYTES,
            // Array(..)   => 9,
            // Struct(..)  => 10,
        }
    }

    pub fn val_size(&self) -> usize {
        match self {
            Nil      => 0,
            Bool(..) => 1,
            U8(..)   => 1,
            U16(..)  => 2,
            U32(..)  => 4,
            U64(..)  => 8,
            U128(..) => 16,
            // U256(..) => 32,
            Bytes(b) => b.len(),
        }
    }

    pub fn valid(self, cap: &SpaceCap) -> VmrtRes<Self> {
        if self.val_size() > cap.max_value_size {
            return itr_err_code!(OutOfValueSize)
        }
        Ok(self)
    }

    pub fn to_uint(&self) -> u128 {
        match self {
            Nil =>          0,
            Bool(true) =>   1,
            Bool(false) =>  0,
            U8(n) =>   *n as u128,
            U16(n) =>  *n as u128,
            U32(n) =>  *n as u128,
            U64(n) =>  *n as u128,
            U128(n) => *n as u128,
            // U256(n) => format!("{}u256", n),
            Bytes(b) => match buf_to_uint(b) {
                Ok(b) => b.to_uint(),
                _ => 0
            },
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Nil =>          s!("nil"),
            Bool(true) =>   s!("true"),
            Bool(false) =>  s!("false"),
            U8(n) =>   format!("{}u8", n),
            U16(n) =>  format!("{}u16", n),
            U32(n) =>  format!("{}u32", n),
            U64(n) =>  format!("{}u64", n),
            U128(n) => format!("{}u128", n),
            // U256(n) => format!("{}u256", n),
            Bytes(b) => match ascii_show_string(b) {
                Some(s) => format!("\"{}\"", s),
                _ => "0x".to_owned() + &hex::encode(b),
            },
        }
    }


}



