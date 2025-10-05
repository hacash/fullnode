

fn locop_arithmetic<F>(x: &mut Value, y: &mut Value, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>
{
    cast_arithmetic(x, y)?;
    let v = f(&x, &y)?;
    *x = v;
    Ok(())
}


/**
*   such as: v = x + y
*/
fn binop_arithmetic<F>(operand_stack: &mut Stack, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>
{
    let mut y = operand_stack.pop()?;
    let x = operand_stack.peek()?;
    locop_arithmetic(x, &mut y, f)
}


/**
*   binop_between
*   such as: v = x && y
*/

fn locop_btw<F>(x: &mut Value, y: &mut Value, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>
{
    let v = f(&x, &y)?;
    *x = v;
    Ok(())
}

fn binop_btw<F>(operand_stack: &mut Stack, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>
{
    let mut y = operand_stack.pop()?;
    let x = operand_stack.peek()?;
    locop_btw(x, &mut y, f)
}




macro_rules! bitop {
    ( $x: expr, $y: expr, $op: ident ) => {
        Ok(match ($x, $y) {
            (U8(l), U8(r))     => Value::U8((*l).$op(*r)),
            (U16(l), U16(r))   => Value::U16((*l).$op(*r)),
            (U32(l), U32(r))   => Value::U32((*l).$op(*r)),
            (U64(l), U64(r))   => Value::U64((*l).$op(*r)),
            (U128(l), U128(r)) => Value::U128((*l).$op(*r)),
            (_, _) => return itr_err_fmt!(Arithmetic, 
                "cannot do bit ops between {:?} and {:?}", $x, $y),
        })
    }
}


macro_rules! ahmtdo {
    ( $x: expr, $y: expr, $op: ident ) => {
        match ($x, $y) {
            (U8(l), U8(r))     => <u8>::$op(*l, *r).map(Value::U8),
            (U16(l), U16(r))   => <u16>::$op(*l, *r).map(Value::U16),
            (U32(l), U32(r))   => <u32>::$op(*l, *r).map(Value::U32),
            (U64(l), U64(r))   => <u64>::$op(*l, *r).map(Value::U64),
            (U128(l), U128(r)) => <u128>::$op(*l, *r).map(Value::U128),
            (_, _) => return itr_err_fmt!(Arithmetic, 
                "cannot do arithmetic between {:?} and {:?}", $x, $y),
        }
    }
}


macro_rules! lgcyuintmatch {
    ($op: ident, $x: expr, $y: expr) => {
        match ($x, $y) {
            (U8(l), U8(r)) =>     lgcdo!($op, l, r, u8),
            (U8(l), U16(r)) =>    lgcdo!($op, l, r, u16),
            (U8(l), U32(r)) =>    lgcdo!($op, l, r, u32),
            (U8(l), U64(r)) =>    lgcdo!($op, l, r, u64),
            (U8(l), U128(r)) =>   lgcdo!($op, l, r, u128),

            (U16(l), U8(r)) =>    lgcdo!($op, l, r, u16),
            (U16(l), U16(r)) =>   lgcdo!($op, l, r, u16),
            (U16(l), U32(r)) =>   lgcdo!($op, l, r, u32),
            (U16(l), U64(r)) =>   lgcdo!($op, l, r, u64),
            (U16(l), U128(r)) =>  lgcdo!($op, l, r, u128),

            (U32(l), U8(r)) =>    lgcdo!($op, l, r, u32),
            (U32(l), U16(r)) =>   lgcdo!($op, l, r, u32),
            (U32(l), U32(r)) =>   lgcdo!($op, l, r, u32),
            (U32(l), U64(r)) =>   lgcdo!($op, l, r, u64),
            (U32(l), U128(r)) =>  lgcdo!($op, l, r, u128),

            (U64(l), U8(r)) =>    lgcdo!($op, l, r, u64),
            (U64(l), U16(r)) =>   lgcdo!($op, l, r, u64),
            (U64(l), U32(r)) =>   lgcdo!($op, l, r, u64),
            (U64(l), U64(r)) =>   lgcdo!($op, l, r, u64),
            (U64(l), U128(r)) =>  lgcdo!($op, l, r, u128),

            (U128(l), U8(r)) =>    lgcdo!($op, l, r, u128),
            (U128(l), U16(r)) =>   lgcdo!($op, l, r, u128),
            (U128(l), U32(r)) =>   lgcdo!($op, l, r, u128),
            (U128(l), U64(r)) =>   lgcdo!($op, l, r, u128),
            (U128(l), U128(r)) =>  lgcdo!($op, l, r, u128),

            (_l, _r) => return itr_err_fmt!(Arithmetic, 
                "cannot do logic operand <{}> between {:?} and {:?}", stringify!($op), $x, $y),
        }
    }
}





