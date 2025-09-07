
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
    EXTFUNC             = 0x06, // *@  call extend action
    EXTENV              = 0x07, // *+  call extend action
    ________________8   = 0x08,
    ________________9   = 0x09,
    ________________10  = 0x0a,
    ________________11  = 0x0b,
    ________________12  = 0x0c,
    ________________13  = 0x0d,
    ________________14  = 0x0e,
    ________________15  = 0x0f,
    ________________16  = 0x10,
    ________________17  = 0x11,
    ________________18  = 0x12,
    ________________19  = 0x13,
    ________________20  = 0x14,
    ________________21  = 0x15,
    ________________22  = 0x16,
    ________________23  = 0x17,
    ________________24  = 0x18,
    ________________25  = 0x19,
    ________________26  = 0x1a,
    ________________27  = 0x1b,
    ________________28  = 0x1c,
    ________________29  = 0x1d,
    ________________30  = 0x1e,
    ________________31  = 0x1f,
    CALLDYN             = 0x20, // arg,fnsg,addr +     
    CALL                = 0x21, // *,****@       
    CALLINR             = 0x22, //   ****@       
    CALLLIB             = 0x23, // *,****@          
    CALLSTATIC          = 0x24, // *,****@          
    CALLCODE            = 0x25, // *,****    
    ________________38  = 0x26,
    NTCALL              = 0x27, // *@  native call
    ________________40  = 0x28,
    ________________41  = 0x29,
    ________________42  = 0x2a,
    ________________43  = 0x2b,
    ________________44  = 0x2c,
    ________________45  = 0x2d,
    ________________46  = 0x2e,
    ________________47  = 0x2f,
    ________________48  = 0x30,
    ________________49  = 0x31,
    ________________50  = 0x32,
    ________________51  = 0x33,
    ________________52  = 0x34,
    ________________53  = 0x35,
    ________________54  = 0x36,
    ________________55  = 0x37,
    ________________56  = 0x38,
    ________________57  = 0x39,
    ________________58  = 0x3a,
    ________________59  = 0x3b,
    ________________60  = 0x3c,
    ________________61  = 0x3d,
    ________________62  = 0x3e,
    ________________63  = 0x3f,
    CU8                 = 0x40, // &      cast u8
    CU16                = 0x41, // &      cast u16
    CU32                = 0x42, // &      cast u32
    CU64                = 0x43, // &      cast u64
    CU128               = 0x44, // &      cast u128
    ________________69  = 0x45,
    CBUF                = 0x46, // &      cast buf
    TYPEID              = 0x47, // &      type id
    PU8                 = 0x48, // *+     push u8
    PU16                = 0x49, // **+    push u16
    P0                  = 0x4a, // +      push u8 0
    P1                  = 0x4b, // +      push 8 1
    PNBUF               = 0x4c, // +      push buf empty
    PBUFL               = 0x4d, // **+    push buf long
    PBUF                = 0x4e, // *+     push buf
    ________________79  = 0x4f,
    DUP                 = 0x50, // +      copy 0
    DUPX                = 0x51, // *+     copy u8
    POP                 = 0x52, // a      pop top
    POPX                = 0x53, // *a...b pop n
    SWAP                = 0x54, // a,b++  swap  b,a = a,b
    REV                 = 0x55, // a...b  reverse u8
    CHIOSE              = 0x56, // a,b,c+ (x ? a : b)
    SIZE                = 0x57, // &      size
    CAT                 = 0x58, // a,b+   buf: b + a
    JOIN                = 0x59, // a...bn+
    BYTE                = 0x5a, // a,b+   val[n] = u8
    CUT                 = 0x5b, // a,b,c+ cut buf (v, ost, len)+
    LEFT                = 0x5c, // *&     cut left  buf *
    RIGHT               = 0x5d, // *&     cut right buf *
    ________________94  = 0x5e,
    ________________95  = 0x5f,
    ________________96  = 0x60,
    ________________97  = 0x61,
    ________________98  = 0x62,
    ________________99  = 0x63,
    ________________100 = 0x64,
    ________________101 = 0x65,
    ________________102 = 0x66,
    ________________103 = 0x67,
    ________________104 = 0x68,
    ________________105 = 0x69,
    ________________106 = 0x6a,
    ________________107 = 0x6b,
    ________________108 = 0x6c,
    ________________109 = 0x6d,
    ________________110 = 0x6e,
    ________________111 = 0x6f,
    BAND                = 0x70, // a,b+   and: &
    BOR                 = 0x71, // a,b+   or:  |
    BXOR                = 0x72, // a,b+   xor: ^
    BSHL                = 0x73, // a,b+   shl: <<
    BSHR                = 0x74, // a,b+   shr: >>
    ________________117 = 0x75,
    ________________118 = 0x76,
    NOT                 = 0x77, // a+   not
    AND                 = 0x78, // a,b+   amd
    OR                  = 0x79, // a,b+   or
    EQ                  = 0x7a, // a,b+   equal
    NEQ                 = 0x7b, // a,b+   not equal
    LT                  = 0x7c, // a,b+   less than
    GT                  = 0x7d, // a,b+   great than
    LE                  = 0x7e, // a,b+   less and eq
    GE                  = 0x7f, // a,b+   great and eq
    ADD                 = 0x80, // a,b+   +
    SUB                 = 0x81, // a,b+   -
    MUL                 = 0x82, // a,b+   *
    DIV                 = 0x83, // a,b+   /
    MOD                 = 0x84, // a,b+   mod
    POW                 = 0x85, // a,b+   pow
    MAX                 = 0x86, // a,b+   max
    MIN                 = 0x87, // a,b+   min
    INC                 = 0x88, // *&     += u8
    DEC                 = 0x89, // *&     -= u8
    ________________138 = 0x8a, // a,b,c+ x+y%z
    ________________139 = 0x8b, // a,b,c+ x*y%z
    ________________140 = 0x8c,
    ________________141 = 0x8d,
    ________________142 = 0x8e,
    ________________143 = 0x8f,
    ________________144 = 0x90,
    ________________145 = 0x91,
    ________________146 = 0x92,
    ________________147 = 0x93,
    ________________148 = 0x94,
    ________________149 = 0x95,
    ________________150 = 0x96,
    ________________151 = 0x97,
    ________________152 = 0x98,
    ________________153 = 0x99,
    ________________154 = 0x9a,
    ________________155 = 0x9b,
    ________________156 = 0x9c,
    ________________157 = 0x9d,
    ________________158 = 0x9e,
    ________________159 = 0x9f,
    ________________160 = 0xa0,
    ________________161 = 0xa1,
    ________________162 = 0xa2,
    ________________163 = 0xa3,
    ________________164 = 0xa4,
    ________________165 = 0xa5,
    ________________166 = 0xa6,
    ________________167 = 0xa7,
    ________________168 = 0xa8,
    ________________169 = 0xa9,
    ________________170 = 0xaa,
    ________________171 = 0xab,
    ________________172 = 0xac,
    ________________173 = 0xad,
    ________________174 = 0xae,
    ________________175 = 0xaf,
    HGROW               = 0xb0, // *      heap grow
    HWRITE              = 0xb1, // a,b    heap write
    HREAD               = 0xb2, // a,b+   heap read
    HWRITEX             = 0xb3, // *+     heap write x
    HWRITEXL            = 0xb4, // **+    heap write xl
    HREADU              = 0xb5, // *+     heap read u
    HREADUL             = 0xb6, // **+    heap read ul
    ________________183 = 0xb7,
    XLG                 = 0xb8, // *&    local logic
    XOP                 = 0xb9, // *a    local operand
    MOVE                = 0xba, // *v... local move from ops
    GETX                = 0xbb, // *+    local getx
    PUTX                = 0xbc, // *a    local putx
    GET                 = 0xbd, // &     local get
    PUT                 = 0xbe, // a,b   local put
    ALLOC               = 0xbf, // *     local alloc
    SRENT               = 0xc0, // a     storage time rent
    SRCV                = 0xc1, // a,b   storage data recover
    SDEL                = 0xc2, // a     storage delete
    SSAVE               = 0xc3, // a,b   storage save
    SLOAD               = 0xc4, // &     storage load
    ________________197 = 0xc5,
    ________________198 = 0xc6,
    ________________199 = 0xc7,
    ________________200 = 0xc8,
    ________________201 = 0xc9,
    ________________202 = 0xca,
    ________________203 = 0xcb,
    MGET                = 0xcc, // &     memory get
    MPUT                = 0xcd, // a,b   memory put
    GGET                = 0xce, // &     global get
    GPUT                = 0xcf, // a,b   global put
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
    RET                 = 0xeb, // a     func return (DATA)
    END                 = 0xec, //       func return nil
    AST                 = 0xed, // c     assert throw
    ERR                 = 0xee, // a     throw (ERR)
    ABT                 = 0xef, //       abord
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


#[derive(Default, Copy, Clone)]
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
            $inst => BytecodeMetadata { valid: true, param: $p, input: $i, otput: $o, intro: stringify!($s) },
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
    EXTFUNC    : 1, 1, 1,     ext_func
    EXTENV     : 1, 0, 1,     ext_env

    CALLDYN    :   0, 3, 1,   call_dynamic
    CALL       : 1+4, 1, 1,   call
    CALLINR    :   4, 1, 1,   call_inner
    CALLLIB    : 1+4, 1, 1,   call_library
    CALLSTATIC : 1+4, 1, 1,   call_static
    CALLCODE   : 1+4, 0, 0,   call_codecopy

    NTCALL     : 1, 1, 1,     native_call

    CU8        : 0, 1, 1,     cast_u8
    CU16       : 0, 1, 1,     cast_u16
    CU32       : 0, 1, 1,     cast_u32
    CU64       : 0, 1, 1,     cast_u64
    CU128      : 0, 1, 1,     cast_u128

    CBUF       : 0, 1, 1,     cast_bytes
    TYPEID     : 0, 1, 1,     type_id
    PU8        : 1, 0, 1,     push_u8
    PU16       : 2, 0, 1,     push_u16
    P0         : 0, 0, 1,     push_0
    P1         : 0, 0, 1,     push_1
    PNBUF      : 0, 0, 1,     push_empty_bytes
    PBUFL      : 2, 0, 1,     push_bytes_long
    PBUF       : 1, 0, 1,     push_bytes

    DUP        : 0, 0, 1,     dump
    DUPX       : 1, 0, 1,     dump_stack
    POP        : 0, 1, 0,     pop_stack
    POPX       : 1, 255, 0,   pop_stack_num
    SWAP       : 0, 2, 2,     swap_stack
    REV        : 0, 255, 255, reverse_stace
    CHIOSE     : 0, 3, 1,     chiose
    SIZE       : 0, 1, 1,     size
    CAT        : 0, 2, 1,     concat
    JOIN       : 0, 255, 1,   join_bytes
    BYTE       : 0, 2, 1,     byte
    CUT        : 0, 3, 1,     buffer_cut
    LEFT       : 1, 1, 1,     buffer_left
    RIGHT      : 1, 1, 1,     buffer_right

    BAND       : 0, 2, 1,     bit_and
    BOR        : 0, 2, 1,     bit_or
    BXOR       : 0, 2, 1,     bit_xor
    BSHL       : 0, 2, 1,     bit_shl
    BSHR       : 0, 2, 1,     bit_shr

    NOT        : 0, 1, 1,     not
    AND        : 0, 2, 1,     and
    OR         : 0, 2, 1,     or
    EQ         : 0, 2, 1,     equal
    NEQ        : 0, 2, 1,     not_equal
    LT         : 0, 2, 1,     less_than
    GT         : 0, 2, 1,     more_than  
    LE         : 0, 2, 1,     less_equal
    GE         : 0, 2, 1,     more_equal

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

    HGROW      : 1, 0, 0,     heap_grow
    HWRITE     : 0, 2, 0,     heap_write
    HREAD      : 0, 2, 1,     head_read
    HWRITEX    : 1, 0, 1,     heap_write_x
    HWRITEXL   : 2, 0, 1,     heap_write_xl
    HREADU     : 1, 0, 1,     head_read_uint
    HREADUL    : 2, 0, 1,     head_read_uint_long

    XLG        : 1, 1, 1,     local_logic
    XOP        : 1, 1, 0,     local_operand
    MOVE       : 1, 255, 0,   local_move
    GETX       : 1, 0, 1,     get_x
    PUTX       : 1, 1, 0,     put_x
    GET        : 0, 1, 1,     get
    PUT        : 0, 2, 0,     put
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


