/* Bytecode define Add one bytecode */
//
// Multi-operand stack notation (when listed as `a,b,c+`):
//   left-to-right = stack bottom-to-top = handwritten IR child order (subx, suby, subz, ...).
//   `+` marks instructions that leave one combined result (often in-place on the bottom slot).
// See `vm/doc/operand-stack.md` for PACK*, XLG/XOP, PUTX/GETX, and in-place peek ops.
//
// range test: row 0 = protocol-family window (opcode == action family byte), row 1 =
// code-call family, row 7 = global/memory 0x70-0x76 + locals 0x79-0x7F, row 8 = locals
// 0x80-0x83 + heap halves, row 9 = log + storage halves, row D = reserved for future
// families. New opcodes land in row D, not in row 0. 0x74-0x78, 0x84-0x88 and 0x97-0x98
// are free because the live network never validated them as opcodes.

#[repr(u8)]
#[allow(non_camel_case_types)]
#[derive(Default, PartialEq, Debug, Clone, Copy)]
pub enum Bytecode {
    // row 0: protocol-family window. Within it the opcode equals the action KIND's
    // high (family) byte: 0x00 action, 0x06 view, 0x07 env.
    #[default]
    ACTION = 0x00, // *@  call action
    ____________01 = 0x01,
    ____________02 = 0x02,
    ____________03 = 0x03,
    ____________04 = 0x04, // GUARD action family (protocol kind 0x04)
    ____________05 = 0x05,
    ACTVIEW = 0x06, // *@  call action view (read-only query)
    ACTENV = 0x07,  // *+  call action env
    NTENV = 0x08,   // *+  native env (VM state read)
    NTCTL = 0x09,   // *@  native runtime control (modify VM tx-local state)
    NTFUNC = 0x0a,  // *@  native pure function
    ____________0b = 0x0b,
    ____________0c = 0x0c,
    ____________0d = 0x0d,
    ____________0e = 0x0e,
    ____________0f = 0x0f,

    // row 1: code-call family (is_call = 0x12..=0x1c)
    ____________10 = 0x10,
    ____________11 = 0x11, // reserved: CALC_CALL
    CODE_CALL = 0x12,    // *,****
    CALL = 0x13,         // **,****@
    CALLEXT = 0x14,      // *,****@
    CALLEXTVIEW = 0x15,  // *,****@
    CALLUSEVIEW = 0x16,  // *,****@
    CALLUSEPURE = 0x17,  // *,****@
    CALLTHIS = 0x18,     // ****@
    CALLSELF = 0x19,     // ****@
    CALLSUPER = 0x1a,    // ****@
    CALLSELFVIEW = 0x1b, // ****@
    CALLSELFPURE = 0x1c, // ****@
    ____________1d = 0x1d,
    ____________1e = 0x1e,
    ____________1f = 0x1f,

    // row 2: push immediates
    PU8 = 0x20,    // *+     push u8
    PU16 = 0x21,   // **+    push u16
    PBUF = 0x22,   // *+     push buf
    PBUFL = 0x23,  // **+    push buf long
    P0 = 0x24,     // +      push u8 0
    P1 = 0x25,     // +      push u8 1
    P2 = 0x26,     // +      push u8 2
    P3 = 0x27,     // +      push u8 3
    PNIL = 0x28,   // +      push nil
    PNBUF = 0x29,  // +      push buf empty
    PTRUE = 0x2a,  // +      push true
    PFALSE = 0x2b, // +      push false
    ____________2c = 0x2c,
    ____________2d = 0x2d,
    ____________2e = 0x2e,
    ____________2f = 0x2f,

    // row 3: cast 0x30-0x37 / type 0x38-0x3C
    CU8 = 0x30,   // &      cast u8
    CU16 = 0x31,  // &      cast u16
    CU32 = 0x32,  // &      cast u32
    CU64 = 0x33,  // &      cast u64
    CU128 = 0x34, // &      cast u128
    ____________35 = 0x35, // reserved (CastU256)
    CBYTES = 0x36, // &      cast bytes
    CTO = 0x37,    // *&     cast to
    TNIL = 0x38,   // &      is nil push Bool(true)
    TLIST = 0x39,  // &      is compo list push Bool(true)
    TMAP = 0x3a,   // &      is compo map  push Bool(true)
    TIS = 0x3b,    // *&     is type id
    TID = 0x3c,    // &      type id
    ____________3d = 0x3d,
    ____________3e = 0x3e,
    ____________3f = 0x3f,

    // row 4: stack 0x40-0x47 / bytes+buffer 0x48-0x4F
    DUP = 0x40,    // +      copy 0
    DUPN = 0x41,   // *+     copy u8
    POP = 0x42,    // a      pop top
    POPN = 0x43,   // *a...b pop n
    ROLL0 = 0x44,  // +      roll0
    ROLL = 0x45,   // *+     roll
    SWAP = 0x46,   // a,b++  swap  b,a = a,b
    REV = 0x47,    // a...b  reverse u8
    CAT = 0x48,    // a,b+   buf: a + b
    JOIN = 0x49,   // a...bn+
    BYTE = 0x4a,   // buf,idx+ pop idx; peek buf -> u8 at idx
    CUT = 0x4b,    // buf,ost,len+ pop len,ost; peek buf -> buf[ost..ost+len]
    LEFT = 0x4c,   // *&     cut left  buf *
    RIGHT = 0x4d,  // *&     cut right buf *
    LDROP = 0x4e,  // *&     drop buf left *
    RDROP = 0x4f,  // *&     drop buf right *

    // row 5: overflow 0x50-0x51 / compo create+mutate 0x58-0x5F
    SIZE = 0x50,   // &      size (u16)
    CHOOSE = 0x51, // cond,yes,no+ cond?yes:no (stack bottom->top; +movement gas 2)
    ____________52 = 0x52,
    ____________53 = 0x53,
    ____________54 = 0x54,
    ____________55 = 0x55,
    ____________56 = 0x56,
    ____________57 = 0x57,
    NEWLIST = 0x58,  // + new compo list
    NEWMAP = 0x59,   // + new compo map
    PACKLIST = 0x5a, // (v...,n)+ items then count on top; not (n,v...)
    PACKMAP = 0x5b,  // (k,v...,n)+ kv pairs then count on top; count is total items
    INSERT = 0x5c,   // t,k,v+  compo insert
    REMOVE = 0x5d,   // t,k+    compo remove
    CLEAR = 0x5e,    // t+      compo clear
    MERGE = 0x5f,    // a,b+    compo merge

    // row 6: compo read / transform
    LENGTH = 0x60,     // t+      compo length
    HASKEY = 0x61,     // t,k+    compo check has key
    ITEMGET = 0x62,    // t,k+    compo iten get
    KEYS = 0x63,       // &       compo keys
    VALUES = 0x64,     // &       compo values
    TAKEFIRST = 0x65,  // t+      compo take first; discard rest
    TAKELAST = 0x66,   // t+      compo take last; discard rest
    APPEND = 0x67,     // t,v+    append v to list compo t (t on bottom)
    CLONE = 0x68,      // a++     compo clone
    UNPACK = 0x69,     // c,i+    pop start idx; peek container; item/4 read + local writes
    PACKTUPLE = 0x6a,  // (v...,n)+ tuple items then count on top
    TUPLE2LIST = 0x6b, // &       tuple to list
    ____________6c = 0x6c,
    ____________6d = 0x6d,
    ____________6e = 0x6e,
    ____________6f = 0x6f,

    // row 7: 0x70-0x76 global/memory, 0x79-0x7F locals. The global/memory head sits here
    // because mainnet P2SH lock scripts (revealed from height 768358 on) encode GET0..GET3
    // as 0x80..0x83 and the live network validated them at those bytes; see
    // `onchain_p2sh_lockboxes_keep_their_deployed_encoding`.
    GPUT = 0x70,   // a,b   global put
    GGET = 0x71,   // &     global get
    MPUT = 0x72,   // a,b   memory put
    MGET = 0x73,   // &     memory get
    MONCE = 0x74,  // a,b   memory once
    MTAKE = 0x75,  // &     memory take
    MPATCH = 0x76, // a,b,c+  key, expected, patch_set → sha2(final)
    ____________77 = 0x77,
    ____________78 = 0x78,
    XLG = 0x79,   // *&    local[idx] op stack_top (rhs only on stack; not GT)
    XOP = 0x7a,   // *a    local[idx] op= stack_top (rhs); +stack_write on result val_size
    GET = 0x7b,   // *+    local get (idx in immediate)
    PUT = 0x7c,   // *a    local put (idx in immediate; value on stack)
    GETX = 0x7d,  // idx+  peek idx -> load local[idx] in place
    PUTX = 0x7e,  // idx,val+ dynamic local put (IR: local_x_put(idx, val))
    ALLOC = 0x7f, // *     local allocQ

    // row 8: locals 0x80-0x83 / free 0x84-0x87 / heap 0x88-0x8F.
    // GET0..GET3 keep the byte values the live network used; the memory family moved to
    // row 7 (0x70-0x76) so the swap costs no deployed bytecode meaning.
    GET0 = 0x80, // +     local get idx 0
    GET1 = 0x81, // +     local get idx 1
    GET2 = 0x82, // +     local get idx 2
    GET3 = 0x83, // +     local get idx 3
    ____________84 = 0x84,
    ____________85 = 0x85,
    ____________86 = 0x86,
    ____________87 = 0x87,
    ____________88 = 0x88, // reserved (removed HSLICE)
    HREADUL = 0x89,  // **+   heap read ul
    HREADU = 0x8a,   // *+    heap read u
    HWRITEXL = 0x8b, // **a   heap write xl (u16 immediate offset)
    HWRITEX = 0x8c,  // *a    heap write x (u8 immediate offset)
    HREAD = 0x8d,    // a,b+  heap read
    HWRITE = 0x8e,   // a,b   heap write (dynamic u32 offset)
    HGROW = 0x8f,    // *     heap grow

    // row 9: log 0x90-0x93 / status 0x96-0x97 / storage 0x98-0x9F
    LOG1 = 0x90,
    LOG2 = 0x91,
    LOG3 = 0x92,
    LOG4 = 0x93,
    ____________94 = 0x94,
    ____________95 = 0x95,
    SPUT = 0x96,   // a,b   status put
    SGET = 0x97,   // &     status get
    SSTAT = 0x98,  // &      storage info
    SLOAD = 0x99,  // &      storage load
    SPATCH = 0x9a, // a,b,c+  key, expected, patch_set → sha2(final)
    SEDIT = 0x9b,  // a,b    storage edit
    SDEL = 0x9c,   // a      storage delete
    SNEW = 0x9d,   // a,b,c  storage create
    SRECV = 0x9e,  // a,b    storage recover rent
    SRENT = 0x9f,  // a,b    storage time rent

    // row A: logic+compare 0xA0-0xA8 / bit 0xAB-0xAF
    AND = 0xa0, // a,b+   and
    OR = 0xa1,  // a,b+   or
    EQ = 0xa2,  // a,b+   equal
    NEQ = 0xa3, // a,b+   not equal
    LT = 0xa4,  // a,b+   less than
    GT = 0xa5,  // a,b+   great than
    LE = 0xa6,  // a,b+   less and eq
    GE = 0xa7,  // a,b+   great and eq
    NOT = 0xa8, // a+   not
    ____________a9 = 0xa9,
    ____________aa = 0xaa,
    BSHR = 0xab, // a,b+   shr: >>
    BSHL = 0xac, // a,b+   shl: <<
    BXOR = 0xad, // a,b+   xor: ^
    BOR = 0xae,  // a,b+   or:  |
    BAND = 0xaf, // a,b+   and: &

    // row B: scalar arithmetic (0xB0-0xBD; 0xBE/0xBF free)
    ADD = 0xb0,     // a,b+   +
    SUB = 0xb1,     // a,b+   -
    MUL = 0xb2,     // a,b+   *
    DIV = 0xb3,     // a,b+   floor(a/b)
    MOD = 0xb4,     // a,b+   mod
    POW = 0xb5,     // a,b+   pow
    SQRT = 0xb6,    // a+     floor isqrt(a)
    SQRTUP = 0xb7,  // a+     ceil sqrt (min y with y*y >= a)
    MAX = 0xb8,     // a,b+   max
    MIN = 0xb9,     // a,b+   min
    CLAMP = 0xba,   // a,b,c+ clamp(x, lo, hi)
    ABSDIFF = 0xbb, // a,b+   abs(x-y)
    INC = 0xbc,     // *&     += u8
    DEC = 0xbd,     // *&     -= u8
    ____________be = 0xbe,
    ____________bf = 0xbf,

    // row C: multi-operand arithmetic 0xC0-0xC7 / finance 0xCA-0xCF (FIN ids)
    ADDMOD = 0xc0,   // a,b,c+ (x+y)%z
    MULMOD = 0xc1,   // a,b,c+ (x*y)%z
    MULADD = 0xc2,   // a,b,c+ (x*y)+z
    MULSUB = 0xc3,   // a,b,c+ (x*y)-z
    DIVUP = 0xc4,    // a,b+   ceil(a/b)
    DIVEXACT = 0xc5, // a,b+   exact(a/b)
    MULDIV = 0xc6,   // a,b,c+ floor((x*y)/z)
    MULDIVUP = 0xc7, // a,b,c+ ceil((x*y)/z)
    ____________c8 = 0xc8,
    ____________c9 = 0xc9,
    FINPOW3 = 0xca, // *,a,b,c+   fin pow id
    FINP4 = 0xcb,   // *,a,b,c,d+ fin 4-input predicate
    FINP3 = 0xcc,   // *,a,b,c+   fin 3-input predicate
    FIN4 = 0xcd,    // *,a,b,c,d+ fin 4-input calc id
    FIN3 = 0xce,    // *,a,b,c+   fin 3-input calc id
    FIN2 = 0xcf,    // *,a,b+     fin 2-input calc id

    // row D: reserved for future opcode families
    ____________d0 = 0xd0,
    ____________d1 = 0xd1,
    ____________d2 = 0xd2,
    ____________d3 = 0xd3,
    ____________d4 = 0xd4,
    ____________d5 = 0xd5,
    ____________d6 = 0xd6,
    ____________d7 = 0xd7,
    ____________d8 = 0xd8,
    ____________d9 = 0xd9,
    ____________da = 0xda,
    ____________db = 0xdb,
    ____________dc = 0xdc,
    ____________dd = 0xdd,
    ____________de = 0xde,
    ____________df = 0xdf,

    // row E: branch 0xE0-0xE6 / control+return 0xEA-0xEF
    JMPL = 0xe0,  // **    jump long
    JMPS = 0xe1,  // *     jump offset
    JMPSL = 0xe2, // **    jump offset long
    BRL = 0xe3,   // **a   branch long
    BRS = 0xe4,   // *a    branch offset
    BRSL = 0xe5,  // **a   branch offset long not_zero
    BRSLN = 0xe6, // **a   branch offset long is_zero
    ____________e7 = 0xe7,
    ____________e8 = 0xe8,
    ____________e9 = 0xe9,
    PRT = 0xea,        // s     print for debug
    AST = 0xeb,        // c     assert throw
    ERR = 0xec,        // a     throw (ERR)
    ABT = 0xed,        // abord
    RET = 0xee,        // a     func return (DATA)
    END = 0xef,        // func return nil

    // row F: IR nodes (never in runtime bytecode) / gas+misc 0xFD-0xFF
    IRBYTECODE = 0xf0, // <IR NODE>
    IRLIST = 0xf1,     // <IR NODE>
    IRBLOCK = 0xf2,    // <IR NODE>
    IRBLOCKR = 0xf3,   // <IR NODE>
    IRIF = 0xf4,       // <IR NODE>
    IRIFR = 0xf5,      // <IR NODE>
    IRWHILE = 0xf6,    // <IR NODE>
    IRBREAK = 0xf7,    // <IR NODE>
    IRCONTINUE = 0xf8, // <IR NODE>
    ____________f9 = 0xf9,
    ____________fa = 0xfa,
    ____________fb = 0xfb,
    ____________fc = 0xfc,
    BURN = 0xfd, // **    burn gas
    NOP = 0xfe,  // do nothing
    NT = 0xff,   // panic: never touch
}

use Bytecode::*;

impl From<Bytecode> for u8 {
    fn from(val: Bytecode) -> u8 {
        val as u8
    }
}

#[derive(Default, Debug, Copy, Clone)]
pub struct BytecodeMetadata {
    pub valid: bool,
    pub param: u8,
    pub input: u8,
    pub output: u8,
    pub intro: &'static str,
}

macro_rules! bytecode_metadata_define {
    ( $( $inst:ident : $p:expr, $i:expr, $o:expr , $s:ident)+ ) => {

impl Bytecode {

    pub fn metadata(&self) -> BytecodeMetadata {
        match self {
            $(
            $inst => BytecodeMetadata {valid: true, param: $p, input: $i, output: $o, intro: stringify!($s)},
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

    pub fn try_from_u8(v: u8) -> VmrtRes<Self> {
        match v {
            $(
            x if x == $inst as u8 => Ok($inst),
            )+
            _ => Err(ItrErr::new(ItrErrCode::InstInvalid, &format!("bytecode 0x{:02x} is invalid", v))),
        }
    }

}

macro_rules! bytecode_intro_sig {
    $(
        ($s) => {
            ($crate::rt::Bytecode::$inst, ($p) as usize, ($i) as usize, ($o) as usize)
        };
    )+
}

    };
}

/* params, stack input, stack output */
bytecode_metadata_define! {
    ACTION     : 1, 1, 0,     action  // no stack output; output=0 to avoid extra POP in IRBLOCK
    ACTVIEW    : 1, 1, 1,     actview
    ACTENV     : 1, 0, 1,     actenv
    NTENV      : 1, 0, 1,     native_env
    NTCTL      : 1, 1, 1,     native_ctl
    NTFUNC     : 1, 1, 1,     native_func

    CODE_CALL    : 1+4, 1, 0,   code_call
    CALL         :   6, 1, 1,   call
    CALLEXT      : 1+4, 1, 1,   callext
    CALLEXTVIEW  : 1+4, 1, 1,   callextview
    CALLUSEVIEW  : 1+4, 1, 1,   calluseview
    CALLUSEPURE  : 1+4, 1, 1,   callusepure
    CALLTHIS     :   4, 1, 1,   callthis
    CALLSELF     :   4, 1, 1,   callself
    CALLSUPER    :   4, 1, 1,   callsuper
    CALLSELFVIEW :   4, 1, 1,   callselfview
    CALLSELFPURE :   4, 1, 1,   callselfpure

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
    PTRUE      : 0, 0, 1,     push_true
    PFALSE     : 0, 0, 1,     push_false

    CU8        : 0, 1, 1,     cast_u8
    CU16       : 0, 1, 1,     cast_u16
    CU32       : 0, 1, 1,     cast_u32
    CU64       : 0, 1, 1,     cast_u64
    CU128      : 0, 1, 1,     cast_u128
    CBYTES     : 0, 1, 1,     cast_bytes
    CTO        : 1, 1, 1,     cast_to
    TNIL       : 0, 1, 1,     type_is_nil
    TLIST      : 0, 1, 1,     type_is_list
    TMAP       : 0, 1, 1,     type_is_map
    TIS        : 1, 1, 1,     type_is
    TID        : 0, 1, 1,     type_id

    DUP        : 0, 0, 1,     dump
    DUPN       : 1, 0, 255,   dump_n
    POP        : 0, 255, 0,   pop
    POPN       : 1, 255, 0,   pop_n
    ROLL0      : 0, 0, 1,     roll_0
    ROLL       : 1, 0, 1,     roll
    SWAP       : 0, 2, 2,     swap
    REV        : 1, 255, 255, reverse
    CAT        : 0, 2, 1,     concat
    JOIN       : 1, 255, 1,   join
    BYTE       : 0, 2, 1,     byte
    CUT        : 0, 3, 1,     buf_cut
    LEFT       : 1, 1, 1,     buf_left
    RIGHT      : 1, 1, 1,     buf_right
    LDROP      : 1, 1, 1,     buf_left_drop
    RDROP      : 1, 1, 1,     buf_right_drop
    SIZE       : 0, 1, 1,     size
    CHOOSE     : 0, 3, 1,     choose

    NEWLIST    : 0, 0, 1,     new_list
    NEWMAP     : 0, 0, 1,     new_map
    PACKLIST   : 0, 255, 1,   pack_list
    PACKMAP    : 0, 255, 1,   pack_map
    INSERT     : 0, 3, 1,     insert
    REMOVE     : 0, 2, 1,     remove
    CLEAR      : 0, 1, 1,     clear
    MERGE      : 0, 2, 1,     merge
    LENGTH     : 0, 1, 1,     length
    HASKEY     : 0, 2, 1,     has_key
    ITEMGET    : 0, 2, 1,     item_get
    KEYS       : 0, 1, 1,     keys
    VALUES     : 0, 1, 1,     values
    TAKEFIRST  : 0, 1, 1,     take_first
    TAKELAST   : 0, 1, 1,     take_last
    APPEND     : 0, 2, 1,     append
    CLONE      : 0, 1, 1,     clone
    PACKTUPLE  : 0, 255, 1,   pack_tuple
    TUPLE2LIST : 0, 1, 1,     tuple_to_list
    UNPACK     : 0, 2, 0,     unpack

    XLG        : 1, 1, 1,     local_logic
    XOP        : 1, 1, 0,     local_operand
    GET        : 1, 0, 1,     local
    PUT        : 1, 1, 0,     local_put
    GETX       : 0, 1, 1,     local_x
    PUTX       : 0, 2, 0,     local_x_put
    ALLOC      : 1, 0, 0,     local_alloc
    GET0       : 0, 0, 1,     local_0
    GET1       : 0, 0, 1,     local_1
    GET2       : 0, 0, 1,     local_2
    GET3       : 0, 0, 1,     local_3

    LOG1       : 0, 2, 0,     log_1
    LOG2       : 0, 3, 0,     log_2
    LOG3       : 0, 4, 0,     log_3
    LOG4       : 0, 5, 0,     log_4

    HREADUL    : 2, 0, 1,     heap_read_uint_long
    HREADU     : 1, 0, 1,     heap_read_uint
    HWRITEXL   : 2, 1, 0,     heap_write_xl
    HWRITEX    : 1, 1, 0,     heap_write_x
    HREAD      : 0, 2, 1,     heap_read
    HWRITE     : 0, 2, 0,     heap_write
    HGROW      : 1, 0, 0,     heap_grow

    GPUT       : 0, 2, 0,     global_put
    GGET       : 0, 1, 1,     global_get
    MPUT       : 0, 2, 0,     memory_put
    MGET       : 0, 1, 1,     memory_get
    MONCE      : 0, 2, 0,     memory_once
    MTAKE      : 0, 1, 1,     memory_take
    MPATCH     : 0, 3, 1,     memory_patch
    SPUT       : 0, 2, 0,     status_put
    SGET       : 0, 1, 1,     status_get

    SSTAT      : 0, 1, 1,     storage_stat
    SLOAD      : 0, 1, 1,     storage_load
    SPATCH     : 0, 3, 1,     storage_patch
    SEDIT      : 0, 2, 0,     storage_edit
    SDEL       : 0, 1, 0,     storage_del
    SNEW       : 0, 3, 0,     storage_new
    SRECV      : 0, 2, 0,     storage_recv
    SRENT      : 0, 2, 0,     storage_rent

    AND        : 0, 2, 1,     and
    OR         : 0, 2, 1,     or
    EQ         : 0, 2, 1,     equal
    NEQ        : 0, 2, 1,     not_equal
    LT         : 0, 2, 1,     less_than
    GT         : 0, 2, 1,     greater_than
    LE         : 0, 2, 1,     less_equal
    GE         : 0, 2, 1,     greater_equal
    NOT        : 0, 1, 1,     not

    BSHR       : 0, 2, 1,     bit_shr
    BSHL       : 0, 2, 1,     bit_shl
    BXOR       : 0, 2, 1,     bit_xor
    BOR        : 0, 2, 1,     bit_or
    BAND       : 0, 2, 1,     bit_and

    ADD         : 0, 2, 1,     add
    SUB         : 0, 2, 1,     sub
    MUL         : 0, 2, 1,     mul
    DIV         : 0, 2, 1,     div
    DIVUP       : 0, 2, 1,     div_up
    DIVEXACT    : 0, 2, 1,     div_exact_op
    MOD         : 0, 2, 1,     mod
    POW         : 0, 2, 1,     pow
    SQRT        : 0, 1, 1,     sqrt
    SQRTUP      : 0, 1, 1,     sqrt_up
    MAX         : 0, 2, 1,     max
    MIN         : 0, 2, 1,     min
    CLAMP       : 0, 3, 1,     clamp
    ABSDIFF     : 0, 2, 1,     abs_diff
    INC         : 1, 1, 1,     increase
    DEC         : 1, 1, 1,     decrease

    ADDMOD      : 0, 3, 1,     add_mod
    MULMOD      : 0, 3, 1,     mul_mod
    MULADD      : 0, 3, 1,     mul_add
    MULSUB      : 0, 3, 1,     mul_sub
    MULDIV      : 0, 3, 1,     mul_div
    MULDIVUP    : 0, 3, 1,     mul_div_up

    FIN2        : 1, 2, 1,     fin_2
    FIN3        : 1, 3, 1,     fin_3
    FIN4        : 1, 4, 1,     fin_4
    FINP3       : 1, 3, 1,     fin_p3
    FINP4       : 1, 4, 1,     fin_p4
    FINPOW3     : 1, 3, 1,     fin_pow3

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
    PRT        : 0, 1, 0,     print

    IRBYTECODE : 2, 255, 0,   ir_bytecode
    IRLIST     : 2, 255, 1,   ir_list
    IRBLOCK    : 2, 255, 0,   ir_block
    IRBLOCKR   : 2, 255, 1,   ir_block_expr
    IRIF       : 0, 3, 0,     ir_if
    IRIFR      : 0, 3, 1,     ir_if_expr
    IRWHILE    : 0, 2, 0,     ir_while
    IRBREAK    : 0, 0, 0,     ir_break      // patch-list lowered; never appears in runtime bytecode
    IRCONTINUE : 0, 0, 0,     ir_continue   // patch-list lowered; never appears in runtime bytecode

    BURN       : 2, 0, 0,     gas_burn
    NOP        : 0, 0, 0,     nop
    NT         : 0, 0, 0,     never_touch

}

#[cfg(test)]
mod bytecode_tests {
    use super::*;

    /// Addresses are the contract the rest of the VM is written against: families are
    /// rows (or half-rows) so membership is one range test, and the call family is
    /// contiguous. Freeze them here so a future edit cannot silently move an opcode.
    #[test]
    fn layout_matches_v3_map() {
        // row 0: protocol-family window, opcode == action family byte
        assert_eq!(ACTION as u8, 0x00);
        assert_eq!(ACTVIEW as u8, 0x06);
        assert_eq!(ACTENV as u8, 0x07);
        assert_eq!(NTENV as u8, 0x08);
        assert_eq!(NTCTL as u8, 0x09);
        assert_eq!(NTFUNC as u8, 0x0a);
        // row 1: 0x10 free, 0x11 reserved (CALC_CALL), code-call family contiguous 0x12..=0x1C
        assert_eq!(CODE_CALL as u8, 0x12);
        assert_eq!(CALL as u8, 0x13);
        assert_eq!(CALLEXT as u8, 0x14);
        assert_eq!(CALLEXTVIEW as u8, 0x15);
        assert_eq!(CALLUSEVIEW as u8, 0x16);
        assert_eq!(CALLUSEPURE as u8, 0x17);
        assert_eq!(CALLTHIS as u8, 0x18);
        assert_eq!(CALLSELF as u8, 0x19);
        assert_eq!(CALLSUPER as u8, 0x1a);
        assert_eq!(CALLSELFVIEW as u8, 0x1b);
        assert_eq!(CALLSELFPURE as u8, 0x1c);
        // row 5: overflow, then compo create/mutate
        assert_eq!(SIZE as u8, 0x50);
        assert_eq!(CHOOSE as u8, 0x51);
        assert_eq!(NEWLIST as u8, 0x58);
        assert_eq!(MERGE as u8, 0x5f);
        // row 6: compo read / transform
        assert_eq!(LENGTH as u8, 0x60);
        assert_eq!(TUPLE2LIST as u8, 0x6b);
        // row 7: global/memory family 0x70-0x76, then locals
        assert_eq!(GPUT as u8, 0x70);
        assert_eq!(MGET as u8, 0x73);
        assert_eq!(MONCE as u8, 0x74);
        assert_eq!(MPATCH as u8, 0x76);
        assert_eq!(XLG as u8, 0x79);
        assert_eq!(ALLOC as u8, 0x7f);
        // row 8: locals 0x80-0x83 (mainnet P2SH encoding), 0x84-0x87 free, then heap
        assert_eq!(GET0 as u8, 0x80);
        assert_eq!(GET1 as u8, 0x81);
        assert_eq!(GET2 as u8, 0x82);
        assert_eq!(GET3 as u8, 0x83);
        assert_eq!(HREADUL as u8, 0x89);
        assert_eq!(HWRITEX as u8, 0x8c);
        assert_eq!(HGROW as u8, 0x8f);
        // row 9: log, then status, then storage
        assert_eq!(LOG1 as u8, 0x90);
        assert_eq!(LOG4 as u8, 0x93);
        assert_eq!(SPUT as u8, 0x96);
        assert_eq!(SGET as u8, 0x97);
        assert_eq!(SSTAT as u8, 0x98);
        assert_eq!(SLOAD as u8, 0x99);
        assert_eq!(SPATCH as u8, 0x9a);
        assert_eq!(SRENT as u8, 0x9f);
        // rows B/C: scalar arithmetic 0xB0-0xBD, multi-operand 0xC0-0xC7, finance 0xCA-0xCF
        assert_eq!(ADD as u8, 0xb0);
        assert_eq!(SUB as u8, 0xb1);
        assert_eq!(MUL as u8, 0xb2);
        assert_eq!(DIV as u8, 0xb3);
        assert_eq!(MOD as u8, 0xb4);
        assert_eq!(POW as u8, 0xb5);
        assert_eq!(SQRT as u8, 0xb6);
        assert_eq!(SQRTUP as u8, 0xb7);
        assert_eq!(MAX as u8, 0xb8);
        assert_eq!(MIN as u8, 0xb9);
        assert_eq!(CLAMP as u8, 0xba);
        assert_eq!(ABSDIFF as u8, 0xbb);
        assert_eq!(INC as u8, 0xbc);
        assert_eq!(DEC as u8, 0xbd);
        assert_eq!(ADDMOD as u8, 0xc0);
        assert_eq!(MULMOD as u8, 0xc1);
        assert_eq!(MULADD as u8, 0xc2);
        assert_eq!(MULSUB as u8, 0xc3);
        assert_eq!(DIVUP as u8, 0xc4);
        assert_eq!(DIVEXACT as u8, 0xc5);
        assert_eq!(MULDIV as u8, 0xc6);
        assert_eq!(MULDIVUP as u8, 0xc7);
        assert_eq!(FINPOW3 as u8, 0xca);
        assert_eq!(FIN2 as u8, 0xcf);
        // row E/F: branch + control/return + IR nodes unchanged
        assert_eq!(JMPL as u8, 0xe0);
        assert_eq!(BRSLN as u8, 0xe6);
        assert_eq!(PRT as u8, 0xea);
        assert_eq!(END as u8, 0xef);
        assert_eq!(IRBYTECODE as u8, 0xf0);
        assert_eq!(IRCONTINUE as u8, 0xf8);
        assert_eq!(BURN as u8, 0xfd);
        assert_eq!(NT as u8, 0xff);
    }

    /// The call family is one range test: `is_user_call_inst` is true exactly on its run.
    #[test]
    fn call_family_is_one_contiguous_range() {
        for op in 0x12u8..=0x1c {
            let inst = Bytecode::try_from_u8(op).expect("call opcode");
            assert!(crate::rt::is_user_call_inst(inst), "0x{op:02x} must be a call");
        }
        for op in [0x0fu8, 0x10, 0x11, 0x1d, 0x20, 0xe0] {
            match Bytecode::try_from_u8(op) {
                Ok(inst) => assert!(
                    !crate::rt::is_user_call_inst(inst),
                    "0x{op:02x} must not be a call"
                ),
                Err(_) => {}
            }
        }
    }

    /// Reserved and free bytes stay unassigned, and the whole named set round-trips.
    #[test]
    fn reserved_slots_are_invalid_and_named_slots_round_trip() {
        let mut named = 0usize;
        for op in 0u8..=255 {
            match Bytecode::try_from_u8(op) {
                Ok(inst) => {
                    named += 1;
                    assert_eq!(inst as u8, op, "0x{op:02x} round-trip");
                    assert!(inst.metadata().valid, "0x{op:02x} metadata");
                }
                Err(e) => assert_eq!(e.0, ItrErrCode::InstInvalid),
            }
        }
        assert_eq!(named, 185, "184 existing opcodes + MPATCH");
        // row 0 protocol window holes, the reserved 0x10/0x11 pair, the 0x77 hole and
        // the 0x84-0x88 row-8 run (all never validated as opcodes by the live network),
        // the row B/C holes, and the row D growth row
        for op in [
            0x01u8, 0x04, 0x05, 0x10, 0x11, 0x77, 0x84, 0x85, 0x86, 0x87, 0x88, 0xbe, 0xc8, 0xd0,
            0xdf,
        ] {
            assert!(
                Bytecode::try_from_u8(op).is_err(),
                "0x{op:02x} must stay free/reserved"
            );
        }
        assert_eq!(Bytecode::parse("MPATCH"), Some(MPATCH));
        assert_eq!(Bytecode::parse("CODE_CALL"), Some(CODE_CALL));
        assert_eq!(Bytecode::parse("CODECALL"), None);
    }

    /// The nine P2SH lock scripts mainnet actually contains (action kind 46, first
    /// revealed at height 768358, the whole population through height 784301). They were
    /// validated by the live network under its own bytecode map, so the current map must
    /// keep both the meaning and the immediate width of every byte they use: a wrong
    /// meaning silently runs different code, a wrong width desynchronises the stream.
    /// Regenerate with `p2sh_scan <block_dir> 765432 <tip>` if this ever needs re-deriving.
    #[test]
    fn onchain_p2sh_lockboxes_keep_their_deployed_encoding() {
        const LOCKBOXES: &[(u64, &str)] = &[
            (768358, "7f0107027c00070122030bba7c32a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
            (768450, "7f0107027c00070122030bdb7732a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c80221500165c77a76c6c267624a2cee98252a1c0645540fe3709a2eb24ee"),
            (768490, "7f0107027c00070122030bdb7732a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c80221500165c77a76c6c267624a2cee98252a1c0645540fe3709a2eb24ee"),
            (768907, "7f0107027c00070122030bba7c32a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
            (769079, "7f0107027c00070122030bbcb132a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
            (769079, "7f0107027c00070122030bbcaf32a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
            (769202, "7f0107027c00070122030bbdc932a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
            (769214, "7f0107027c00070122030bbcb032a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
            (769214, "7f0107027c00070122030bbcae32a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee"),
        ];
        // Instruction sequence the live network executed for all nine of them. The
        // addresses in the two PBUF operands differ per script; the shape does not.
        const EXPECTED: &[&str] = &[
            "ALLOC", "ACTENV", "PUT", "ACTENV", "PBUF", "CU32", "GE", "BRSL", "GET0", "PBUF",
            "CTO", "EQ", "AST", "JMPSL", "GET0", "PBUF", "CTO", "EQ", "AST", "P0", "RET",
        ];

        for (height, hex_lockbox) in LOCKBOXES {
            let codes = hex::decode(hex_lockbox).expect("lockbox hex");
            let mut got = Vec::new();
            let mut pc = 0usize;
            while pc < codes.len() {
                let inst = match Bytecode::try_from_u8(codes[pc]) {
                    Ok(inst) => inst,
                    Err(_) => panic!("height {height}: 0x{:02x} is not an opcode", codes[pc]),
                };
                got.push(format!("{:?}", inst));
                pc += 1;
                pc += match inst {
                    // PBUF carries its own length byte, so `metadata().param` is not the
                    // whole story: overrun here is exactly the desync this test guards.
                    PBUF => 1 + codes[pc] as usize,
                    PBUFL => 2 + u16::from_be_bytes([codes[pc], codes[pc + 1]]) as usize,
                    _ => inst.metadata().param as usize,
                };
            }
            assert_eq!(pc, codes.len(), "height {height}: stream does not end on a boundary");
            assert_eq!(codes.len(), 79, "height {height}: unexpected script length");
            assert_eq!(
                got, EXPECTED,
                "height {height}: on-chain P2SH script no longer decodes to the same instructions"
            );
            // Applies the map's stack effects too (GET0 pushes, GPUT pops 2), so a future
            // renumbering that swaps a push for a consumer fails here before it can fork.
            crate::rt::verify_bytecodes_with_entry_stack(
                &codes,
                crate::rt::VerifyEntryStack::OptionalArgv,
            )
            .unwrap_or_else(|e| panic!("height {height}: P2SH lockbox fails verification: {e}"));
        }
    }

    #[test]
    fn mpatch_metadata_gas_and_irfn() {
        let meta = MPATCH.metadata();
        assert!(meta.valid);
        assert_eq!((meta.param, meta.input, meta.output), (0, 3, 1));
        assert_eq!(meta.intro, "memory_patch");
        // same stack shape as SPATCH: key, expected, patch_set -> digest
        let spatch = SPATCH.metadata();
        assert_eq!(
            (meta.param, meta.input, meta.output),
            (spatch.param, spatch.input, spatch.output)
        );

        let gst = GasTable::new(0);
        assert_eq!(gst.gas(MPATCH as u8), 12);
        assert!(
            gst.gas(MPATCH as u8) >= gst.gas(MPUT as u8),
            "MPATCH is a memory write and must not be cheaper than MPUT"
        );

        let Some((_, bc, pms, args, rs)) = crate::rt::pick_ir_func("memory_patch") else {
            panic!("memory_patch irfn missing");
        };
        assert_eq!(bc, MPATCH);
        assert_eq!((pms, args, rs), (0, 3, 1));
    }
}
