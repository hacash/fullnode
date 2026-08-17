use field::*;
use std::sync::Arc;
use sys::{Error, Ret};

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

pub(crate) fn bufeat(buf: &[u8], n: usize) -> Ret<Vec<u8>> {
    if buf.len() < n {
        return sys::errf!("buffer too short");
    }
    Ok(buf[..n].to_vec())
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
mod lang_min;
#[cfg(feature = "full")]
pub use lang_min::OpTy;
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
    CallSpec, CallTarget, decode_user_call_site, encode_call_body, encode_splice_body,
    encode_user_call_site, is_user_call_inst,
};
mod verify;
#[cfg(feature = "full")]
pub use verify::{
    ensure_terminal_instruction, verify_bytecodes, verify_bytecodes_for_cap,
};
pub use verify::verify_bytecodes_with_registry;
// Re-export entry-stack helpers for external callers / future fitshc.
#[allow(unused_imports)]
pub use verify::{
    VerifyEntryStack, verify_bytecodes_with_entry_stack,
    verify_bytecodes_with_entry_stack_and_registry,
};

pub fn ascii_show_string(s: &[u8]) -> Option<String> {
    maybe!(
        s.iter().any(|&a| a != 10 && (a < 32 || a > 126)),
        None,
        Some(String::from_utf8(s.to_vec()).unwrap())
    )
}
