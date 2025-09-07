
/********************************/


macro_rules! keyword_define {
    ( $( $k:ident : $s:expr  )+ ) => {


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum KwTy {
    $( $k ),+
}


impl KwTy {

    pub fn build(s: &str) -> Ret<KwTy> {
        use KwTy::*;
        Ok(match s {
            $( $s => $k, )+
            _ => return errf!("unsupport Keyword '{}'", s)
        })
    }
}

    }
}


keyword_define!{
    DColon    : "::"
    Colon     : ":"
    Dot       : "."
    Assign    : "="
    AsgAdd    : "+="
    AsgSub    : "-="
    AsgMul    : "*="
    AsgDiv    : "/="
    Use       : "use"
    Lib       : "lib"
    Let       : "let"
    If        : "if"
    Else      : "else"
    While     : "while"
    Finish    : "finish"
    Return    : "return"
    Abort     : "abort"
    Throw     : "throw"
    Assert    : "assert"
    Bool      : "bool"
    Bytes     : "bytes"
    CallCode  : "callcode"
    ByteCode  : "bytecode"
    As        : "as"
    U8        : "u8"
    U16       : "u16"
    U32       : "u32"
    U64       : "u64"
    U128      : "u128"
}   




/********************************/



macro_rules! operator_define {
    ( $( $k:ident : $s:expr, $lv:expr  )+ ) => {
        
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u8)]
pub enum OpTy {
    $( $k ),+
}


impl OpTy {

    pub fn level(&self) -> u8 {
        use OpTy::*;
        match self {
            $( $k => $lv ),+
        }
    }

    pub fn symbol(&self) -> &'static str {
        use OpTy::*;
        match self {
            $( $k => $s ),+
        }
    }

    pub fn build(s: &str) -> Ret<OpTy> {
        use OpTy::*;
        Ok(match s {
            $( $s => $k, )+
            _ => return errf!("unsupport Operator '{}'", s)
        })
    }

    pub fn bytecode(&self) -> Bytecode {
        use OpTy::*;
        match self {
            $( $k => Bytecode::$k, )+
        }
    }

    pub fn from_bytecode(code: Bytecode) -> Ret<OpTy> {
        Ok(match code {
            $( Bytecode::$k => OpTy::$k, )+
            _ => return errf!("cannot find OpTy {:?}", code)
        })
    }
}


    }
}



operator_define!{
    NOT       : "!" ,     200
    POW       : "**",     175 
    MUL       : "*" ,     150
    DIV       : "/" ,     150
    MOD       : "%" ,     150
    ADD       : "+" ,     120
    SUB       : "-" ,     120
    BSHL      : "<<",     110
    BSHR      : ">>",     110
    BAND      : "&" ,     109
    BXOR      : "^" ,     108
    BOR       : "|" ,     107
    EQ        : "==",     90 
    NEQ       : "!=",     90
    GE        : ">=",     90 
    LE        : "<=",     90 
    GT        : ">" ,     90
    LT        : "<" ,     90
    AND       : "&&",     79 
    OR        : "||",     78 
    CAT       : "++",     60
}



/********************************/



macro_rules! irfn_define {
    ( $( $c:ident : $pl:expr, $args:expr, $rts:expr, $k:ident )+ ) => {
       
#[allow(non_camel_case_types)] 
#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum IrFn {
    $( $k ),+
}


impl IrFn {

    pub fn from_name(s: &str) -> Option<(IrFn, Bytecode, usize, usize, usize)> {
        Some(match s {
            $(
                stringify!($k) => (IrFn::$k, $c, $pl, $args, $rts),
            )+
            _ => return None
        })
    }

}

    }
}



irfn_define!{

    CALLDYN    : 0, 3, 1,     call_dynamic
    
    MOVE       : 1, 0, 0,     local_move   

    TYPEID     : 0, 1, 1,     type_id        
    CHIOSE     : 0, 3, 1,     chiose         
    SIZE       : 0, 1, 1,     size           
    CAT        : 0, 2, 1,     concat         
    BYTE       : 0, 2, 1,     byte           
    CUT        : 0, 3, 1,     buffer_cut     
    LEFT       : 1, 1, 1,     buffer_left    
    RIGHT      : 1, 1, 1,     buffer_right   
    INC        : 1, 1, 1,     increase       
    DEC        : 1, 1, 1,     decrease       

    HGROW      : 1, 0, 0,     heap_grow      
    HWRITE     : 0, 2, 0,     heap_write     
    HREAD      : 0, 2, 1,     heap_read      

    ALLOC      : 1, 0 ,0,     local_alloc

    SRENT      : 0, 1, 0,     storage_rent   
    SRCV       : 0, 2, 0,     storage_recover
    SDEL       : 0, 1, 0,     storage_delete 
    SSAVE      : 0, 2, 0,     storage_save   
    SLOAD      : 0, 1, 1,     storage_load   

    MGET       : 0, 1, 1,     memory_get     
    MPUT       : 0, 2, 0,     memory_put     
    GGET       : 0, 1, 1,     global_get     
    GPUT       : 0, 2, 0,     global_put     

    BURN       : 2, 0, 0,     gas_burn       

}





/********************************/



#[derive(Default, Eq, PartialEq)]
#[repr(u8)]
pub enum TokenType {
    #[default] 
    Blank,  // \s\n\t\r
    Word,   // _a~zA~Z0~9
    Number, // 0~9 x b . 
    Str,
    StrEsc,
    Split,  // () {} []
    Symbol, // +-*/|&
}

/********************************/



#[derive(Debug, Eq, PartialEq)]
pub enum Token {
    Keyword(KwTy),
    Operator(OpTy),
    Partition(char),
    Identifier(String),
    Integer(u128),
    Bytes(Vec<u8>),
}




