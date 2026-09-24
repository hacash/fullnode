/// Native opcode catalog (codec-safe): `NativeCtl` / `NativeEnv` / `NativeFunc`
/// enums with their metadata tables (idx, name, argv length, gas, return type,
/// argv pack). The execute-only dispatch (`NativeFunc::call` / `call_packed`)
/// is generated from the same rows and calls handlers in `vm::native`.
use crate::value::ValueTy;

// Execution handlers live in `vm::native`, while this catalog is also used by
// codec-only builds. The generated calls therefore stay behind `execute`.
// The accumulator is needed because `macro_rules!` cannot expand directly to
// a single match arm in an outer `match` expression.
macro_rules! native_func_dispatch_match {
    ($cty:ident, $env:ident, $argv:ident; $( $name:ident = $pack:ident )+) => {
        native_func_dispatch_match!(@acc $cty, $env, $argv; []; $( $name = $pack )+)
    };
    (@acc $cty:ident, $env:ident, $argv:ident; [$($arms:tt)*];) => {
        match $cty {
            $($arms)*
            _ => unreachable!(),
        }
    };
    (@acc $cty:ident, $env:ident, $argv:ident;
        [$($arms:tt)*]; $name:ident = Concat $( $rest:ident = $pack:ident )*) => {
        native_func_dispatch_match!(@acc $cty, $env, $argv;
            [$($arms)* Self::$name => crate::native::$name($env, $argv)?,];
            $( $rest = $pack )*)
    };
    (@acc $cty:ident, $env:ident, $argv:ident;
        [$($arms:tt)*]; $name:ident = Packed $( $rest:ident = $pack:ident )*) => {
        native_func_dispatch_match!(@acc $cty, $env, $argv;
            [$($arms)*]; $( $rest = $pack )*)
    };
}

macro_rules! native_func_dispatch_packed_match {
    ($cty:ident, $env:ident, $argv:ident; $( $name:ident = $pack:ident )+) => {
        native_func_dispatch_packed_match!(@acc $cty, $env, $argv; []; $( $name = $pack )+)
    };
    (@acc $cty:ident, $env:ident, $argv:ident; [$($arms:tt)*];) => {
        match $cty {
            $($arms)*
            _ => unreachable!(),
        }
    };
    (@acc $cty:ident, $env:ident, $argv:ident;
        [$($arms:tt)*]; $name:ident = Packed $( $rest:ident = $pack:ident )*) => {
        native_func_dispatch_packed_match!(@acc $cty, $env, $argv;
            [$($arms)* Self::$name => crate::native::$name($env, $argv)?,];
            $( $rest = $pack )*)
    };
    (@acc $cty:ident, $env:ident, $argv:ident;
        [$($arms:tt)*]; $name:ident = Concat $( $rest:ident = $pack:ident )*) => {
        native_func_dispatch_packed_match!(@acc $cty, $env, $argv;
            [$($arms)*]; $( $rest = $pack )*)
    };
}

/// How a NativeFunc's single NTFUNC argv slot is filled.
/// Criterion is the slot's payload, not arity:
/// - Concat: callee wants a byte string. Compiler `CAT`s (1-arg is passthrough);
///   interpreter `extract_call_data` (Nil→`[]`; one-layer list of Bytes is deferred
///   CAT, ≤ `call_data_size`). Hashes, Amount/fold wire, ascii text.
/// - Packed: callee wants structured Values (typed uint/address, Tuple, list argv).
///   Compiler uses 0 Nil / 1 Raw / ≥2 Tuple; interpreter `call_packed`.
///   `patches` is Packed at argc 1 because the slot is a list, not CAT fragments.
/// Runtime disambiguates via `argv_pack`.
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
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:ident, $argv_pack:ident )+ ) => {
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

            /// Declared return type from the catalog row.
            ///
            /// `Nil` means **two different things** depending on the kind:
            /// - `NativeFunc`: the handler really does return `Value::Nil`, and
            ///   `finish_ntfunc` enforces the match.
            /// - `NativeCtl`: `Nil` means *dynamic / unspecified* return type.
            ///   The handler may return any `ValueTy` (e.g. the intent increment
            ///   controls widen to the width of the operands involved), so a
            ///   `Nil` catalog row must **not** be read as "returns `Value::Nil`".
            ///
            /// Only a non-`Nil` `rty` means the value has one fixed, known type.
            pub const fn rty_of(&self) -> ValueTy {
                match self {
                    $( Self::$name => ValueTy::$rty, )+
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
/// accessors and execute dispatch are derived from the same rows; handlers
/// live in `vm::native` and idx/gas/arity/pack are not restated there.
///
/// Func rows: `$name = idx, argv_len, gas, ValueTy variant, argv_pack`.
/// Every row must name Concat or Packed and has a same-named handler in
/// `vm::native`; dispatch is generated from these rows.
/// Ctl/env rows keep the 5-field form.
///
/// `rty` semantics differ by kind:
/// - Func rows declare the one type the handler actually returns; the runtime
///   asserts it in `finish_ntfunc`.
/// - Ctl rows are metadata only. `rty = Nil` is the marker for a *dynamic /
///   unspecified* return type (a handler may yield any `ValueTy`), so `Nil`
///   here does not promise `Value::Nil`. Any non-`Nil` ctl `rty` is a real,
///   fixed return type.
macro_rules! native_func_env_define {
    ( func, $EnumName:ident, $ErrCode:ident,
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:ident, $argv_pack:ident )+ ) => {
        native_catalog_enum! {
            func, $EnumName, $ErrCode, with_argv_pack,
            $( $name = $v, $argv_len, $gas, $rty, $argv_pack )+
        }

        #[cfg(feature = "execute")]
        impl $EnumName {
            pub fn call(
                env: NativeFnEnv<'_>,
                idx: u8,
                argv: &[u8],
            ) -> VmrtRes<(crate::value::Value, i64)> {
                let cty = Self::try_from_u8(idx)?;
                if cty.argv_pack_of() != NativeArgvPack::Concat {
                    return itr_err_fmt!(
                        $ErrCode,
                        "native func {} requires packed argv",
                        cty.name()
                    );
                }
                let r = native_func_dispatch_match!(
                    cty, env, argv; $( $name = $argv_pack )+
                );
                crate::native::finish_ntfunc(cty, r)
            }

            pub fn call_packed(
                env: NativeFnEnv<'_>,
                idx: u8,
                argv: crate::value::Value,
            ) -> VmrtRes<(crate::value::Value, i64)> {
                let cty = Self::try_from_u8(idx)?;
                if cty.argv_pack_of() != NativeArgvPack::Packed {
                    return itr_err_fmt!(
                        $ErrCode,
                        "native func {} requires concat argv",
                        cty.name()
                    );
                }
                let r = native_func_dispatch_packed_match!(
                    cty, env, argv; $( $name = $argv_pack )+
                );
                crate::native::finish_ntfunc(cty, r)
            }
        }
    };
    ( $kind:ident, $EnumName:ident, $ErrCode:ident,
      $( $name:ident = $v:expr, $argv_len:expr, $gas:expr, $rty:ident )+ ) => {
        native_catalog_enum! {
            $kind, $EnumName, $ErrCode, no_argv_pack,
            $( $name = $v, $argv_len, $gas, $rty, Concat )+
        }
    };
}

native_func_env_define! { env, NativeEnv, NativeEnvError,
    context_address     = 1,    0,      6,    Address
}

native_func_env_define! { func, NativeFunc, NativeFuncError,
    sha2                = 1,    1,     25,    Bytes,   Concat
    sha3                = 2,    1,     32,    Bytes,   Concat
    ripemd160           = 3,    1,     20,    Bytes,   Concat
    keccak256           = 4,    1,     32,    Bytes,   Concat
    blake2s256          = 5,    1,     32,    Bytes,   Concat
    blake2b256          = 6,    1,     32,    Bytes,   Concat


    sha2_prefix_u64     = 33, 1,      25,    U64,     Concat
    sha2_prefix_u160    = 34, 1,      25,    Bytes,   Concat
    sha2_prefix_u64_cat = 35, 1,      25,    U64,     Packed

    verify_signature    = 40,   3,     40,    Bool,    Packed
    asset_meta_fields   = 41,   1,      8,    Tuple,   Packed
    asset_meta_require  = 42,   4,     10,    Bool,    Packed
    check_addr_set      = 43,   3,     16,    Bool,    Packed
    check_hacd_wire     = 44,   2,     10,    Bool,    Packed
    fee_add_bps_ceil    = 45,   2,      8,    U64,     Packed
    
    hac_to_mei          = 51,   1,      6,    U64,     Concat
    hac_to_mei_checked  = 52,   1,      8,    U64,     Concat
    hac_to_zhu          = 53,   1,      6,    U128,    Concat
    hac_to_zhu_checked  = 54,   1,      8,    U128,    Concat
    hac_to_unit         = 55,   2,      6,    U128,    Packed
    hac_to_unit_checked = 56,   2,      8,    U128,    Packed
    hac_is_exact_mei    = 57,   1,      6,    Bool,    Concat
    hac_is_exact_zhu    = 58,   1,      6,    Bool,    Concat
    hac_is_exact_unit   = 59,   2,      6,    Bool,    Packed
    mei_to_hac          = 60,   1,      6,    Bytes,   Packed
    zhu_to_hac          = 61,   1,      6,    Bytes,   Packed
    unit_to_hac         = 62,   2,      6,    Bytes,   Packed
    hac_zhu_tail        = 63,   1,      6,    Bytes,   Concat

    u64_to_fold64       = 71,   1,      6,    Bytes,   Packed
    fold64_to_u64       = 72,   1,      6,    U64,     Concat

    address_ptr         = 81,   1,      4,    U8,      Packed
    patches             = 82,   1,     16,    Bytes,   Packed
    pack_asset          = 83,   2,      8,    Bytes,   Packed
    pack_fungible_amount = 84,  2,     12,    Bytes,   Packed

    address_version     = 91,   1,      4,    U8,      Packed
    is_privkey_unknown  = 92,   1,      4,    Bool,    Packed
    is_privkey_not_unknown = 93, 1,     4,    Bool,    Packed
    is_privkey          = 94,   1,      4,    Bool,    Packed
    is_contract         = 95,   1,      4,    Bool,    Packed
    is_scriptmh         = 96,   1,      4,    Bool,    Packed

    buf_u16             = 101, 2,       8,    U16,     Packed
    buf_u32             = 102, 2,       8,    U32,     Packed
    buf_u64             = 103, 2,       8,    U64,     Packed
    buf_address         = 104, 2,       8,    Address, Packed
    buf_scan_u32        = 105, 5,      12,    Tuple,   Packed

    ascii_parse_flat_kv = 121, 8,      40,    Tuple,   Packed
    ascii_validate_transform = 122, 3, 24,    Tuple,   Packed
    ascii_u128_dec_unit = 123, 3,      20,    Tuple,   Packed
    ascii_hex_lower    = 124, 1,       16,    Tuple,   Concat
    ascii_base58_validate_or_echo = 125, 1, 20, Tuple, Concat
}

native_func_env_define! { ctl, NativeCtl, NativeCtlError,
    defer              = 1,     1,       8,    Nil
    defer_current      = 2,     0,       8,    Nil

    intent_new         = 21,    1,      24,    Handle
    intent_new_flat_kv = 22,    2,      40,    Handle
    intent_use         = 23,    1,       8,    Nil
    intent_use_kind    = 24,    1,       8,    Nil
    intent_pop         = 25,    0,       8,    Nil
    intent_bound       = 26,    0,       6,    Bool
    intent_is_own_handle = 27,  1,       8,    Bool
    intent_kind        = 28,    0,       6,    Bytes
    intent_kind_is     = 29,    1,       6,    Bool
    intent_destroy     = 30,    0,       8,    Nil
    intent_destroy_if_empty = 31, 0,     8,    Bool
    intent_clear       = 32,    0,       6,    Nil
    intent_len         = 33,    0,       6,    U64
    intent_has         = 34,    1,       6,    Bool
    intent_keys        = 35,    0,      10,    Compo
    intent_keys_page   = 36,    2,      10,    Tuple
    intent_keys_after  = 37,    2,      10,    Tuple
    intent_get         = 38,    1,       6,    Nil
    intent_get_or      = 39,    2,       8,    Nil
    intent_open_page   = 40,    2,      12,    Tuple
    intent_require     = 41,    1,       6,    Nil
    intent_require_eq  = 42,    2,       6,    Nil
    intent_require_absent = 43, 1,       6,    Nil
    intent_require_many = 44,   1,       8,    Compo
    intent_require_map = 45,    1,       8,    Compo
    intent_has_all     = 46,    1,       8,    Bool
    intent_has_any     = 47,    1,       6,    Bool
    intent_put         = 48,    2,      12,    Nil
    intent_put_if_absent = 49,  2,      12,    Bool
    intent_put_if_absent_or_match = 50, 2, 12, Bool
    intent_put_flat_kv   = 51,  1,      16,    Nil
    intent_replace     = 52,    2,       6,    Nil
    intent_replace_if  = 53,    3,       8,    Bool
    intent_rename      = 54,    2,       6,    Nil
    intent_take        = 55,    1,       6,    Nil
    intent_take_or     = 56,    2,       8,    Nil
    intent_take_if     = 57,    2,       8,    Tuple
    intent_take_many   = 58,    1,      10,    Compo
    intent_take_map    = 59,    1,      10,    Compo
    intent_consume     = 60,    1,       8,    Nil
    intent_consume_many = 61,   1,      10,    Compo
    intent_del         = 62,    1,       6,    Nil
    intent_del_if      = 63,    2,       8,    Bool
    intent_del_many    = 64,    1,       6,    U64
    intent_append      = 65,    2,       8,    U64
    intent_inc         = 66,    2,       8,    Nil
    intent_add         = 67,    2,       8,    Nil
    intent_sub         = 68,    2,       8,    Nil
}
