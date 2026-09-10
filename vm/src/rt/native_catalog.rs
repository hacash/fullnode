/// Native opcode catalog (codec-safe): `NativeCtl` / `NativeEnv` / `NativeFunc`
/// enums with their metadata tables (idx, name, argv length, gas, return type,
/// argv pack). The execute-only dispatch (`NativeFunc::call` / `call_packed`)
/// lives in `vm::native` and must not re-state idx/gas/arity/pack.
use crate::value::ValueTy;

/// How a NativeFunc's single NTFUNC argv slot is filled.
/// Concat: compiler `CAT`s params to bytes (eager, ≤ `value_size`); interpreter
/// uses `extract_call_data`. `extract_call_data` also accepts a one-layer list of
/// Bytes as deferred CAT (≤ `call_data_size`). Concat list = fragments. Packed
/// list = argv vector. Runtime disambiguates via `argv_pack`.
/// Packed: compiler uses NativeCtl packing (0 Nil, 1 Raw, ≥2 Tuple); interpreter
/// passes the `Value` to `call_packed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeArgvPack {
    Concat,
    Packed,
}

macro_rules! native_catalog_argv_pack_api {
    (with_argv_pack, $( $name:ident, $argv_pack:ident )+) => {
        pub const fn argv_pack_of(&self) -> NativeArgvPack {
            match self {
                $( Self::$name => NativeArgvPack::$argv_pack, )+
                Self::Null => NativeArgvPack::Concat,
            }
        }

        pub fn argv_pack(idx: u8) -> VmrtRes<NativeArgvPack> {
            Ok(Self::try_from_u8(idx)?.argv_pack_of())
        }
    };
    (no_argv_pack, $( $name:ident, $argv_pack:ident )+) => {};
}

/// Shared enum + metadata expansion. Func rows carry an explicit `argv_pack`;
/// ctl/env keep the 5-field source form and pass a dummy Concat internally.
macro_rules! native_catalog_enum {
    ( $kind:ident, $EnumName:ident, $ErrCode:ident, $pack_flag:ident,
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $argv_pack:ident )+ ) => {
        #[allow(non_camel_case_types)]
        #[repr(u8)]
        #[derive(Default, PartialEq, Debug, Clone, Copy)]
        pub enum $EnumName {
            #[default] Null = 0u8,
            $( $name = $v, )+
        }

        impl $EnumName {
            #[inline]
            pub fn try_from_u8(idx: u8) -> VmrtRes<Self> {
                match idx {
                    $( x if x == Self::$name as u8 => Ok(Self::$name), )+
                    _ => itr_err_fmt!($ErrCode, "not find {} idx {}", stringify!($EnumName), idx),
                }
            }

            native_catalog_argv_pack_api!($pack_flag, $( $name, $argv_pack )+);

            pub const fn gas_of(&self) -> i64 {
                match self {
                    $( Self::$name => $gas, )+
                    Self::Null => 0,
                }
            }

            pub fn gas(idx: u8) -> VmrtRes<i64> {
                Ok(Self::try_from_u8(idx)?.gas_of())
            }

            pub const fn rty_of(&self) -> ValueTy {
                match self {
                    $( Self::$name => $rty, )+
                    Self::Null => ValueTy::Nil,
                }
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

/// Catalog-only generation: the enum discriminant is the idx; metadata
/// accessors are derived from the same rows. Execute dispatch lives in
/// `vm::native` and must not re-state idx/gas/arity/pack.
///
/// Func rows: `$name = idx, argv_len, gas, rty, argv_pack`
/// (every idx must name Concat or Packed; no default).
/// Ctl/env rows keep the 5-field form.
macro_rules! native_func_env_define {
    ( func, $EnumName:ident, $ErrCode:ident,
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr, $argv_pack:ident )+ ) => {
        native_catalog_enum! {
            func, $EnumName, $ErrCode, with_argv_pack,
            $( $name = $v, $argv_len, $gas, $rty, $argv_pack )+
        }
    };
    ( $kind:ident, $EnumName:ident, $ErrCode:ident,
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:expr )+ ) => {
        native_catalog_enum! {
            $kind, $EnumName, $ErrCode, no_argv_pack,
            $( $name = $v, $argv_len, $gas, $rty, Concat )+
        }
    };
}

native_func_env_define! { env, NativeEnv, NativeEnvError,
    context_address    = 1,    0,        6,    ValueTy::Address
}

native_func_env_define! { func, NativeFunc, NativeFuncError,
    sha2               = 1,    1,       32,    ValueTy::Bytes,     Concat
    sha3               = 2,    1,       32,    ValueTy::Bytes,     Concat
    ripemd160          = 3,    1,       20,    ValueTy::Bytes,     Concat
    keccak256          = 4,    1,       32,    ValueTy::Bytes,     Concat
    blake2s256         = 5,    1,       32,    ValueTy::Bytes,     Concat
    blake2b256         = 6,    1,       32,    ValueTy::Bytes,     Concat

    hac_to_mei         = 31,   1,        6,    ValueTy::U64,       Packed
    hac_to_zhu         = 32,   1,        6,    ValueTy::U128,      Packed

    mei_to_hac         = 36,   1,        6,    ValueTy::Bytes,     Packed
    zhu_to_hac         = 37,   1,        6,    ValueTy::Bytes,     Packed
    
    u64_to_fold64      = 41,   1,        8,    ValueTy::Bytes,     Packed
    fold64_to_u64      = 42,   1,        8,    ValueTy::U64,       Packed
    
    address_ptr        = 51,   1,        4,    ValueTy::U8,        Packed
    pack_asset         = 52,   2,        8,    ValueTy::Bytes,     Packed
    patches            = 53,   1,       24,    ValueTy::Bytes,     Packed
    
    verify_signature   = 81,   3,       96,    ValueTy::Bool,      Packed

    ascii_parse_flat_kv = 101, 8,      64,    ValueTy::Tuple,      Packed
    ascii_validate_transform = 102, 3, 24,    ValueTy::Tuple,      Packed
    ascii_u128_dec_unit = 103, 3,      24,    ValueTy::Tuple,      Packed
    ascii_hex_lower    = 104, 1,       20,    ValueTy::Tuple,      Packed
    ascii_base58_validate_or_echo = 105, 1, 20, ValueTy::Tuple,    Packed
}

native_func_env_define! { ctl, NativeCtl, NativeCtlError,
    defer              = 1,     1,        8,    ValueTy::Nil
    intent_new         = 21,    1,       32,    ValueTy::Handle
    intent_use         = 22,    1,        8,    ValueTy::Nil
    intent_pop         = 23,    0,        8,    ValueTy::Nil
    intent_is_own_handle      = 24,    1,       10,    ValueTy::Bool
    intent_kind        = 25,    0,        8,    ValueTy::Bytes
    intent_kind_is     = 26,    1,        8,    ValueTy::Bool
    intent_destroy     = 27,    0,       10,    ValueTy::Nil
    intent_destroy_if_empty = 28, 0,     10,    ValueTy::Bool
    intent_clear       = 29,    0,       10,    ValueTy::Nil
    intent_len         = 30,    0,       10,    ValueTy::U64
    intent_has         = 31,    1,       10,    ValueTy::Bool
    intent_keys        = 32,    0,       16,    ValueTy::Compo
    intent_keys_page   = 33,    2,       16,    ValueTy::Tuple
    intent_keys_after   = 34,    2,       16,    ValueTy::Tuple
    intent_get         = 35,    1,       10,    ValueTy::Nil
    intent_get_or      = 36,    2,       12,    ValueTy::Nil
    intent_require     = 37,    1,       10,    ValueTy::Nil
    intent_require_eq  = 38,    2,       10,    ValueTy::Nil
    intent_require_absent = 39, 1,       10,    ValueTy::Nil
    intent_require_many = 40,   1,       16,    ValueTy::Compo
    intent_require_map = 41,    1,       16,    ValueTy::Compo
    intent_has_all     = 42,    1,       12,    ValueTy::Bool
    intent_has_any     = 43,    1,       12,    ValueTy::Bool
    intent_put         = 44,    2,       24,    ValueTy::Nil
    intent_put_if_absent = 45,  2,       24,    ValueTy::Bool
    intent_put_if_absent_or_match = 46, 2,  24, ValueTy::Bool
    intent_put_flat_kv   = 47,    1,       32,    ValueTy::Nil
    intent_replace     = 48,    2,       14,    ValueTy::Nil
    intent_replace_if  = 49,    3,       16,    ValueTy::Bool
    intent_rename        = 50,    2,       14,    ValueTy::Nil
    intent_take        = 51,    1,       12,    ValueTy::Nil
    intent_take_or     = 52,    2,       14,    ValueTy::Nil
    intent_take_if     = 53,    2,       14,    ValueTy::Tuple
    intent_take_many   = 54,    1,       16,    ValueTy::Compo
    intent_take_map    = 55,    1,       16,    ValueTy::Compo
    intent_consume     = 56,    1,       14,    ValueTy::Nil
    intent_consume_many = 57,   1,       16,    ValueTy::Compo
    intent_del         = 58,    1,       10,    ValueTy::Nil
    intent_del_if      = 59,    2,       14,    ValueTy::Bool
    intent_del_many    = 60,    1,       12,    ValueTy::U64
    intent_append      = 61,    2,       14,    ValueTy::U64
    intent_inc         = 62,    2,       14,    ValueTy::U64
    intent_add         = 63,    2,       14,    ValueTy::U64
    intent_sub         = 64,    2,       14,    ValueTy::U64
}
