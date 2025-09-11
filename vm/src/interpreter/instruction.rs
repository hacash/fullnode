
fn check_failed_tip(op: &str, x: &Value, y: &Value) -> String {
    format!("arithmetic {} check failed with {:?} and {:?}", op, x, y)
}

/////////////////////// logic ///////////////////////


macro_rules! lgcv {
    ($v: expr) => {
        Ok(Bool(maybe!($v, true, false)))
    }
}

macro_rules! lgcdo {
    ($op: ident, $l: expr, $r: expr, $t: ty) => {
        lgcv!( (*$l as $t).$op(&(*$r as $t)) )
    }
}

fn lgc_and(x: &Value, y: &Value) -> VmrtRes<Value> {
    let ok = x.check_true() && y.check_true();
    lgcv!(ok)
}

fn lgc_or(x: &Value, y: &Value) -> VmrtRes<Value> {
    let ok = x.check_true() || y.check_true();
    lgcv!(ok)
}

fn lgc_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    match (x, y) {
        (Bool(l), Bool(r)) => lgcv!(l.eq(r)) ,
        (Bytes(l), Bytes(r)) => lgcv!(l.eq(r)) ,
        _ => lgcuintmatch!(eq, x, y)
    }
}

fn lgc_not_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    match (x, y) {
        (Bool(l), Bool(r)) => lgcv!(l.ne(r)) ,
        (Bytes(l), Bytes(r)) => lgcv!(l.ne(r)) ,
        _ => lgcuintmatch!(ne, x, y)
    }
}

fn lgc_less(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgcuintmatch!(lt, x, y)
}

fn lgc_less_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgcuintmatch!(le, x, y)
}

fn lgc_greater(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgcuintmatch!(gt, x, y)
}

fn lgc_greater_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgcuintmatch!(ge, x, y)
}


fn bit_and(x: &Value, y: &Value) -> VmrtRes<Value> {
    bitop!(x, y, bitand)
}

fn bit_or(x: &Value, y: &Value) -> VmrtRes<Value> {
    bitop!(x, y, bitor)
}

fn bit_xor(x: &Value, y: &Value) -> VmrtRes<Value> {
    bitop!(x, y, bitxor)
}

fn bit_shl(x: &Value, y: &Value) -> VmrtRes<Value> {
    bitop!(x, y, shl)
}

fn bit_shr(x: &Value, y: &Value) -> VmrtRes<Value> {
    bitop!(x, y, shr)
}



/////////////////////// arithmetic ///////////////////////



macro_rules! ahmtdocheck {
    ( $x: expr, $y: expr, $op: ident, $tip: expr ) => {
        ahmtdo!($x, $y, $op)
        .ok_or_else(||ItrErr::new(Arithmetic, &check_failed_tip($tip, $x, $y)))
    }
}

fn add_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_add, "add")
}

fn sub_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_sub, "sub")
}

fn mul_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_mul, "mul")
}

fn div_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_div, "div")
}

fn mod_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_rem, "mod") // rem = mod
}

// the value is must within u32
fn pow_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    match (x, y) {
        (U8(l), U8(r))   => <u8>::checked_pow(*l, *r as u32).map(Value::U8),
        (U16(l), U16(r)) => <u16>::checked_pow(*l, *r as u32).map(Value::U16),
        (U32(l), U32(r)) => <u32>::checked_pow(*l, *r).map(Value::U32),
        (_, _) => return itr_err_fmt!(Arithmetic, 
            "cannot do pow arithmetic between {:?} and {:?}", x, y),
    }.ok_or_else(||ItrErr::new(Arithmetic, &check_failed_tip("pow", x, y)))
}

fn max_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    let a = x.checked_uint()?;
    let b = y.checked_uint()?;
    Ok(maybe!(a > b, x.clone(), y.clone()))
}


fn min_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    let a = x.checked_uint()?;
    let b = y.checked_uint()?;
    Ok(maybe!(a < b, x.clone(), y.clone()))
}


