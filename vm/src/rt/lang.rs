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
            _ => return errf!("unsupported keyword '{}'", s)
        })
    }
}

    }
}

keyword_define! {
    Arrow     : "->"
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
    Pragma    : "pragma"
    Use       : "use"
    Lib       : "lib"
    Ext       : "ext"
    Let       : "let"
    Bind      : "bind"
    Const     : "const"
    Var       : "var"
    Log       : "log"
    If        : "if"
    Else      : "else"
    While     : "while"
    Break     : "break"
    Continue  : "continue"
    End       : "end"
    Return    : "return"
    Abort     : "abort"
    Throw     : "throw"
    Assert    : "assert"
    Print     : "print"
    Call      : "call"
    CallExt      : "callext"
    CallExtView  : "callextview"
    CallUseView  : "calluseview"
    CallUsePure  : "callusepure"
    CallThis     : "callthis"
    CallSelf     : "callself"
    CallSuper    : "callsuper"
    CallSelfView : "callselfview"
    CallSelfPure : "callselfpure"
    CodeCall     : "codecall"
    ByteCode  : "bytecode"
    IrCode    : "ircode"
    Contract  : "contract"
    Library   : "library"
    Inherit   : "inherit"
    Abstract  : "abstract"
    Function  : "function"
    External  : "external"
    Inner     : "inner"
    Edit      : "edit"
    View      : "view"
    Pure      : "pure"
    Virtual   : "virtual"
    Deploy    : "deploy"
    Param     : "param"
    This      : "this"
    Self_     : "self"
    Upper     : "upper"
    Super     : "super"
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
            _ => return errf!("unsupported operator '{}'", s)
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

    pub fn is_right_assoc(&self) -> bool {
        matches!(self, OpTy::POW)
    }

    pub fn next_min_prec(&self) -> u8 {
        if self.is_right_assoc() {
            self.level()
        } else {
            self.level().saturating_add(1)
        }
    }
}


    }
}

operator_define! {
    NOT       : "!" ,     13
    POW       : "**",     12
    MUL       : "*" ,     11
    DIV       : "/" ,     11
    MOD       : "%" ,     11
    ADD       : "+" ,     10
    SUB       : "-" ,     10
    BSHL      : "<<",     9
    BSHR      : ">>",     9
    GE        : ">=",     8
    LE        : "<=",     8
    GT        : ">" ,     8
    LT        : "<" ,     8
    EQ        : "==",     7
    NEQ       : "!=",     7
    BAND      : "&" ,     6
    BXOR      : "^" ,     5
    BOR       : "|" ,     4
    AND       : "&&",     3
    OR        : "||",     2
    CAT       : "++",     1
}

/********************************/

macro_rules! irfn_define {
    ( $( $k:ident )+ ) => {

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum IrFn {
    $( $k ),+
}


pub fn pick_ir_func(s: &str) -> Option<(IrFn, Bytecode, usize, usize, usize)> {
    Some(match s {
        $(
            stringify!($k) => {
                let sig = bytecode_intro_sig!($k);
                (IrFn::$k, sig.0, sig.1, sig.2, sig.3)
            },
        )+
        _ => return None
    })
}


impl IrFn {

    pub fn from_name(s: &str) -> Option<(IrFn, Bytecode, usize, usize, usize)> {
        pick_ir_func(s)
    }

}

    }
}

irfn_define! {

    // Direct IR calls only: keep source-level functions that are not already covered
    // by dedicated syntax, action/native dispatch, or tuple/list literal sugar.
    cast_to
    type_is
    type_id

    roll_0
    roll
    byte
    buf_cut
    buf_left
    buf_right
    buf_left_drop
    buf_right_drop
    size
    choose

    new_list
    new_map
    tuple_to_list
    insert
    remove
    clear
    merge
    length
    has_key
    item_get
    keys
    values
    take_first
    take_last
    append
    clone
    unpack

    // Keep frame-local helpers so sourcemap-free decompilation can still roundtrip.
    local_alloc
    local_x_put
    local_x

    heap_read_uint_long
    heap_read_uint
    heap_write_xl
    heap_write_x
    heap_read
    heap_write
    heap_grow

    global_put
    global_get
    memory_put
    memory_get
    memory_take
    status_put
    status_get

    storage_new
    storage_recv
    storage_stat
    storage_load
    storage_del
    storage_edit
    storage_rent

    div_up
    div_exact_op
    add_mod
    mul_mod
    mul_div
    mul_div_up
    mul_add
    mul_sub
    clamp
    abs_diff
    max
    min
    increase
    decrease
}
/* ******************************* #[derive(Default, Eq, PartialEq)] #[repr(u8)] pub enum TokenType { #[default] Blank,  // \s\n\t\r Word,   // _a~zA~Z0~9 Number, // 0~9 x b . Str, StrEsc, Split,  // () {} [] Symbol, // +-* /|& } */

/********************************/

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Token {
    Keyword(KwTy),
    Operator(OpTy),
    Partition(char),
    Identifier(String),
    Integer(u128),
    IntegerWithSuffix(u128, KwTy),
    Character(u8),
    Bytes(Vec<u8>),
    Address(Address),
}

