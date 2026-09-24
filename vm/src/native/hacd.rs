//! Diamond-name list check. Returns Bool; a failed check is `false`, not an abort.

use field::DiamondName;

use crate::rt::{NativeFnEnv, NativeFunc, VmrtRes};
use crate::value::Value;

use super::{func_argv, func_bytes, func_u64};

const NAME_LEN: usize = 6;
const MAX_NAMES: u64 = 200;

pub(crate) fn check_hacd_wire(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::check_hacd_wire;
    let a = func_argv(argv, cty)?;
    let count = func_u64(&a[0], cty, "count")?;
    let names = func_bytes(&a[1], cty, "names")?;
    Ok(Value::Bool(wire_ok(count, &names)))
}

fn wire_ok(count: u64, names: &[u8]) -> bool {
    if count > MAX_NAMES {
        return false;
    }
    let need = (count as usize).saturating_mul(NAME_LEN);
    if names.len() != need {
        return false;
    }
    let mut seen: Vec<&[u8]> = Vec::with_capacity(count as usize);
    for name in names.chunks(NAME_LEN) {
        if !DiamondName::is_valid(name) || seen.contains(&name) {
            return false;
        }
        seen.push(name);
    }
    true
}
