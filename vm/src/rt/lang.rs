
/********************************/


macro_rules! keyword_define {
    ( $( $k:ident : $s:expr  )+ ) => {


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum KwTy {
    $( $k ),+
}


impl KwTy {

    pub fn build(s: &str) -> Ret<Self> {
        Ok(match s {
            $( $s => Self::$k, )+
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
    LineCms   : "//"
    BlkCmsL   : "/*"
    BlkCmsR   : "*/"
    Progma    : "progma"
    Use       : "use"
    Lib       : "lib"
    Let       : "let"
    Var       : "var"
    If        : "if"
    Else      : "else"
    While     : "while"
    Finish    : "finish"
    Return    : "return"
    Abort     : "abort"
    Throw     : "throw"
    Assert    : "assert"
    CallCode  : "callcode"
    ByteCode  : "bytecode"
    And       : "and"
    Or        : "or"
    Not       : "not"
    As        : "as"
    Is        : "is"
    Nil       : "nil"
    Bool      : "bool"
    True      : "true"
    False     : "false"
    U8        : "u8"
    U16       : "u16"
    U32       : "u32"
    U64       : "u64"
    U128      : "u128"
    U256      : "u256"
    Uint      : "uint"
    Bytes     : "bytes"
    Address   : "address"
    List      : "list"
    Map       : "map"
    Struct    : "struct"
    Tuple     : "tuple"

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

    EXTACTION  : 1, 1, 1,     ext_action
    // EXTENV     : 1, 0, 1,     ext_env
    // EXTFUNC    : 1, 1, 1,     ext_func

    // CALLDYN    :   0, 3, 1,   call_dynamic
    // CALL       : 1+4, 1, 1,   call
    // CALLINR    :   4, 1, 1,   call_inner
    // CALLLIB    : 1+4, 1, 1,   call_library
    // CALLSTATIC : 1+4, 1, 1,   call_static
    // CALLCODE   : 1+4, 0, 0,   call_code

    // NTCALL     : 1, 1, 1,     native_call

    PU8        : 1, 0, 1,     push_u8
    PU16       : 2, 0, 1,     push_u16
    PBUF       : 1, 0, 1,     push_buf
    PBUFL      : 2, 0, 1,     push_buf_long
    P0         : 0, 0, 1,     push_0
    P1         : 0, 0, 1,     push_1
    P2         : 0, 0, 1,     push_2
    P3         : 0, 0, 1,     push_3
    PNBUF      : 0, 0, 1,     push_empty_buf
    PNIL       : 0, 0, 1,     push_nil

    CU8        : 0, 1, 1,     cast_u8
    CU16       : 0, 1, 1,     cast_u16
    CU32       : 0, 1, 1,     cast_u32
    CU64       : 0, 1, 1,     cast_u64
    CU128      : 0, 1, 1,     cast_u128
    CBUF       : 0, 1, 1,     cast_bytes
    CTO        : 1, 1, 1,     cast_to
    TNIL       : 0, 1, 1,     type_is_nil
    TLIST      : 0, 1, 1,     type_is_list
    TMAP       : 0, 1, 1,     type_is_map
    TIS        : 1, 1, 1,     type_is
    TID        : 0, 1, 1,     type_id

    DUP        : 0, 0, 1,     dump
    DUPX       : 1, 0, 1,     dump_x
    // POP        : 0, 255, 0,   pop
    // POPN       : 1, 255, 0,   pop_n
    PICK       : 1, 0, 1,     pick
    SWAP       : 0, 2, 2,     swap
    // REV        : 0, 255, 255, reverse_stace
    CHOISE     : 0, 3, 1,     choise
    SIZE       : 0, 1, 1,     size
    CAT        : 0, 2, 1,     concat
    // JOIN       : 0, 255, 1,   join
    CUT        : 0, 3, 1,     buf_cut
    LEFT       : 1, 1, 1,     buf_left
    RIGHT      : 1, 1, 1,     buf_right
    LDROP      : 1, 1, 1,     buf_left_drop
    RDROP      : 1, 1, 1,     buf_right_drop
    BYTE       : 0, 2, 1,     byte

    NEWLIST    : 0, 0, 1,     new_list
    NEWMAP     : 0, 0, 1,     new_map
    // PACKLIST   : 0, 255, 1,   new_list
    // PACKMAP    : 0, 255, 1,   new_map
    INSERT     : 0, 3, 1,     insert
    REMOVE     : 0, 2, 1,     remove
    CLEAR      : 0, 1, 1,     clear
    MERGE      : 0, 2, 1,     merge
    LENGTH     : 0, 1, 1,     length
    HASKEY     : 0, 2, 1,     has_key
    ITEMGET    : 0, 2, 1,     item_get
    KEYS       : 0, 1, 1,     keys
    VALUES     : 0, 1, 1,     values
    HEAD       : 0, 1, 1,     head
    TAIL       : 0, 1, 1,     tail
    APPEND     : 0, 2, 1,     append
    CLONE      : 0, 1, 1,     clone
    UPLIST     : 0, 2, 0,     unpack_list

    XLG        : 1, 1, 1,     local_logic    
    XOP        : 1, 1, 0,     local_operand         
    ALLOC      : 1, 0 ,0,     local_alloc       
    PUTX       : 0, 2, 0,     local_x_put          
    GETX       : 0, 1, 1,     local_x              
    PUT        : 1, 1, 0,     local_put          
    GET        : 1, 0, 1,     local          
    GET0       : 0, 0, 1,     local_0        
    GET1       : 0, 0, 1,     local_1        
    GET2       : 0, 0, 1,     local_2   
    GET3       : 0, 0, 1,     local_3    

    HSLICE     : 0, 2, 1,     heap_slice
    HREADUL    : 2, 0, 1,     heap_read_uint_long
    HREADU     : 1, 0, 1,     heap_read_uint
    HWRITEXL   : 2, 0, 1,     heap_write_xl
    HWRITEX    : 1, 0, 1,     heap_write_x
    HREAD      : 0, 2, 1,     heap_read
    HWRITE     : 0, 2, 0,     heap_write
    HGROW      : 1, 0, 0,     heap_grow

    GPUT       : 0, 2, 0,     global_put
    GGET       : 0, 1, 1,     global_get
    MPUT       : 0, 2, 0,     memory_put
    MGET       : 0, 1, 1,     memory_get
        
    STIME      : 0, 1, 1,     storage_time
    SLOAD      : 0, 1, 1,     storage_load
    SDEL       : 0, 1, 0,     storage_del
    SSAVE      : 0, 2, 0,     storage_save
    SRENT      : 0, 2, 0,     storage_rent

    // AND        : 0, 2, 1,     and
    // OR         : 0, 2, 1,     or
    // EQ         : 0, 2, 1,     equal
    // NEQ        : 0, 2, 1,     not_equal
    // LT         : 0, 2, 1,     less_than
    // GT         : 0, 2, 1,     more_than  
    // LE         : 0, 2, 1,     less_equal
    // GE         : 0, 2, 1,     more_equal
    // NOT        : 0, 1, 1,     not

    BSHR       : 0, 2, 1,     bit_shr
    BSHL       : 0, 2, 1,     bit_shl
    BXOR       : 0, 2, 1,     bit_xor
    BOR        : 0, 2, 1,     bit_or
    BAND       : 0, 2, 1,     bit_and

    // ADD        : 0, 2, 1,     add
    // SUB        : 0, 2, 1,     sub
    // MUL        : 0, 2, 1,     mul
    // DIV        : 0, 2, 1,     div
    // MOD        : 0, 2, 1,     mod
    // POW        : 0, 2, 1,     pow
    MAX        : 0, 2, 1,     max
    MIN        : 0, 2, 1,     min
    INC        : 1, 1, 1,     increase
    DEC        : 1, 1, 1,     decrease

    // JMPL       : 2, 0, 0,     jump_long
    // JMPS       : 1, 0, 0,     jump_offset
    // JMPSL      : 2, 0, 0,     jump_offset_long
    // BRL        : 2, 1, 0,     branch_long
    // BRS        : 1, 1, 0,     branch_offset
    // BRSL       : 2, 1, 0,     branch_offset_long
    // BRSLN      : 2, 1, 0,     branch_offset_long_not

    // RET        : 0, 1, 0,     return
    // END        : 0, 0, 0,     end
    // AST        : 0, 1, 0,     assert
    // ERR        : 0, 1, 0,     throw
    // ABT        : 0, 0, 0,     abort

    // IRCODE     : 2, 255, 0,   ir_code
    // IRBLOCK    : 2, 255, 0,   ir_block
    // IRIF       : 0, 3, 0,     ir_if
    // IRWHILE    : 0, 2, 0,     ir_while

    // BURN       : 2, 0, 0,     gas_burn
    // NOP        : 0, 0, 0,     nop
    // NT         : 0, 0, 0,     never_touch

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



#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Token {
    Keyword(KwTy),
    Operator(OpTy),
    Partition(char),
    Identifier(String),
    Integer(u128),
    Bytes(Vec<u8>),
    Addr(Address),
}


