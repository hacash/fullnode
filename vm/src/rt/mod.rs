use field::*;
use std::sync::Arc;
use sys::{Error, Rerr, Ret};

macro_rules! enum_try_from_u8_by_variant {
    (
        $EnumName:ident,
        $ErrCode:expr,
        $ErrFmt:literal,
        [ $( $Variant:ident ),+ $(,)? ]
    ) => {
        impl $EnumName {
            #[inline]
            pub fn try_from_u8(v: u8) -> VmrtRes<Self> {
                match v {
                    $( x if x == Self::$Variant as u8 => Ok(Self::$Variant), )+
                    _ => itr_err_fmt!($ErrCode, $ErrFmt, v),
                }
            }
        }
    };
}

macro_rules! bit4l {
    ($v:expr) => {
        (($v) >> 4) & 0x0f
    };
}

macro_rules! bit4r {
    ($v:expr) => {
        ($v) & 0x0f
    };
}

/// Reinterpret a `u8` as the `#[repr(u8)] Bytecode` enum value (disassembly
/// path only; the value was validated via `Bytecode::metadata`).
macro_rules! std_mem_transmute {
    ($v:expr) => {
        unsafe { std::mem::transmute($v) }
    };
}

include!("xop.rs");
pub const IR_NAME_DIV: &str = "div";
pub const IR_NAME_MUL_DIV: &str = "mul_div";
include!("bytecode.rs");
include!("error.rs");
use ItrErrCode::*;
include!("code.rs");
include!("code_stuff.rs");
include!("fin.rs");
// fitsh language tokens: keyword/operator tables (`KwTy`/`OpTy`) and the
// shared `Token` stream type. Codec-safe: the decompiler consumes them in
// codec-only (SDK) builds, so they are not execute-gated.
include!("lang.rs");
// Native opcode catalog (codec-safe metadata tables; execution dispatch lives
// in `vm::native`). Codec-safe: the fitsh decompiler renders NTCTL/NTENV/NTFUNC
// by name in codec-only builds.
include!("native_catalog.rs");
include!("cap.rs");
include!("gas.rs");
mod func_argv;
pub use func_argv::FuncArgvTypes;
include!("function.rs");
include!("exec.rs");
mod abst_call;
pub use abst_call::AbstCall;
mod call_site;
#[allow(unused_imports)] // encode_call_body/encode_splice_body used only in tests
pub use call_site::{
    CallSpec, CallTarget, CALL_BODY_WIDTH, SPLICE_BODY_WIDTH, decode_call_body,
    decode_splice_body, decode_user_call_site, encode_call_body, encode_splice_body,
    encode_user_call_site, is_user_call_inst,
};
include!("action_defs.rs");
include!("parse.rs");
include!("sourcemap.rs");
#[cfg(feature = "execute")]
mod verify;
#[cfg(feature = "execute")]
#[allow(unused_imports)] // entry-stack verify entry points reserved for machine entry checks
pub use verify::{
    VerifyEntryStack, verify_bytecodes, verify_bytecodes_for_cap,
    verify_bytecodes_with_entry_stack, verify_bytecodes_with_entry_stack_and_registry,
    verify_bytecodes_with_registry,
};

pub fn ascii_show_string(s: &[u8]) -> Option<String> {
    maybe!(
        s.iter().any(|&a| a != 10 && (a < 32 || a > 126)),
        None,
        Some(String::from_utf8(s.to_vec()).unwrap())
    )
}

/// Ensure the last instruction is terminal (RET/END/ERR/ABT or exposed call
/// opcode). Codec-safe (the IR `convert_ir_to_runtime_bytecode` path and the
/// decompiler both rely on it); failure propagates as `CodeNotWithEnd`.
pub fn ensure_terminal_instruction(inst: Bytecode) -> VmrtErr {
    if matches!(inst, RET | END | ERR | ABT) || is_user_call_inst(inst) {
        return Ok(());
    }
    itr_err_code!(CodeNotWithEnd)
}
