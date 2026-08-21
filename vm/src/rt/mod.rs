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

include!("xop.rs");
pub const IR_NAME_DIV: &str = "div";
pub const IR_NAME_MUL_DIV: &str = "mul_div";
include!("bytecode.rs");
include!("error.rs");
use ItrErrCode::*;
include!("code.rs");
include!("code_stuff.rs");
include!("fin.rs");
#[cfg(feature = "execute")]
mod lang_min;
#[cfg(feature = "execute")]
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
#[cfg(feature = "execute")]
mod verify;
#[cfg(feature = "execute")]
#[allow(unused_imports)] // entry-stack verify entry points reserved for machine entry checks
pub use verify::{
    VerifyEntryStack, ensure_terminal_instruction, verify_bytecodes, verify_bytecodes_for_cap,
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
