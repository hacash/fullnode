

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
    // U256     = 7
    // ...      = 8 
    // ...      = 9 
    Bytes       = 10,
    Addr        = 11,
    // ...      = 12
    // ...      = 13
    HeapSlice   = 14,
    Compo       = 15
}

impl ValueTy {

    pub fn canbe_argv(&self) -> Rerr {
        use ValueTy::*;
        match self {
            Nil | HeapSlice | Compo => errf!("Value Type {:?} cannot be func argv", self),
            _ => Ok(())
        }
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
            /* */
            10 => Bytes     ,
            11 => Addr      ,
            /* */
            14 => HeapSlice ,
            15 => Compo     ,
            _ => return errf!("ValueTy {} not find", t)
        })
    }



}


/**********************************************/



#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum Value {
    #[default] Nil,        // type_id = 0
    Bool(bool),            //           1
    U8(u8),                //           2
    U16(u16),              //           3
    U32(u32),              //           4
    U64(u64),              //           5
    U128(u128),            //           6
    /*U256(u256),*/        //           7
    /*U512(u512),*/        //           8
    /*U1024(u1024),*/      //           9
    Bytes(Vec<u8>),        //           11
    Addr(Address),         //           12
    /*...*/                //           ..
    HeapSlice((u32, u32)), //           21
    /*Array(),*/           //           22
    /*Struct(),*/          //           23
    Compo(CompoItem),      //           25
}


impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}


use Value::*;

impl Value {

    pub fn ty(&self) -> ValueTy {
        match self {
            Nil           => ValueTy::Nil,
            Bool(..)      => ValueTy::Bool,
            U8(..)        => ValueTy::U8,
            U16(..)       => ValueTy::U16,
            U32(..)       => ValueTy::U32,
            U64(..)       => ValueTy::U64,
            U128(..)      => ValueTy::U128,
            Bytes(..)     => ValueTy::Bytes,
            Addr(..)      => ValueTy::Addr,
            HeapSlice(..) => ValueTy::HeapSlice,
            Compo(..)     => ValueTy::Compo,
        }
    }

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

    pub fn is_addr(&self) -> bool {
        match self {
            Addr(..) => true,
            _ => false,
        }
    }


    pub fn compo_ref(&self) -> VmrtRes<&CompoItem> {
        let Value::Compo(compo) = self else {
            return itr_err_code!(CompoOpNotMatch)
        };
        Ok(compo)
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
            Bytes(b)=> buf_not_zero(b),
            _       => true, // Addr Compo ....
        }
    }

    /*
    pub fn _____deval(&self, heap: &Heap) -> VmrtRes<Vec<u8>> {
        match self {
            Compo(..) => itr_err_code!(CompoToSerialize),
            HeapSlice((s, l)) => {
                match heap.do_read(*s as usize, *l as usize)? {
                    Bytes(buf) => Ok(buf),
                    _ => never!()
                }
            }
            _ => Ok(self.raw())
        }
    }
    */


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
            Addr(a)    => a.serialize(),
            HeapSlice((s, l)) => vec![s.to_be_bytes(), l.to_be_bytes()].concat(),
            // not support
            Compo(..) => "{compo value ...}".to_owned().into_bytes(),
        }
    }

    pub fn compo(&mut self) -> VmrtRes<&mut CompoItem> {
        let Value::Compo(compo) = self else {
            return itr_err_code!(CompoOpNotMatch)
        };
        Ok(compo)
    }

    pub fn ty_num(&self) -> u8 {
        self.ty() as u8
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
            Bytes(b) => b.len(),
            Addr(..) => Address::SIZE,
            HeapSlice(..) => 4 + 4,
            // not support
            Compo(..) => usize::MAX,
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
            Bytes(b) => match buf_to_uint(b) {
                Ok(b) => b.to_uint(),
                _ => 0
            },
            _ => 0,
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
            Bytes(b) => match ascii_show_string(b) {
                Some(s) => format!("\"{}\"", s),
                _ => "0x".to_owned() + &hex::encode(b),
            },
            Addr(a) => a.to_readable(),
            HeapSlice((s, l)) => format!("heap[{},{}]", s, l),
            Compo(a) => format!("compo[{}]", a.len()),
        }
    }


}



