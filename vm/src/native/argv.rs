use crate::rt::{ItrErr, ItrErrCode, NativeFunc, VmrtRes};
use crate::value::{CallArgsPack, Value, classify_call_args_len};

const FUNC: &str = "func";

fn expect_argv(
    argv: Value,
    kind: &str,
    name: &str,
    expect: usize,
    ec: ItrErrCode,
) -> VmrtRes<Vec<Value>> {
    match classify_call_args_len(expect)? {
        CallArgsPack::Nil => {
            if argv.is_nil() {
                Ok(vec![])
            } else {
                Err(argv_arity_error(kind, name, expect, ec))
            }
        }
        CallArgsPack::Raw => Ok(vec![argv]),
        CallArgsPack::Tuple => {
            let Value::Tuple(args) = argv else {
                return Err(argv_arity_error(kind, name, expect, ec));
            };
            if args.len() != expect {
                return Err(argv_arity_error(kind, name, expect, ec));
            }
            Ok(args.to_vec())
        }
    }
}

fn argv_arity_error(kind: &str, name: &str, expect: usize, ec: ItrErrCode) -> ItrErr {
    ItrErr::new(
        ec,
        &format!("native {} '{}' requires {} argument(s)", kind, name, expect),
    )
}

fn expect_u8(value: &Value, kind: &str, name: &str, label: &str, ec: ItrErrCode) -> VmrtRes<u8> {
    value.extract_u8().map_err(|_| {
        ItrErr::new(
            ec,
            &format!(
                "native {} '{}' requires {} uint convertible to u8",
                kind, name, label
            ),
        )
    })
}

fn expect_u16(value: &Value, kind: &str, name: &str, label: &str, ec: ItrErrCode) -> VmrtRes<u16> {
    value.extract_u16().map_err(|_| {
        ItrErr::new(
            ec,
            &format!(
                "native {} '{}' requires {} uint convertible to u16",
                kind, name, label
            ),
        )
    })
}

fn expect_u64(value: &Value, kind: &str, name: &str, label: &str, ec: ItrErrCode) -> VmrtRes<u64> {
    value.extract_u64().map_err(|_| {
        ItrErr::new(
            ec,
            &format!(
                "native {} '{}' requires {} uint convertible to u64",
                kind, name, label
            ),
        )
    })
}

fn expect_u128(
    value: &Value,
    kind: &str,
    name: &str,
    label: &str,
    ec: ItrErrCode,
) -> VmrtRes<u128> {
    value.extract_u128().map_err(|_| {
        ItrErr::new(
            ec,
            &format!(
                "native {} '{}' requires {} uint convertible to u128",
                kind, name, label
            ),
        )
    })
}

fn expect_bytes(
    value: &Value,
    kind: &str,
    name: &str,
    label: &str,
    ec: ItrErrCode,
) -> VmrtRes<Vec<u8>> {
    match value {
        Value::Bytes(b) => Ok(b.clone()),
        _ => itr_err_fmt!(
            ec,
            "native {} '{}' requires {} bytes argument",
            kind,
            name,
            label
        ),
    }
}

fn expect_address(
    value: &Value,
    kind: &str,
    name: &str,
    label: &str,
    ec: ItrErrCode,
) -> VmrtRes<field::Address> {
    value.extract_address().map_err(|_| {
        ItrErr::new(
            ec,
            &format!(
                "native {} '{}' requires {} address argument",
                kind, name, label
            ),
        )
    })
}

fn expect_list(argv: Value, kind: &str, name: &str, ec: ItrErrCode) -> VmrtRes<Vec<Value>> {
    let Ok(compo) = argv.compo_ref() else {
        return itr_err_fmt!(ec, "native {} '{}' requires list argument", kind, name);
    };
    let Ok(list) = compo.list_ref() else {
        return itr_err_fmt!(ec, "native {} '{}' requires list argument", kind, name);
    };
    Ok(list.iter().cloned().collect())
}

pub fn func_argv(argv: Value, cty: NativeFunc) -> VmrtRes<Vec<Value>> {
    expect_argv(
        argv,
        FUNC,
        cty.name(),
        cty.argv_len_of(),
        ItrErrCode::NativeFuncError,
    )
}

pub fn func_u8(value: &Value, cty: NativeFunc, label: &str) -> VmrtRes<u8> {
    expect_u8(value, FUNC, cty.name(), label, ItrErrCode::NativeFuncError)
}

pub fn func_u16(value: &Value, cty: NativeFunc, label: &str) -> VmrtRes<u16> {
    expect_u16(value, FUNC, cty.name(), label, ItrErrCode::NativeFuncError)
}

pub fn func_u64(value: &Value, cty: NativeFunc, label: &str) -> VmrtRes<u64> {
    expect_u64(value, FUNC, cty.name(), label, ItrErrCode::NativeFuncError)
}

pub fn func_u128(value: &Value, cty: NativeFunc, label: &str) -> VmrtRes<u128> {
    expect_u128(value, FUNC, cty.name(), label, ItrErrCode::NativeFuncError)
}

pub fn func_bytes(value: &Value, cty: NativeFunc, label: &str) -> VmrtRes<Vec<u8>> {
    expect_bytes(value, FUNC, cty.name(), label, ItrErrCode::NativeFuncError)
}

pub fn func_address(value: &Value, cty: NativeFunc, label: &str) -> VmrtRes<field::Address> {
    expect_address(value, FUNC, cty.name(), label, ItrErrCode::NativeFuncError)
}

/// Packed arity-1 whose single Raw argument is a list (`patches`).
pub fn func_list(argv: Value, cty: NativeFunc) -> VmrtRes<Vec<Value>> {
    if cty.argv_len_of() != 1 {
        return itr_err_fmt!(
            ItrErrCode::NativeFuncError,
            "native func '{}' list argv requires catalog arity 1",
            cty.name()
        );
    }
    expect_list(argv, FUNC, cty.name(), ItrErrCode::NativeFuncError)
}
