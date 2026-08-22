/// Native opcode catalog (codec-safe): `NativeCtl` / `NativeEnv` / `NativeFunc`
/// enums with their metadata tables (idx, name, argv length, gas, return type).
/// The execute-only dispatch (`NativeFunc::call`) lives in the `native` module;
/// the fitsh decompiler/compiler (`lang`) consumes this catalog in codec-only
/// (SDK) builds. Keep the tables here in sync with `vm::native`.
use crate::value::ValueTy;

macro_rules! native_tar_uint_tys_api {
    (func, $EnumName:ident, $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $tar_uint_tys:expr )+) => {
        pub fn tar_uint_tys_of(&self) -> &'static [ValueTy] {
            match self {
                $( Self::$name => $tar_uint_tys, )+
                Self::Null => &[],
            }
        }

        pub fn tar_uint_tys(idx: u8) -> Option<&'static [ValueTy]> {
            Self::try_from_u8(idx).ok().and_then(|n| {
                let tys = n.tar_uint_tys_of();
                if tys.is_empty() {
                    None
                } else {
                    Some(tys)
                }
            })
        }
    };
    (ctl, $EnumName:ident, $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $tar_uint_tys:expr )+) => {};
    (env, $EnumName:ident, $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $tar_uint_tys:expr )+) => {};
}

/// Catalog-only generation: the enum, its idx constants and the metadata
/// accessors. The `call` dispatch is generated in `vm::native` (execute).
macro_rules! native_func_env_define {
    ( $kind:ident, $EnumName:ident, $ErrCode:ident,
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $tar_uint_tys:expr )+ ) => {
        #[allow(non_camel_case_types)]
        #[repr(u8)]
        #[derive(Default, PartialEq, Debug, Clone, Copy)]
        pub enum $EnumName {
            #[default] Null = 0u8,
            $( $name = $v, )+
        }

        impl $EnumName {
            $(
            #[allow(non_upper_case_globals)]
            pub const $name: u8 = $v;
            )+

            #[inline]
            pub fn try_from_u8(idx: u8) -> VmrtRes<Self> {
                match idx {
                    $( x if x == Self::$name as u8 => Ok(Self::$name), )+
                    _ => itr_err_fmt!($ErrCode, "not find {} idx {}", stringify!($EnumName), idx),
                }
            }

            native_tar_uint_tys_api!($kind, $EnumName, $( $name = $v, $argv_len, $gas, $rty, $tar_uint_tys )+);

            pub const fn gas_of(&self) -> i64 {
                match self {
                    $( Self::$name => $gas, )+
                    Self::Null => 0,
                }
            }

            pub fn gas(idx: u8) -> VmrtRes<i64> {
                Ok(Self::try_from_u8(idx)?.gas_of())
            }

            pub fn name(&self) -> &'static str {
                match self {
                    $( Self::$name => stringify!($name), )+
                    _ => unreachable!(),
                }
            }

            pub fn from_name(name: &str) -> Option<(u8, $EnumName)> {
                Some(match name {
                    $( stringify!($name) => (Self::$name as u8, Self::$name), )+
                    _ => return None,
                })
            }

            pub fn has_idx(idx: u8) -> bool {
                match idx {
                    $( $v => true, )+
                    _ => false,
                }
            }

            pub fn argv_len(idx: u8) -> Option<usize> {
                match idx {
                    $( $v => Some($argv_len), )+
                    _ => None,
                }
            }

            pub fn argv_len_of(&self) -> usize {
                match self {
                    $( Self::$name => $argv_len, )+
                    Self::Null => 0,
                }
            }
        }
    };
}

native_func_env_define! { env, NativeEnv, NativeEnvError,
    context_address    = 1,    0,        6,    ValueTy::Address,    &[]
}

native_func_env_define! { func, NativeFunc, NativeFuncError,
    hac_to_mei         = 31,   1,        6,    ValueTy::U64,        &[]
    hac_to_zhu         = 32,   1,        6,    ValueTy::U128,       &[]
    u64_to_fold64      = 33,   1,        8,    ValueTy::Bytes,      &[]
    fold64_to_u64      = 34,   1,        8,    ValueTy::U64,        &[]
    pack_asset         = 37,   2,        8,    ValueTy::Bytes,      &[ValueTy::U64, ValueTy::U64]
    mei_to_hac         = 35,   1,        6,    ValueTy::Bytes,      &[]
    zhu_to_hac         = 36,   1,        6,    ValueTy::Bytes,      &[]
    address_ptr        = 41,   1,        4,    ValueTy::U8,         &[]
    sha2               = 101, 1,       32,    ValueTy::Bytes,      &[]
    sha3               = 102, 1,       32,    ValueTy::Bytes,      &[]
    ripemd160          = 103, 1,       20,    ValueTy::Bytes,      &[]
    verify_signature   = 104, 3,       96,    ValueTy::Bool,       &[]
    keccak256          = 105, 1,       32,    ValueTy::Bytes,      &[]
    blake2s256         = 106, 1,       32,    ValueTy::Bytes,      &[]
    blake2b256         = 107, 1,       32,    ValueTy::Bytes,      &[]
    ascii_parse_flat_kv = 120, 2,      64,    ValueTy::Tuple,      &[]
    ascii_validate_transform = 121, 2, 24,    ValueTy::Tuple,      &[]
    ascii_u128_dec_unit = 122, 2,      24,    ValueTy::Tuple,      &[]
    ascii_hex_lower    = 123, 1,       20,    ValueTy::Tuple,      &[]
    ascii_base58_validate_or_echo = 124, 1, 20, ValueTy::Tuple,      &[]
}

native_func_env_define! { ctl, NativeCtl, NativeCtlError,
    defer              = 1,     1,        8,    ValueTy::Nil,        &[]
    intent_new         = 21,    1,       32,    ValueTy::Handle,     &[]
    intent_use         = 22,    1,        8,    ValueTy::Nil,        &[]
    intent_pop         = 23,    0,        8,    ValueTy::Nil,        &[]
    intent_is_own_handle      = 24,    1,       10,    ValueTy::Bool,       &[]
    intent_kind        = 25,    0,        8,    ValueTy::Bytes,      &[]
    intent_kind_is     = 26,    1,        8,    ValueTy::Bool,       &[]
    intent_destroy     = 27,    0,       10,    ValueTy::Nil,        &[]
    intent_destroy_if_empty = 28, 0,     10,    ValueTy::Bool,       &[]
    intent_clear       = 29,    0,       10,    ValueTy::Nil,        &[]
    intent_len         = 30,    0,       10,    ValueTy::U64,        &[]
    intent_has         = 31,    1,       10,    ValueTy::Bool,       &[]
    intent_keys        = 32,    0,       16,    ValueTy::Compo,      &[]
    intent_keys_page   = 33,    2,       16,    ValueTy::Tuple,      &[]
    intent_keys_after   = 34,    2,       16,    ValueTy::Tuple,      &[]
    intent_get         = 35,    1,       10,    ValueTy::Nil,        &[]
    intent_get_or      = 36,    2,       12,    ValueTy::Nil,        &[]
    intent_require     = 37,    1,       10,    ValueTy::Nil,        &[]
    intent_require_eq  = 38,    2,       10,    ValueTy::Nil,        &[]
    intent_require_absent = 39, 1,       10,    ValueTy::Nil,        &[]
    intent_require_many = 40,   1,       16,    ValueTy::Compo,      &[]
    intent_require_map = 41,    1,       16,    ValueTy::Compo,      &[]
    intent_has_all     = 42,    1,       12,    ValueTy::Bool,       &[]
    intent_has_any     = 43,    1,       12,    ValueTy::Bool,       &[]
    intent_put         = 44,    2,       24,    ValueTy::Nil,        &[]
    intent_put_if_absent = 45,  2,       24,    ValueTy::Bool,       &[]
    intent_put_if_absent_or_match = 46, 2,  24, ValueTy::Bool,       &[]
    intent_put_flat_kv   = 47,    1,       32,    ValueTy::Nil,        &[]
    intent_replace     = 48,    2,       14,    ValueTy::Nil,        &[]
    intent_replace_if  = 49,    3,       16,    ValueTy::Bool,       &[]
    intent_rename        = 50,    2,       14,    ValueTy::Nil,        &[]
    intent_take        = 51,    1,       12,    ValueTy::Nil,        &[]
    intent_take_or     = 52,    2,       14,    ValueTy::Nil,        &[]
    intent_take_if     = 53,    2,       14,    ValueTy::Tuple,      &[]
    intent_take_many   = 54,    1,       16,    ValueTy::Compo,      &[]
    intent_take_map    = 55,    1,       16,    ValueTy::Compo,      &[]
    intent_consume     = 56,    1,       14,    ValueTy::Nil,        &[]
    intent_consume_many = 57,   1,       16,    ValueTy::Compo,      &[]
    intent_del         = 58,    1,       10,    ValueTy::Nil,        &[]
    intent_del_if      = 59,    2,       14,    ValueTy::Bool,       &[]
    intent_del_many    = 60,    1,       12,    ValueTy::U64,        &[]
    intent_append      = 61,    2,       14,    ValueTy::U64,        &[]
    intent_inc         = 62,    2,       14,    ValueTy::U64,        &[]
    intent_add         = 63,    2,       14,    ValueTy::U64,        &[]
    intent_sub         = 64,    2,       14,    ValueTy::U64,        &[]
}
