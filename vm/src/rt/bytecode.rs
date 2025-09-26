
/*
    Bytecode define

    Add one bytecode

*/


#[repr(u8)]
#[derive(Default, PartialEq, Debug, Clone, Copy)]
pub enum Bytecode {
    #[default]
    EXTACTION           = 0x00, // *@  call extend action
    ________________1   = 0x01,
    ________________2   = 0x02,
    ________________3   = 0x03,
    ________________4   = 0x04,
    ________________5   = 0x05,
    ________________6   = 0x06,
    ________________7   = 0x07,
    ________________8   = 0x08,
    ________________9   = 0x09,
    ________________10  = 0x0a,
    EXTENV              = 0x0b, // *+  call extend action
    ________________12  = 0x0c,
    ________________13  = 0x0d,
    ________________14  = 0x0e,
    EXTFUNC             = 0x0f, // *@  call extend action
    ________________16  = 0x10,
    CALL                = 0x11, // *,****@ 
    CALLINR             = 0x12, //   ****@ 
    CALLLIB             = 0x13, // *,****@ 
    CALLSTATIC          = 0x14, // *,****@ 
    CALLCODE            = 0x15, // *,****  
    ________________22  = 0x16,
    ________________23  = 0x17,
    ________________24  = 0x18,
    ________________25  = 0x19,
    ________________26  = 0x1a,
    ________________27  = 0x1b,
    ________________28  = 0x1c,
    ________________29  = 0x1d,
    ________________30  = 0x1e,
    NTCALL              = 0x1f, // *@  native call
    PU8                 = 0x20, // *+     push u8
    PU16                = 0x21, // **+    push u16
    PBUF                = 0x22, // *+     push buf
    PBUFL               = 0x23, // **+    push buf long  
    P0                  = 0x24, // +      push u8 0
    P1                  = 0x25, // +      push u8 1
    P2                  = 0x26, // +      push u8 2
    P3                  = 0x27, // +      push u8 3
    PNIL                = 0x28, // +      push nil
    PNBUF               = 0x29, // +      push buf empty
    ________________42  = 0x2a,
    ________________43  = 0x2b,       
    ________________44  = 0x2c,       
    ________________45  = 0x2d,          
    ________________46  = 0x2e,       
    ________________47  = 0x2f,     
    CU8                 = 0x30, // &      cast u8
    CU16                = 0x31, // &      cast u16
    CU32                = 0x32, // &      cast u32
    CU64                = 0x33, // &      cast u64
    CU128               = 0x34, // &      cast u128
    ________________53  = 0x35,
    CBUF                = 0x36, // &      cast buf
    CTO                 = 0x37, // *&     cast to
    TNIL                = 0x38, // &   is nil push Bool(true)
    TLIST               = 0x39, // &   is compo list push Bool(true)
    TMAP                = 0x3a, // &   is compo map  push Bool(true)
    TIS                 = 0x3b, // *&  is type id
    TID                 = 0x3c, // &   type id
    ________________61  = 0x3d,
    ________________62  = 0x3e,       
    ________________63  = 0x3f,   
    DUP                 = 0x40, // +      copy 0
    DUPX                = 0x41, // *+     copy u8
    POP                 = 0x42, // a      pop top
    POPN                = 0x43, // *a...b pop n
    PICK                = 0x44, // *      pick
    SWAP                = 0x45, // a,b++  swap  b,a = a,b
    REV                 = 0x46, // a...b  reverse u8
    CHOISE              = 0x47, // a,b,c+ (x ? a : b)
    SIZE                = 0x48, // &      size (u16)
    CAT                 = 0x49, // a,b+   buf: b + a
    JOIN                = 0x4a, // a...bn+
    CUT                 = 0x4b, // a,b,c+ cut buf (v, ost, len)+
    LEFT                = 0x4c, // *&     cut left  buf *
    RIGHT               = 0x4d, // *&     cut right buf *
    LDROP               = 0x4e, // *&     drop buf left *
    RDROP               = 0x4f, // *&     drop buf right *
    BYTE                = 0x50, // a,b+   val[n] = u8
    ________________81  = 0x51,
    ________________82  = 0x52,
    ________________83  = 0x53,
    ________________84  = 0x54,
    ________________85  = 0x55,
    ________________86  = 0x56,
    ________________87  = 0x57,
    ________________88  = 0x58,
    ________________89  = 0x59,
    ________________90  = 0x5a,
    ________________91  = 0x5b,
    ________________92  = 0x5c,
    ________________93  = 0x5d,
    ________________94  = 0x5e,
    ________________95  = 0x5f,
    NEWLIST             = 0x60, // + new compo list
    NEWMAP              = 0x61, // + new compo map
    PACKLIST            = 0x62, // (v...,n)+ pack compo list
    PACKMAP             = 0x63, // (v...,n)+ pack compo map
    INSERT              = 0x64, // t,k,v+  compo insert
    REMOVE              = 0x65, // t,k+    compo remove
    CLEAR               = 0x66, // t+      compo clear
    MERGE               = 0x67, // a,b+    compo merge
    LENGTH              = 0x68, // t+      compo length
    HASKEY              = 0x69, // t,k+    compo check has key
    ITEMGET             = 0x6a, // t,k+    compo iten get
    KEYS                = 0x6b, // &       compo keys
    VALUES              = 0x6c, // &       compo values
    HEAD                = 0x6d, // &       compo pick last
    TAIL                = 0x6e, // &       compo pick last
    APPEND              = 0x6f, // &       compo append
    CLONE               = 0x70, // a++     compo clone
    _______________113  = 0x71,
    UPLIST              = 0x72, // a       up pack list to local
    _______________115  = 0x73,
    _______________116  = 0x74,
    _______________117  = 0x75,
    _______________118  = 0x76,
    _______________119  = 0x77,
    _______________120  = 0x78,
    _______________121  = 0x79,
    _______________122  = 0x7a,
    _______________123  = 0x7b,
    _______________124  = 0x7c,
    XLG                 = 0x7d, // *&    local logic
    XOP                 = 0x7e, // *a    local operand
    ALLOC               = 0x7f, // *     local allocQ 
    PUTX                = 0x80, // v,i   local x put     
    GETX                = 0x81, // &     local x get  
    PUT                 = 0x82, // *a,b  local put       
    GET                 = 0x83, // *+    local get  
    GET0                = 0x84, // +     local get idx 0    
    GET1                = 0x85, // +     local get idx 1     
    GET2                = 0x86, // +     local get idx 2
    GET3                = 0x87, // +     local get idx 3
    HSLICE              = 0x88, // a,b+  create heap slice
    HREADUL             = 0x89, // **+   heap read ul
    HREADU              = 0x8a, // *+    heap read u
    HWRITEXL            = 0x8b, // **+   heap write xl
    HWRITEX             = 0x8c, // *+    heap write x
    HREAD               = 0x8d, // a,b+  heap read
    HWRITE              = 0x8e, // a,b   heap write
    HGROW               = 0x8f, // *     heap grow
    GGET                = 0x90, // &     global get
    GPUT                = 0x91, // a,b   global put
    MGET                = 0x92, // &     memory get
    MPUT                = 0x93, // a,b   memory put
    ________________148 = 0x94,
    ________________149 = 0x95,
    ________________150 = 0x96,
    ________________151 = 0x97,
    ________________152 = 0x98,
    ________________153 = 0x99,
    ________________154 = 0x9a,
    STIME               = 0x9b, // &     storage expire block
    SLOAD               = 0x9c, // &     storage load
    SDEL                = 0x9d, // a     storage delete
    SSAVE               = 0x9e, // a,b   storage save
    SRENT               = 0x9f, // a,b   storage time rent
    AND                 = 0xa0, // a,b+   amd
    OR                  = 0xa1, // a,b+   or
    EQ                  = 0xa2, // a,b+   equal
    NEQ                 = 0xa3, // a,b+   not equal
    LT                  = 0xa4, // a,b+   less than
    GT                  = 0xa5, // a,b+   great than
    LE                  = 0xa6, // a,b+   less and eq
    GE                  = 0xa7, // a,b+   great and eq
    NOT                 = 0xa8, // a+   not
    ________________169 = 0xa9,
    ________________170 = 0xaa,
    BSHR                = 0xab, // a,b+   shr: >>
    BSHL                = 0xac, // a,b+   shl: <<
    BXOR                = 0xad, // a,b+   xor: ^
    BOR                 = 0xae, // a,b+   or:  |
    BAND                = 0xaf, // a,b+   and: &
    ADD                 = 0xb0, // a,b+   +
    SUB                 = 0xb1, // a,b+   -
    MUL                 = 0xb2, // a,b+   *
    DIV                 = 0xb3, // a,b+   /
    MOD                 = 0xb4, // a,b+   mod
    POW                 = 0xb5, // a,b+   pow
    MAX                 = 0xb6, // a,b+   max
    MIN                 = 0xb7, // a,b+   min
    INC                 = 0xb8, // *&     += u8
    DEC                 = 0xb9, // *&     -= u8
    ________________186 = 0xba, // a,b,c+ x+y%z
    ________________187 = 0xbb, // a,b,c+ x*y%z
    ________________188 = 0xbc,
    ________________189 = 0xbd,
    ________________190 = 0xbe,
    ________________191 = 0xbf,
    ________________192 = 0xc0,
    ________________193 = 0xc1,
    ________________194 = 0xc2,
    ________________195 = 0xc3,
    ________________196 = 0xc4,
    ________________197 = 0xc5,
    ________________198 = 0xc6,
    ________________199 = 0xc7,
    ________________200 = 0xc8,
    ________________201 = 0xc9,
    ________________202 = 0xca,
    ________________203 = 0xcb,
    ________________204 = 0xcc,
    ________________205 = 0xcd,
    ________________206 = 0xce,
    ________________207 = 0xcf,
    ________________208 = 0xd0,
    ________________209 = 0xd1,
    ________________210 = 0xd2,
    ________________211 = 0xd3,
    ________________212 = 0xd4,
    ________________213 = 0xd5,
    ________________214 = 0xd6,
    ________________215 = 0xd7,
    ________________216 = 0xd8,
    ________________217 = 0xd9,
    ________________218 = 0xda,
    ________________219 = 0xdb,
    ________________220 = 0xdc,
    ________________221 = 0xdd,
    ________________222 = 0xde,
    ________________223 = 0xdf,
    JMPL                = 0xe0, // **    jump long
    JMPS                = 0xe1, // *     jump offset
    JMPSL               = 0xe2, // **    jump offset long
    BRL                 = 0xe3, // **a   branch long
    BRS                 = 0xe4, // *a    branch offset
    BRSL                = 0xe5, // **a   branch offset long not_zero
    BRSLN               = 0xe6, // **a   branch offset long is_zero
    ________________231 = 0xe7, 
    ________________232 = 0xe8,
    ________________233 = 0xe9,
    ________________234 = 0xea,
    AST                 = 0xeb, // c     assert throw
    ERR                 = 0xec, // a     throw (ERR)
    ABT                 = 0xed, //       abord
    RET                 = 0xee, // a     func return (DATA)
    END                 = 0xef, //       func return nil
    IRCODE              = 0xf0, // <IR NODE>
    IRBLOCK             = 0xf1, // <IR NODE>
    IRIF                = 0xf2, // <IR NODE>
    IRWHILE             = 0xf3, // <IR NODE>
    ________________244 = 0xf4,
    ________________245 = 0xf5,
    ________________246 = 0xf6,
    ________________247 = 0xf7,
    ________________248 = 0xf8,
    ________________249 = 0xf9,
    ________________250 = 0xfa,
    ________________251 = 0xfb,
    ________________252 = 0xfc,
    BURN                = 0xfd, // **    burn gas
    NOP                 = 0xfe, //       do nothing
    NT                  = 0xff, //       panic: never touch
} 

use Bytecode::*;

impl Into<u8> for Bytecode {
    fn into(self) -> u8 {
        self as u8
    }
}


#[derive(Default, Debug, Copy, Clone)]
pub struct BytecodeMetadata {
    pub valid: bool,
    pub param: u8,
    pub input: u8,
    pub otput: u8,
    pub intro: &'static str,
}

macro_rules! bytecode_metadata_define {
    ( $( $inst:ident : $p:expr, $i:expr, $o:expr , $s:ident)+ ) => {

impl Bytecode {

    pub fn metadata(&self) -> BytecodeMetadata {
        match self {
            $(
            $inst => BytecodeMetadata {valid: true, param: $p, input: $i, otput: $o, intro: stringify!($s)},
            )+
            _ => BytecodeMetadata::default(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            $(
            stringify!($inst) => Some($inst),
            )+
            _ => None
        }
    }

}

    };
}









/*
    params, stack input, stack output
*/
bytecode_metadata_define!{
    EXTACTION  : 1, 1, 1,     ext_action
    EXTENV     : 1, 0, 1,     ext_env
    EXTFUNC    : 1, 1, 1,     ext_func

    // CALLDYN    :   0, 3, 1,   call_dynamic
    CALL       : 1+4, 1, 1,   call
    CALLINR    :   4, 1, 1,   call_inner
    CALLLIB    : 1+4, 1, 1,   call_library
    CALLSTATIC : 1+4, 1, 1,   call_static
    CALLCODE   : 1+4, 0, 0,   call_code

    NTCALL     : 1, 1, 1,     native_call

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
    POP        : 0, 255, 0,   pop
    POPN       : 1, 255, 0,   pop_n
    PICK       : 1, 0, 1,     pick
    SWAP       : 0, 2, 2,     swap
    REV        : 0, 255, 255, reverse_stace
    CHOISE     : 0, 3, 1,     choise
    SIZE       : 0, 1, 1,     size
    CAT        : 0, 2, 1,     concat
    JOIN       : 0, 255, 1,   join
    CUT        : 0, 3, 1,     buf_cut
    LEFT       : 1, 1, 1,     buf_left
    RIGHT      : 1, 1, 1,     buf_right
    LDROP      : 1, 1, 1,     buf_left_drop
    RDROP      : 1, 1, 1,     buf_right_drop
    BYTE       : 0, 2, 1,     byte

    NEWLIST    : 0, 0, 1,     new_list
    NEWMAP     : 0, 0, 1,     new_map
    PACKLIST   : 0, 255, 1,   new_list
    PACKMAP    : 0, 255, 1,   new_map
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

    AND        : 0, 2, 1,     and
    OR         : 0, 2, 1,     or
    EQ         : 0, 2, 1,     equal
    NEQ        : 0, 2, 1,     not_equal
    LT         : 0, 2, 1,     less_than
    GT         : 0, 2, 1,     more_than  
    LE         : 0, 2, 1,     less_equal
    GE         : 0, 2, 1,     more_equal
    NOT        : 0, 1, 1,     not

    BSHR       : 0, 2, 1,     bit_shr
    BSHL       : 0, 2, 1,     bit_shl
    BXOR       : 0, 2, 1,     bit_xor
    BOR        : 0, 2, 1,     bit_or
    BAND       : 0, 2, 1,     bit_and

    ADD        : 0, 2, 1,     add
    SUB        : 0, 2, 1,     sub
    MUL        : 0, 2, 1,     mul
    DIV        : 0, 2, 1,     div
    MOD        : 0, 2, 1,     mod
    POW        : 0, 2, 1,     pow
    MAX        : 0, 2, 1,     max
    MIN        : 0, 2, 1,     min
    INC        : 1, 1, 1,     increase
    DEC        : 1, 1, 1,     decrease

    JMPL       : 2, 0, 0,     jump_long
    JMPS       : 1, 0, 0,     jump_offset
    JMPSL      : 2, 0, 0,     jump_offset_long
    BRL        : 2, 1, 0,     branch_long
    BRS        : 1, 1, 0,     branch_offset
    BRSL       : 2, 1, 0,     branch_offset_long
    BRSLN      : 2, 1, 0,     branch_offset_long_not

    RET        : 0, 1, 0,     return
    END        : 0, 0, 0,     end
    AST        : 0, 1, 0,     assert
    ERR        : 0, 1, 0,     throw
    ABT        : 0, 0, 0,     abort

    IRCODE     : 2, 255, 0,   ir_code
    IRBLOCK    : 2, 255, 0,   ir_block
    IRIF       : 0, 3, 0,     ir_if
    IRWHILE    : 0, 2, 0,     ir_while

    BURN       : 2, 0, 0,     gas_burn
    NOP        : 0, 0, 0,     nop
    NT         : 0, 0, 0,     never_touch

}


