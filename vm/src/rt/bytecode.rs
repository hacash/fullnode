
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
    ________________32  = 0x20, // CALLDYN arg,fnsg,addr + 
    ________________33  = 0x21, // *,****@    
    ________________34  = 0x22, //   ****@ 
    ________________35  = 0x23, // *,****@ 
    ________________36  = 0x24, // *,****@     
    ________________37  = 0x25, // *,****  
    ________________38  = 0x26,
    TNIL                = 0x27, // & is nil push Bool(true)
    ________________40  = 0x28,   
    ________________41  = 0x29,
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
    TYPEID              = 0x37, // &      type id
    PU8                 = 0x38, // *+     push u8
    PU16                = 0x39, // **+    push u16
    P0                  = 0x3a, // +      push u8 0
    P1                  = 0x3b, // +      push 8 1
    PNBUF               = 0x3c, // +      push buf empty
    PBUFL               = 0x3d, // **+    push buf long
    PBUF                = 0x3e, // *+     push buf
    ________________63  = 0x3f,
    DUP                 = 0x40, // +      copy 0
    DUPX                = 0x41, // *+     copy u8
    POP                 = 0x42, // a      pop top
    POPX                = 0x43, // *a...b pop n
    SWAP                = 0x44, // a,b++  swap  b,a = a,b
    REV                 = 0x45, // a...b  reverse u8
    CHOISE              = 0x46, // a,b,c+ (x ? a : b)
    SIZE                = 0x47, // &      size
    CAT                 = 0x48, // a,b+   buf: b + a
    JOIN                = 0x49, // a...bn+
    BYTE                = 0x4a, // a,b+   val[n] = u8
    CUT                 = 0x4b, // a,b,c+ cut buf (v, ost, len)+
    LEFT                = 0x4c, // *&     cut left  buf *
    RIGHT               = 0x4d, // *&     cut right buf *
    LDROP               = 0x4e, // *&     drop buf left *
    ________________79  = 0x4f,
    ________________80  = 0x50,
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
    HGROW               = 0x70, // *      heap grow
    HWRITE              = 0x71, // a,b    heap write
    HREAD               = 0x72, // a,b+   heap read
    HWRITEX             = 0x73, // *+     heap write x
    HWRITEXL            = 0x74, // **+    heap write xl
    HREADU              = 0x75, // *+     heap read u
    HREADUL             = 0x76, // **+    heap read ul
    ________________119 = 0x77,
    XLG                 = 0x78, // *&    local logic
    XOP                 = 0x79, // *a    local operand
    GET                 = 0x7a, // &     local get       
    PUT                 = 0x7b, // a,b   local put      
    GETX                = 0x7c, // *+    local getx 
    PUTX                = 0x7d, // *a    local putx       
    MOVE                = 0x7e, // *     move one to local from ops
    ALLOC               = 0x7f, // *     local alloc
    SRENT               = 0x80, // a,b   storage time rent
    SSAVE               = 0x81, // a,b   storage save
    SDEL                = 0x82, // a     storage delete
    SLOAD               = 0x83, // &     storage load
    STIME               = 0x84, // &     storage expire block
    ________________133 = 0x85,
    ________________134 = 0x86,
    ________________135 = 0x87,
    ________________236 = 0x88,
    ________________237 = 0x89,
    ________________238 = 0x8a,
    ________________239 = 0x8b,
    MGET                = 0x8c, // &     memory get
    MPUT                = 0x8d, // a,b   memory put
    GGET                = 0x8e, // &     global get
    GPUT                = 0x8f, // a,b   global put
    AND                 = 0x90, // a,b+   amd
    OR                  = 0x91, // a,b+   or
    EQ                  = 0x92, // a,b+   equal
    NEQ                 = 0x93, // a,b+   not equal
    LT                  = 0x94, // a,b+   less than
    GT                  = 0x95, // a,b+   great than
    LE                  = 0x96, // a,b+   less and eq
    GE                  = 0x97, // a,b+   great and eq
    NOT                 = 0x98, // a+   not
    ________________121 = 0x99,
    ________________122 = 0x9a,
    BSHR                = 0x9b, // a,b+   shr: >>
    BSHL                = 0x9c, // a,b+   shl: <<
    BXOR                = 0x9d, // a,b+   xor: ^
    BOR                 = 0x9e, // a,b+   or:  |
    BAND                = 0x9f, // a,b+   and: &
    ADD                 = 0xa0, // a,b+   +
    SUB                 = 0xa1, // a,b+   -
    MUL                 = 0xa2, // a,b+   *
    DIV                 = 0xa3, // a,b+   /
    MOD                 = 0xa4, // a,b+   mod
    POW                 = 0xa5, // a,b+   pow
    MAX                 = 0xa6, // a,b+   max
    MIN                 = 0xa7, // a,b+   min
    INC                 = 0xa8, // *&     += u8
    DEC                 = 0xa9, // *&     -= u8
    ________________138 = 0xaa, // a,b,c+ x+y%z
    ________________139 = 0xab, // a,b,c+ x*y%z
    ________________140 = 0xac,
    ________________141 = 0xad,
    ________________142 = 0xae,
    ________________143 = 0xaf,
    ________________144 = 0xb0,
    ________________145 = 0xb1,
    ________________146 = 0xb2,
    ________________147 = 0xb3,
    ________________148 = 0xb4,
    ________________149 = 0xb5,
    ________________150 = 0xb6,
    ________________151 = 0xb7,
    ________________152 = 0xb8,
    ________________153 = 0xb9,
    ________________154 = 0xba,
    ________________155 = 0xbb,
    ________________156 = 0xbc,
    ________________157 = 0xbd,
    ________________158 = 0xbe,
    ________________159 = 0xbf,
    ________________160 = 0xc0,
    ________________161 = 0xc1,
    ________________162 = 0xc2,
    ________________163 = 0xc3,
    ________________164 = 0xc4,
    ________________165 = 0xc5,
    ________________166 = 0xc6,
    ________________167 = 0xc7,
    ________________168 = 0xc8,
    ________________169 = 0xc9,
    ________________170 = 0xca,
    ________________171 = 0xcb,
    ________________172 = 0xcc,
    ________________173 = 0xcd,
    ________________174 = 0xce,
    ________________175 = 0xcf,
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

    // CALLDYN    :   0, 3, 1,   call_dynamic
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
    CHOISE     : 0, 3, 1,     choise
    SIZE       : 0, 1, 1,     size
    CAT        : 0, 2, 1,     concat
    JOIN       : 0, 255, 1,   join_bytes
    BYTE       : 0, 2, 1,     byte
    CUT        : 0, 3, 1,     buffer_cut
    LEFT       : 1, 1, 1,     buffer_left
    RIGHT      : 1, 1, 1,     buffer_right
    LDROP      : 1, 1, 1,     buffer_left_drop

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

    XLG        : 1, 1, 1,     logic       //  local_      
    XOP        : 1, 1, 0,     operand     //  local_      
    GETX       : 1, 0, 1,     get_x       //  local_        
    PUTX       : 1, 1, 0,     put_x       //  local_     
    GET        : 0, 1, 1,     get         //  local_       
    PUT        : 0, 2, 0,     put         //  local_     
    MOVE       : 1, 0, 0,     local_move  //  local_         
    ALLOC      : 1, 0 ,0,     local_alloc //  local_        
    SRENT      : 0, 2, 0,     storage_rent
    SSAVE      : 0, 2, 0,     storage_save
    SDEL       : 0, 1, 0,     storage_del
    SLOAD      : 0, 1, 1,     storage_load
    STIME      : 0, 1, 1,     storage_time

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


