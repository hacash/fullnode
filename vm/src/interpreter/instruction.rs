/////////////////////// opset ///////////////////////

fn locop_arithmetic<F>(x: &mut Value, y: &mut Value, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>,
{
    let (lx, ry) = Value::arithmetic_args2(x, y)?;
    let v = f(&lx, &ry)?;
    *x = v;
    Ok(())
}

/* * *   such as: v = x + y */
fn binop_arithmetic<F>(operand_stack: &mut Stack, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>,
{
    let mut y = operand_stack.pop()?;
    let x = operand_stack.peek()?;
    locop_arithmetic(x, &mut y, f)
}

fn locop_arithmetic3<F>(x: &mut Value, y: &mut Value, z: &mut Value, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value, &Value) -> VmrtRes<Value>,
{
    let (lx, my, rz) = Value::arithmetic_args3(x, y, z)?;
    let v = f(&lx, &my, &rz)?;
    *x = v;
    Ok(())
}

fn triop_arithmetic<F>(operand_stack: &mut Stack, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value, &Value) -> VmrtRes<Value>,
{
    let mut z = operand_stack.pop()?;
    let mut y = operand_stack.pop()?;
    let x = operand_stack.peek()?;
    locop_arithmetic3(x, &mut y, &mut z, f)
}

fn locop_arithmetic4<F>(x: &mut Value, y: &mut Value, z: &mut Value, w: &mut Value, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value, &Value, &Value) -> VmrtRes<Value>,
{
    let (lx, my, rz, qw) = Value::arithmetic_args4(x, y, z, w)?;
    let v = f(&lx, &my, &rz, &qw)?;
    *x = v;
    Ok(())
}

fn quadop_arithmetic<F>(operand_stack: &mut Stack, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value, &Value, &Value) -> VmrtRes<Value>,
{
    let mut w = operand_stack.pop()?;
    let mut z = operand_stack.pop()?;
    let mut y = operand_stack.pop()?;
    let x = operand_stack.peek()?;
    locop_arithmetic4(x, &mut y, &mut z, &mut w, f)
}

/* * *   binop_between *   such as: v = x && y */

fn locop_btw<F>(x: &mut Value, y: &mut Value, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>,
{
    let v = f(&x, &y)?;
    *x = v;
    Ok(())
}

fn binop_btw<F>(operand_stack: &mut Stack, f: F) -> VmrtErr
where
    F: FnOnce(&Value, &Value) -> VmrtRes<Value>,
{
    let mut y = operand_stack.pop()?;
    let x = operand_stack.peek()?;
    locop_btw(x, &mut y, f)
}

macro_rules! bitop {
    ( $x: expr, $y: expr, $op: ident ) => {
        Ok(match ($x, $y) {
            (U8(l), U8(r)) => Value::U8((*l).$op(*r)),
            (U16(l), U16(r)) => Value::U16((*l).$op(*r)),
            (U32(l), U32(r)) => Value::U32((*l).$op(*r)),
            (U64(l), U64(r)) => Value::U64((*l).$op(*r)),
            (U128(l), U128(r)) => Value::U128((*l).$op(*r)),
            (_, _) => {
                return itr_err_fmt!(
                    Arithmetic,
                    "cannot do bit ops between {:?} and {:?}",
                    $x,
                    $y
                )
            }
        })
    };
}

macro_rules! ahmtdo {
    ( $x: expr, $y: expr, $op: ident ) => {
        match ($x, $y) {
            (U8(l), U8(r)) => <u8>::$op(*l, *r).map(Value::U8),
            (U16(l), U16(r)) => <u16>::$op(*l, *r).map(Value::U16),
            (U32(l), U32(r)) => <u32>::$op(*l, *r).map(Value::U32),
            (U64(l), U64(r)) => <u64>::$op(*l, *r).map(Value::U64),
            (U128(l), U128(r)) => <u128>::$op(*l, *r).map(Value::U128),
            (_, _) => {
                return itr_err_fmt!(
                    Arithmetic,
                    "cannot do arithmetic between {:?} and {:?}",
                    $x,
                    $y
                )
            }
        }
    };
}

/////////////////////// logic ///////////////////////

fn check_failed_tip(op: &str, x: &Value, y: &Value) -> String {
    format!("arithmetic {} check failed with {:?} and {:?}", op, x, y)
}

fn check_failed_tip3(op: &str, x: &Value, y: &Value, z: &Value) -> String {
    format!(
        "arithmetic {} check failed with {:?}, {:?} and {:?}",
        op, x, y, z
    )
}

fn check_failed_tip4(op: &str, x: &Value, y: &Value, z: &Value, w: &Value) -> String {
    format!(
        "arithmetic {} check failed with {:?}, {:?}, {:?} and {:?}",
        op, x, y, z, w
    )
}

fn check_failed_tip1(op: &str, x: &Value) -> String {
    format!("arithmetic {} check failed with {:?}", op, x)
}

#[inline]
fn cast_uint_like_sample(sample: &Value, out: u128, err: impl Fn() -> ItrErr) -> VmrtRes<Value> {
    Ok(match sample {
        U8(..) => Value::U8(u8::try_from(out).map_err(|_| err())?),
        U16(..) => Value::U16(u16::try_from(out).map_err(|_| err())?),
        U32(..) => Value::U32(u32::try_from(out).map_err(|_| err())?),
        U64(..) => Value::U64(u64::try_from(out).map_err(|_| err())?),
        U128(..) => Value::U128(out),
        _ => return Err(err()),
    })
}

#[inline]
fn require_nonzero_u128(d: u128, err: impl Fn() -> ItrErr) -> VmrtRes<u128> {
    if d == 0 {
        Err(err())
    } else {
        Ok(d)
    }
}

#[inline]
fn half_up_round_u128_threshold(div: u128) -> u128 {
    div - div / 2
}

#[inline]
fn ceil_quot_if_rem_u128(quo: u128, rem: u128, err: impl Fn() -> ItrErr) -> VmrtRes<u128> {
    if rem == 0 {
        Ok(quo)
    } else {
        quo.checked_add(1).ok_or_else(err)
    }
}

fn lgc_and(x: &Value, y: &Value) -> VmrtRes<Value> {
    let lx = x.extract_bool()?;
    let ry = y.extract_bool()?;
    Ok(Value::bool(lx && ry))
}

fn lgc_or(x: &Value, y: &Value) -> VmrtRes<Value> {
    let lx = x.extract_bool()?;
    let ry = y.extract_bool()?;
    Ok(Value::bool(lx || ry))
}

#[allow(unused)]
fn lgc_not(x: &Value) -> VmrtRes<Value> {
    let v = x.extract_bool()?;
    Ok(Value::bool(!v))
}

fn lgc_equal_bool(x: &Value, y: &Value) -> VmrtRes<bool> {
    value_content_eq(x, y)
}

fn lgc_compare_fee(x: &Value, y: &Value, gas_extra: &GasExtra) -> usize {
    value_compare_fee(x, y, gas_extra.container_cmp_header)
}

fn lgc_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    Ok(Value::bool(lgc_equal_bool(x, y)?))
}

fn lgc_not_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    Ok(Value::bool(!lgc_equal_bool(x, y)?))
}

fn lgc_ord_cmp<F>(x: &Value, y: &Value, f: F) -> VmrtRes<Value>
where
    F: FnOnce(u128, u128) -> bool,
{
    if !x.is_uint() || !y.is_uint() {
        return itr_err_fmt!(
            Arithmetic,
            "ordered compare only supports uint operands, got {:?} and {:?}",
            x,
            y
        );
    }
    let lx = x.extract_u128()?;
    let ry = y.extract_u128()?;
    Ok(Value::bool(f(lx, ry)))
}

fn lgc_less(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgc_ord_cmp(x, y, |l, r| l < r)
}

fn lgc_less_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgc_ord_cmp(x, y, |l, r| l <= r)
}

fn lgc_greater(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgc_ord_cmp(x, y, |l, r| l > r)
}

fn lgc_greater_equal(x: &Value, y: &Value) -> VmrtRes<Value> {
    lgc_ord_cmp(x, y, |l, r| l >= r)
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

fn bit_shift_overflow(op: &str, x: &Value, y: &Value) -> ItrErr {
    ItrErr::new(
        Arithmetic,
        &format!("bit {} shift overflow between {:?} and {:?}", op, x, y),
    )
}

fn bit_shl(x: &Value, y: &Value) -> VmrtRes<Value> {
    let res = match (x, y) {
        (U8(l), U8(r)) => <u8>::checked_shl(*l, *r as u32).map(Value::U8),
        (U16(l), U16(r)) => <u16>::checked_shl(*l, *r as u32).map(Value::U16),
        (U32(l), U32(r)) => <u32>::checked_shl(*l, *r as u32).map(Value::U32),
        (U64(l), U64(r)) => {
            let s = u32::try_from(*r).map_err(|_| bit_shift_overflow("left", x, y))?;
            <u64>::checked_shl(*l, s).map(Value::U64)
        }
        (U128(l), U128(r)) => {
            if *r > u32::MAX as u128 {
                return Err(bit_shift_overflow("left", x, y));
            }
            <u128>::checked_shl(*l, *r as u32).map(Value::U128)
        }
        (_, _) => return itr_err_fmt!(Arithmetic, "cannot do bit ops between {:?} and {:?}", x, y),
    };
    res.ok_or_else(|| bit_shift_overflow("left", x, y))
}

fn bit_shr(x: &Value, y: &Value) -> VmrtRes<Value> {
    let res = match (x, y) {
        (U8(l), U8(r)) => <u8>::checked_shr(*l, *r as u32).map(Value::U8),
        (U16(l), U16(r)) => <u16>::checked_shr(*l, *r as u32).map(Value::U16),
        (U32(l), U32(r)) => <u32>::checked_shr(*l, *r as u32).map(Value::U32),
        (U64(l), U64(r)) => {
            let s = u32::try_from(*r).map_err(|_| bit_shift_overflow("right", x, y))?;
            <u64>::checked_shr(*l, s).map(Value::U64)
        }
        (U128(l), U128(r)) => {
            if *r > u32::MAX as u128 {
                return Err(bit_shift_overflow("right", x, y));
            }
            <u128>::checked_shr(*l, *r as u32).map(Value::U128)
        }
        (_, _) => return itr_err_fmt!(Arithmetic, "cannot do bit ops between {:?} and {:?}", x, y),
    };
    res.ok_or_else(|| bit_shift_overflow("right", x, y))
}

/////////////////////// arithmetic ///////////////////////

macro_rules! ahmtdocheck {
    ( $x: expr, $y: expr, $op: ident, $tip: expr ) => {
        ahmtdo!($x, $y, $op).ok_or_else(|| ItrErr::new(Arithmetic, &check_failed_tip($tip, $x, $y)))
    };
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

fn div_checked(x: &Value, y: &Value, op: &'static str) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_div, op)
}

fn div_up_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    div_with_round_checked(x, y, FinRoundPolicy::Ceil, "div_up")
}

fn div_exact_op_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    div_with_round_checked(x, y, FinRoundPolicy::Exact, "div_exact_op")
}

fn mod_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    ahmtdocheck!(x, y, checked_rem, "mod") // rem = mod
}

#[inline(always)]
fn add_mod_u128(a: u128, b: u128, m: u128) -> u128 {
    let lhs = a % m;
    let rhs = b % m;
    let gap = m - rhs;
    if lhs >= gap {
        lhs - gap
    } else {
        lhs + rhs
    }
}

fn mul_mod_u128(a: u128, b: u128, m: u128) -> u128 {
    let mut lhs = a % m;
    let mut rhs = b % m;
    let mut out = 0u128;
    while rhs != 0 {
        if rhs & 1 == 1 {
            out = add_mod_u128(out, lhs, m);
        }
        rhs >>= 1;
        if rhs != 0 {
            lhs = add_mod_u128(lhs, lhs, m);
        }
    }
    out
}

#[inline(always)]
fn low_bits_mask(bits: u32) -> u128 {
    match bits {
        0 => 0,
        128.. => u128::MAX,
        _ => (1u128 << bits) - 1,
    }
}

#[inline(always)]
fn mul_wide_u128(a: u128, b: u128) -> (u128, u128) {
    let (lo, hi) = a.carrying_mul(b, 0);
    (hi, lo)
}

fn add_u256_u128(hi: u128, lo: u128, add: u128) -> Option<(u128, u128)> {
    let (lo, carry) = lo.overflowing_add(add);
    Some((hi.checked_add(carry as u128)?, lo))
}

fn add_u256(ahi: u128, alo: u128, bhi: u128, blo: u128) -> Option<(u128, u128)> {
    let (lo, carry) = alo.overflowing_add(blo);
    let hi = ahi.checked_add(bhi)?.checked_add(carry as u128)?;
    Some((hi, lo))
}

fn sub_u256(ahi: u128, alo: u128, bhi: u128, blo: u128) -> Option<(u128, u128)> {
    let (lo, borrow) = alo.overflowing_sub(blo);
    let hi = ahi.checked_sub(bhi)?.checked_sub(borrow as u128)?;
    Some((hi, lo))
}

fn sub_u256_u128(hi: u128, lo: u128, sub: u128) -> Option<(u128, u128)> {
    sub_u256(hi, lo, 0, sub)
}

fn mul_xy_addsub_z_fit_u128(
    x: u128,
    y: u128,
    z: u128,
    add_z: bool,
    err: impl Fn() -> ItrErr,
) -> VmrtRes<u128> {
    let (hi, lo) = mul_wide_u128(x, y);
    let (hi, lo) = if add_z {
        add_u256_u128(hi, lo, z).ok_or_else(|| err())?
    } else {
        sub_u256_u128(hi, lo, z).ok_or_else(|| err())?
    };
    if hi != 0 {
        return Err(err());
    }
    Ok(lo)
}

fn mul_xy_addsub_z_div_u128(
    x: u128,
    y: u128,
    z: u128,
    div: u128,
    add_z: bool,
    round: FinRoundPolicy,
    err: impl Fn() -> ItrErr,
) -> VmrtRes<u128> {
    let (hi, lo) = mul_wide_u128(x, y);
    let (hi, lo) = if add_z {
        add_u256_u128(hi, lo, z).ok_or_else(|| err())?
    } else {
        sub_u256_u128(hi, lo, z).ok_or_else(|| err())?
    };
    div_u256_by_u128_with_round(hi, lo, div, round, err)
}

fn div_u256_by_u128_to_u128(hi: u128, lo: u128, d: u128) -> Option<(u128, u128)> {
    if d == 0 || hi >= d {
        return None;
    }
    let mut rem = hi;
    let mut quo = 0u128;
    for shift in (0..128).rev() {
        let carry = rem >> 127;
        rem = (rem << 1) | ((lo >> shift) & 1);
        if carry != 0 || rem >= d {
            rem = rem.wrapping_sub(d);
            quo |= 1u128 << shift;
        }
    }
    Some((quo, rem))
}

fn div_u256_by_u129_to_u128(
    n_hi: u128,
    n_lo: u128,
    d_hi: u128,
    d_lo: u128,
) -> Option<(u128, u128, u128)> {
    if d_hi > 1 || (d_hi == 0 && d_lo == 0) {
        return None;
    }
    if d_hi == 0 {
        let (quo, rem) = div_u256_by_u128_to_u128(n_hi, n_lo, d_lo)?;
        return Some((quo, 0, rem));
    }
    let mut quo = 0u128;
    let mut rem_hi = 0u128;
    let mut rem_lo = 0u128;
    for shift in (0..256).rev() {
        let next_bit = if shift >= 128 {
            (n_hi >> (shift - 128)) & 1
        } else {
            (n_lo >> shift) & 1
        };
        let carry = rem_lo >> 127;
        rem_hi = (rem_hi << 1) | carry;
        rem_lo = (rem_lo << 1) | next_bit;
        if cmp_u256(rem_hi, rem_lo, d_hi, d_lo).is_ge() {
            let (new_hi, new_lo) = sub_u256(rem_hi, rem_lo, d_hi, d_lo)?;
            rem_hi = new_hi;
            rem_lo = new_lo;
            if shift >= 128 {
                return None;
            }
            quo |= 1u128 << shift;
        }
    }
    Some((quo, rem_hi, rem_lo))
}

fn shr_u256_to_u128(hi: u128, lo: u128, shift: u32) -> Option<(u128, bool)> {
    match shift {
        0 => {
            if hi == 0 {
                Some((lo, false))
            } else {
                None
            }
        }
        1..=127 => {
            if hi >> shift != 0 {
                return None;
            }
            let out = (hi << (128 - shift)) | (lo >> shift);
            let dropped = lo & low_bits_mask(shift) != 0;
            Some((out, dropped))
        }
        128 => Some((hi, lo != 0)),
        129..=255 => {
            let rhs = shift - 128;
            let out = hi >> rhs;
            let dropped = lo != 0 || hi & low_bits_mask(rhs) != 0;
            Some((out, dropped))
        }
        _ => None,
    }
}

fn mul_u256_u128_to_u256_checked(hi: u128, lo: u128, mul: u128) -> Option<(u128, u128)> {
    let (lo_hi, lo_lo) = mul_wide_u128(lo, mul);
    let (hi_hi, hi_lo) = mul_wide_u128(hi, mul);
    if hi_hi != 0 {
        return None;
    }
    let out_hi = hi_lo.checked_add(lo_hi)?;
    Some((out_hi, lo_lo))
}

fn cmp_u256(ahi: u128, alo: u128, bhi: u128, blo: u128) -> std::cmp::Ordering {
    ahi.cmp(&bhi).then(alo.cmp(&blo))
}

fn isqrt_u256_floor(hi: u128, lo: u128) -> u128 {
    if hi == 0 {
        return lo.isqrt();
    }
    let mut out = 0u128;
    let mut bit = 1u128 << 127;
    while bit != 0 {
        let candidate = out | bit;
        let (sq_hi, sq_lo) = mul_wide_u128(candidate, candidate);
        if cmp_u256(sq_hi, sq_lo, hi, lo).is_le() {
            out = candidate;
        }
        bit >>= 1;
    }
    out
}

fn cast_uint_result1(sample: &Value, out: u128, op: &str, x: &Value) -> VmrtRes<Value> {
    cast_uint_like_sample(sample, out, || ItrErr::new(Arithmetic, &check_failed_tip1(op, x)))
}

fn cast_uint_result2(sample: &Value, out: u128, op: &str, x: &Value, y: &Value) -> VmrtRes<Value> {
    cast_uint_like_sample(sample, out, || ItrErr::new(Arithmetic, &check_failed_tip(op, x, y)))
}

fn cast_uint_result3(
    sample: &Value,
    out: u128,
    op: &str,
    x: &Value,
    y: &Value,
    z: &Value,
) -> VmrtRes<Value> {
    cast_uint_like_sample(sample, out, || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z)))
}

fn cast_uint_result4(
    sample: &Value,
    out: u128,
    op: &str,
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
) -> VmrtRes<Value> {
    cast_uint_like_sample(sample, out, || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w)))
}

fn round_half_up_div_u256_by_u128(
    hi: u128,
    lo: u128,
    d: u128,
    op: &str,
    x: &Value,
    y: &Value,
    z: &Value,
) -> VmrtRes<u128> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    let (mut quo, rem) = div_u256_by_u128_to_u128(hi, lo, d).ok_or_else(err)?;
    let threshold = half_up_round_u128_threshold(d);
    if rem >= threshold {
        quo = quo.checked_add(1).ok_or_else(err)?;
    }
    Ok(quo)
}

fn round_half_even_quot_u128(
    mut quo: u128,
    rem: u128,
    d: u128,
    err: impl Fn() -> ItrErr,
) -> VmrtRes<u128> {
    if rem == 0 {
        return Ok(quo);
    }
    let cmp_half = if rem > u128::MAX / 2 {
        std::cmp::Ordering::Greater
    } else {
        rem.checked_mul(2).ok_or_else(&err)?.cmp(&d)
    };
    if cmp_half.is_gt() || (cmp_half.is_eq() && quo & 1 == 1) {
        quo = quo.checked_add(1).ok_or_else(err)?;
    }
    Ok(quo)
}

fn round_quot_u128_with_policy(
    mut quo: u128,
    rem: u128,
    d: u128,
    round: FinRoundPolicy,
    err: impl Fn() -> ItrErr,
) -> VmrtRes<u128> {
    match round {
        FinRoundPolicy::Exact => {
            if rem != 0 {
                return Err(err());
            }
        }
        FinRoundPolicy::Floor => {}
        FinRoundPolicy::Ceil => {
            quo = ceil_quot_if_rem_u128(quo, rem, &err)?;
        }
        FinRoundPolicy::HalfUp => {
            if rem >= half_up_round_u128_threshold(d) {
                quo = quo.checked_add(1).ok_or_else(&err)?;
            }
        }
        FinRoundPolicy::HalfEven => {
            quo = round_half_even_quot_u128(quo, rem, d, &err)?;
        }
    }
    Ok(quo)
}

fn div_u256_by_u128_with_round(
    hi: u128,
    lo: u128,
    d: u128,
    round: FinRoundPolicy,
    err: impl Fn() -> ItrErr,
) -> VmrtRes<u128> {
    let (quo, rem) = div_u256_by_u128_to_u128(hi, lo, d).ok_or_else(&err)?;
    round_quot_u128_with_policy(quo, rem, d, round, err)
}

fn div_u256_by_u129_with_round(
    hi: u128,
    lo: u128,
    d_hi: u128,
    d_lo: u128,
    round: FinRoundPolicy,
    half_even_parity_offset: u128,
    err: impl Fn() -> ItrErr,
) -> VmrtRes<u128> {
    let (mut quo, rem_hi, rem_lo) = div_u256_by_u129_to_u128(hi, lo, d_hi, d_lo).ok_or_else(&err)?;
    let has_rem = rem_hi != 0 || rem_lo != 0;
    match round {
        FinRoundPolicy::Exact => {
            if has_rem {
                return Err(err());
            }
        }
        FinRoundPolicy::Floor => {}
        FinRoundPolicy::Ceil => {
            if has_rem {
                quo = quo.checked_add(1).ok_or_else(&err)?;
            }
        }
        FinRoundPolicy::HalfUp => {
            let (dbl_hi, dbl_lo) = add_u256(rem_hi, rem_lo, rem_hi, rem_lo).ok_or_else(&err)?;
            if cmp_u256(dbl_hi, dbl_lo, d_hi, d_lo).is_ge() {
                quo = quo.checked_add(1).ok_or_else(&err)?;
            }
        }
        FinRoundPolicy::HalfEven => {
            let (dbl_hi, dbl_lo) = add_u256(rem_hi, rem_lo, rem_hi, rem_lo).ok_or_else(&err)?;
            let cmp_half = cmp_u256(dbl_hi, dbl_lo, d_hi, d_lo);
            let final_quo_is_odd = (quo & 1) != (half_even_parity_offset & 1);
            if cmp_half.is_gt() || (cmp_half.is_eq() && final_quo_is_odd) {
                quo = quo.checked_add(1).ok_or_else(&err)?;
            }
        }
    }
    Ok(quo)
}

fn mul_div_half_up(
    lhs: u128,
    rhs: u128,
    d: u128,
    op: &str,
    x: &Value,
    y: &Value,
    z: &Value,
) -> VmrtRes<u128> {
    let (hi, lo) = mul_wide_u128(lhs, rhs);
    round_half_up_div_u256_by_u128(hi, lo, d, op, x, y, z)
}

fn scaled_abs_diff_div_u128(x: u128, reference: u128, scale: u128) -> Option<(u128, u128)> {
    if reference == 0 || scale == 0 {
        return None;
    }
    let diff = x.abs_diff(reference);
    let (hi, lo) = mul_wide_u128(diff, scale);
    div_u256_by_u128_to_u128(hi, lo, reference)
}

fn saturating_uint_add(x: &Value, y: &Value) -> VmrtRes<Value> {
    Ok(match (x, y) {
        (U8(l), U8(r)) => Value::U8(l.saturating_add(*r)),
        (U16(l), U16(r)) => Value::U16(l.saturating_add(*r)),
        (U32(l), U32(r)) => Value::U32(l.saturating_add(*r)),
        (U64(l), U64(r)) => Value::U64(l.saturating_add(*r)),
        (U128(l), U128(r)) => Value::U128(l.saturating_add(*r)),
        (_, _) => {
            return itr_err_fmt!(
                Arithmetic,
                "cannot do arithmetic between {:?} and {:?}",
                x,
                y
            )
        }
    })
}

fn saturating_uint_sub(x: &Value, y: &Value) -> VmrtRes<Value> {
    Ok(match (x, y) {
        (U8(l), U8(r)) => Value::U8(l.saturating_sub(*r)),
        (U16(l), U16(r)) => Value::U16(l.saturating_sub(*r)),
        (U32(l), U32(r)) => Value::U32(l.saturating_sub(*r)),
        (U64(l), U64(r)) => Value::U64(l.saturating_sub(*r)),
        (U128(l), U128(r)) => Value::U128(l.saturating_sub(*r)),
        (_, _) => {
            return itr_err_fmt!(
                Arithmetic,
                "cannot do arithmetic between {:?} and {:?}",
                x,
                y
            )
        }
    })
}

fn absdiff_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    Ok(match (x, y) {
        (U8(l), U8(r)) => Value::U8(l.abs_diff(*r)),
        (U16(l), U16(r)) => Value::U16(l.abs_diff(*r)),
        (U32(l), U32(r)) => Value::U32(l.abs_diff(*r)),
        (U64(l), U64(r)) => Value::U64(l.abs_diff(*r)),
        (U128(l), U128(r)) => Value::U128(l.abs_diff(*r)),
        (_, _) => {
            return itr_err_fmt!(
                Arithmetic,
                "cannot do arithmetic between {:?} and {:?}",
                x,
                y
            )
        }
    })
}

fn sqrt_floor_checked(x: &Value) -> VmrtRes<Value> {
    let n = x.extract_u128()?;
    let out = n.isqrt();
    cast_uint_result1(x, out, "sqrt", x)
}

fn sqrt_up_checked(x: &Value) -> VmrtRes<Value> {
    let n = x.extract_u128()?;
    let err = || ItrErr::new(Arithmetic, &check_failed_tip1("sqrt_up", x));
    let f = n.isqrt();
    let out = if n <= 1 {
        n
    } else if f.checked_mul(f) == Some(n) {
        f
    } else {
        f.checked_add(1).ok_or_else(err)?
    };
    cast_uint_result1(x, out, "sqrt_up", x)
}

fn sqrtmul_with_round_checked(
    x: &Value,
    y: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip(op, x, y));
    let (hi, lo) = mul_wide_u128(x.extract_u128()?, y.extract_u128()?);
    let floor = isqrt_u256_floor(hi, lo);
    let out = match round {
        FinRoundPolicy::Floor => floor,
        FinRoundPolicy::Ceil => {
            let (sq_hi, sq_lo) = mul_wide_u128(floor, floor);
            if cmp_u256(sq_hi, sq_lo, hi, lo).is_eq() {
                floor
            } else {
                floor.checked_add(1).ok_or_else(err)?
            }
        }
        _ => return Err(err()),
    };
    cast_uint_result2(x, out, op, x, y)
}

fn quantize_with_round_checked(
    x: &Value,
    y: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip(op, x, y));
    let value = x.extract_u128()?;
    let step = require_nonzero_u128(y.extract_u128()?, err)?;
    let quo = value / step;
    let rem = value % step;
    let out = match round {
        FinRoundPolicy::Floor => quo.checked_mul(step).ok_or_else(err)?,
        FinRoundPolicy::Ceil if rem == 0 => value,
        FinRoundPolicy::Ceil => quo
            .checked_add(1)
            .and_then(|q| q.checked_mul(step))
            .ok_or_else(err)?,
        _ => return Err(err()),
    };
    cast_uint_result2(x, out, op, x, y)
}

fn addmod_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3("add_mod", x, y, z));
    let a = x.extract_u128()?;
    let b = y.extract_u128()?;
    let modu = require_nonzero_u128(z.extract_u128()?, err)?;
    let out = add_mod_u128(a, b, modu);
    cast_uint_result3(x, out, "add_mod", x, y, z)
}

fn mulmod_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3("mul_mod", x, y, z));
    let modu = require_nonzero_u128(z.extract_u128()?, err)?;
    let out = mul_mod_u128(x.extract_u128()?, y.extract_u128()?, modu);
    cast_uint_result3(x, out, "mul_mod", x, y, z)
}

fn muldiv_checked(x: &Value, y: &Value, z: &Value, op: &'static str) -> VmrtRes<Value> {
    muldiv_with_round_checked(x, y, z, FinRoundPolicy::Floor, op)
}

fn muldiv_up_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    muldiv_with_round_checked(x, y, z, FinRoundPolicy::Ceil, "mul_div_up")
}

fn muldiv_with_round_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    let div = require_nonzero_u128(z.extract_u128()?, err)?;
    let (hi, lo) = mul_wide_u128(x.extract_u128()?, y.extract_u128()?);
    let quo = div_u256_by_u128_with_round(hi, lo, div, round, err)?;
    cast_uint_result3(x, quo, op, x, y, z)
}

fn scaled_addsub_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    add_delta: bool,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    if !matches!(round, FinRoundPolicy::Floor | FinRoundPolicy::Ceil) {
        return Err(err());
    }
    let value = x.extract_u128()?;
    let rate = y.extract_u128()?;
    let scale = require_nonzero_u128(z.extract_u128()?, err)?;
    let (hi, lo) = mul_wide_u128(value, rate);
    let delta = div_u256_by_u128_with_round(hi, lo, scale, round, err)?;
    let out = if add_delta {
        value.checked_add(delta)
    } else {
        value.checked_sub(delta)
    }
    .ok_or_else(err)?;
    cast_uint_result3(x, out, op, x, y, z)
}

fn muldiv_den_addsub_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    add_to_den: bool,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    if !matches!(round, FinRoundPolicy::Floor | FinRoundPolicy::Ceil) {
        return Err(err());
    }
    let lhs = x.extract_u128()?;
    let rhs = y.extract_u128()?;
    let den_base = z.extract_u128()?;
    let (hi, lo) = mul_wide_u128(lhs, rhs);
    let quo = if add_to_den {
        let (den_hi, den_lo) = add_u256_u128(0, den_base, rhs).ok_or_else(err)?;
        div_u256_by_u129_with_round(hi, lo, den_hi, den_lo, round, 0, err)?
    } else {
        let den = den_base.checked_sub(rhs).ok_or_else(err)?;
        let den = require_nonzero_u128(den, err)?;
        div_u256_by_u128_with_round(hi, lo, den, round, err)?
    };
    cast_uint_result3(x, quo, op, x, y, z)
}

fn muladd_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3("mul_add", x, y, z));
    let lo = mul_xy_addsub_z_fit_u128(
        x.extract_u128()?,
        y.extract_u128()?,
        z.extract_u128()?,
        true,
        err,
    )?;
    cast_uint_result3(x, lo, "mul_add", x, y, z)
}

fn mul_shr_impl(x: &Value, y: &Value, z: &Value, op: &'static str, ceil_dropped: bool) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    let shift = z.extract_u128()?;
    let (hi, lo) = mul_wide_u128(x.extract_u128()?, y.extract_u128()?);
    let out = if shift >= 256 {
        if ceil_dropped && (hi != 0 || lo != 0) { 1 } else { 0 }
    } else {
        let (mut out, dropped) = shr_u256_to_u128(hi, lo, shift as u32).ok_or_else(err)?;
        if ceil_dropped && dropped {
            out = out.checked_add(1).ok_or_else(err)?;
        }
        out
    };
    cast_uint_result3(x, out, op, x, y, z)
}

fn rpow_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let op = "rpow_half_up";
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    let mut n = y.extract_u128()?;
    let base = z.extract_u128()?;
    if base == 0 {
        return Err(err());
    }
    if n == 0 {
        return cast_uint_result3(x, base, op, x, y, z);
    }
    let mut bas = x.extract_u128()?;
    let mut out = if n & 1 == 1 { bas } else { base };
    while n > 1 {
        n >>= 1;
        bas = mul_div_half_up(bas, bas, base, op, x, y, z)?;
        if n & 1 == 1 {
            out = mul_div_half_up(out, bas, base, op, x, y, z)?;
        }
    }
    cast_uint_result3(x, out, op, x, y, z)
}

fn clamp_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3("clamp", x, y, z));
    let xv = x.extract_u128()?;
    let lo = y.extract_u128()?;
    let hi = z.extract_u128()?;
    if lo > hi {
        return Err(err());
    }
    let out = xv.clamp(lo, hi);
    cast_uint_result3(x, out, "clamp", x, y, z)
}

fn satadd_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    saturating_uint_add(x, y)
}

fn satsub_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    saturating_uint_sub(x, y)
}

fn div_with_round_checked(
    x: &Value,
    y: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip(op, x, y));
    let div = require_nonzero_u128(y.extract_u128()?, err)?;
    let num = x.extract_u128()?;
    let quo = round_quot_u128_with_policy(num / div, num % div, div, round, err)?;
    cast_uint_result2(x, quo, op, x, y)
}

fn mulsub_checked(x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3("mul_sub", x, y, z));
    let lo = mul_xy_addsub_z_fit_u128(
        x.extract_u128()?,
        y.extract_u128()?,
        z.extract_u128()?,
        false,
        err,
    )?;
    cast_uint_result3(x, lo, "mul_sub", x, y, z)
}

fn muladddiv_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w));
    let div = require_nonzero_u128(w.extract_u128()?, err)?;
    let quo = mul_xy_addsub_z_div_u128(
        x.extract_u128()?,
        y.extract_u128()?,
        z.extract_u128()?,
        div,
        true,
        round,
        err,
    )?;
    cast_uint_result4(x, quo, op, x, y, z, w)
}

fn mulsubdiv_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w));
    let div = require_nonzero_u128(w.extract_u128()?, err)?;
    let quo = mul_xy_addsub_z_div_u128(
        x.extract_u128()?,
        y.extract_u128()?,
        z.extract_u128()?,
        div,
        false,
        round,
        err,
    )?;
    cast_uint_result4(x, quo, op, x, y, z, w)
}

fn mul3div_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w));
    let div = require_nonzero_u128(w.extract_u128()?, err)?;
    let (hi, lo) = mul_wide_u128(x.extract_u128()?, y.extract_u128()?);
    let (hi, lo) = mul_u256_u128_to_u256_checked(hi, lo, z.extract_u128()?).ok_or_else(err)?;
    let quo = div_u256_by_u128_with_round(hi, lo, div, round, err)?;
    cast_uint_result4(x, quo, op, x, y, z, w)
}

fn devscaled_with_round_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip3(op, x, y, z));
    let reference = y.extract_u128()?;
    let scale = z.extract_u128()?;
    let (quo, rem) = scaled_abs_diff_div_u128(x.extract_u128()?, reference, scale)
        .ok_or_else(&err)?;
    let out = round_quot_u128_with_policy(quo, rem, reference, round, err)?;
    cast_uint_result3(x, out, op, x, y, z)
}

fn withinbps_checked(x: &Value, y: &Value, z: &Value, w: &Value) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4("within_bps", x, y, z, w));
    let value = x.extract_u128()?;
    let reference = y.extract_u128()?;
    let tolerance = z.extract_u128()?;
    let scale = w.extract_u128()?;
    if reference == 0 || scale == 0 {
        return Err(err());
    }
    if tolerance > scale {
        return Err(err());
    }
    let diff = value.abs_diff(reference);
    let (lhs_hi, lhs_lo) = mul_wide_u128(diff, scale);
    let (rhs_hi, rhs_lo) = mul_wide_u128(reference, tolerance);
    Ok(Value::bool(cmp_u256(lhs_hi, lhs_lo, rhs_hi, rhs_lo).is_le()))
}

fn crossmul_pred_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
    kernel: FinKernel,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w));
    let (lhs_hi, lhs_lo) = mul_wide_u128(x.extract_u128()?, y.extract_u128()?);
    let (rhs_hi, rhs_lo) = mul_wide_u128(z.extract_u128()?, w.extract_u128()?);
    let ord = cmp_u256(lhs_hi, lhs_lo, rhs_hi, rhs_lo);
    let out = match kernel {
        FinKernel::CrossLte => ord.is_le(),
        FinKernel::CrossGte => ord.is_ge(),
        FinKernel::CrossEq => ord.is_eq(),
        _ => return Err(err()),
    };
    Ok(Value::bool(out))
}

fn wavg2_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w));
    let lhs = x.extract_u128()?;
    let rhs = z.extract_u128()?;
    let wx = y.extract_u128()?;
    let wy = w.extract_u128()?;
    let (den_hi, den_lo) = add_u256_u128(0, wx, wy).ok_or_else(err)?;
    if den_hi == 0 && den_lo == 0 {
        return Err(err());
    }
    if lhs == rhs {
        return cast_uint_result4(x, lhs, op, x, y, z, w);
    }
    let (base, diff, diff_weight) = if lhs < rhs {
        (lhs, rhs - lhs, wy)
    } else {
        (rhs, lhs - rhs, wx)
    };
    let (part_hi, part_lo) = mul_wide_u128(diff, diff_weight);
    let part = div_u256_by_u129_with_round(part_hi, part_lo, den_hi, den_lo, round, base, err)?;
    let out = base.checked_add(part).ok_or_else(err)?;
    cast_uint_result4(x, out, op, x, y, z, w)
}

fn lerp_checked(
    x: &Value,
    y: &Value,
    z: &Value,
    w: &Value,
    round: FinRoundPolicy,
    op: &'static str,
) -> VmrtRes<Value> {
    let err = || ItrErr::new(Arithmetic, &check_failed_tip4(op, x, y, z, w));
    let start = x.extract_u128()?;
    let end = y.extract_u128()?;
    let weight = z.extract_u128()?;
    let base = w.extract_u128()?;
    if base == 0 || weight > base {
        return Err(err());
    }
    let left_weight = base - weight;
    let (lhs_hi, lhs_lo) = mul_wide_u128(start, left_weight);
    let (rhs_hi, rhs_lo) = mul_wide_u128(end, weight);
    let (num_hi, num_lo) = add_u256(lhs_hi, lhs_lo, rhs_hi, rhs_lo).ok_or_else(err)?;
    let quo = div_u256_by_u128_with_round(num_hi, num_lo, base, round, err)?;
    cast_uint_result4(x, quo, op, x, y, z, w)
}

fn invalid_fin_spec(spec: FinSpec) -> VmrtRes<Value> {
    itr_err_fmt!(
        InstParamsErr,
        "invalid fin spec {} ({:?}, round {:?})",
        spec.name,
        spec.kernel,
        spec.round
    )
}

fn fin2_checked(spec: FinSpec, x: &Value, y: &Value) -> VmrtRes<Value> {
    let round = spec.round_or_exact();
    match spec.kernel {
        FinKernel::Div => div_with_round_checked(x, y, round, spec.name),
        FinKernel::SqrtMul => sqrtmul_with_round_checked(x, y, round, spec.name),
        FinKernel::Quantize => quantize_with_round_checked(x, y, round, spec.name),
        FinKernel::SatAdd => satadd_checked(x, y),
        FinKernel::SatSub => satsub_checked(x, y),
        _ => invalid_fin_spec(spec),
    }
}

fn fin3_checked(spec: FinSpec, x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    let round = spec.round_or_exact();
    match spec.kernel {
        FinKernel::MulDiv | FinKernel::ScaledDiv => {
            muldiv_with_round_checked(x, y, z, round, spec.name)
        }
        FinKernel::DevScaled => devscaled_with_round_checked(x, y, z, round, spec.name),
        FinKernel::ScaledAdd => scaled_addsub_checked(x, y, z, true, round, spec.name),
        FinKernel::ScaledSub => scaled_addsub_checked(x, y, z, false, round, spec.name),
        FinKernel::MulShr if round == FinRoundPolicy::Floor => {
            mul_shr_impl(x, y, z, spec.name, false)
        }
        FinKernel::MulShr if round == FinRoundPolicy::Ceil => {
            mul_shr_impl(x, y, z, spec.name, true)
        }
        FinKernel::MulDivDenAdd => {
            muldiv_den_addsub_checked(x, y, z, true, round, spec.name)
        }
        FinKernel::MulDivDenSub => {
            muldiv_den_addsub_checked(x, y, z, false, round, spec.name)
        }
        _ => invalid_fin_spec(spec),
    }
}

fn fin4_checked(spec: FinSpec, x: &Value, y: &Value, z: &Value, w: &Value) -> VmrtRes<Value> {
    let round = spec.round_or_exact();
    match spec.kernel {
        FinKernel::MulAddDiv => muladddiv_checked(x, y, z, w, round, spec.name),
        FinKernel::MulSubDiv => mulsubdiv_checked(x, y, z, w, round, spec.name),
        FinKernel::Mul3Div => mul3div_checked(x, y, z, w, round, spec.name),
        FinKernel::Wavg2 => wavg2_checked(x, y, z, w, round, spec.name),
        FinKernel::Lerp => lerp_checked(x, y, z, w, round, spec.name),
        _ => invalid_fin_spec(spec),
    }
}

fn finp3_checked(spec: FinSpec, x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    match spec.kernel {
        FinKernel::AbsDiffLte => {
            let xv = x.extract_u128()?;
            let yv = y.extract_u128()?;
            let tol = z.extract_u128()?;
            Ok(Value::bool(xv.abs_diff(yv) <= tol))
        }
        _ => invalid_fin_spec(spec),
    }
}

fn finp4_checked(spec: FinSpec, x: &Value, y: &Value, z: &Value, w: &Value) -> VmrtRes<Value> {
    match spec.kernel {
        FinKernel::WithinBps => withinbps_checked(x, y, z, w),
        FinKernel::CrossLte | FinKernel::CrossGte | FinKernel::CrossEq => {
            crossmul_pred_checked(x, y, z, w, spec.kernel, spec.name)
        }
        _ => invalid_fin_spec(spec),
    }
}

fn finpow3_checked(spec: FinSpec, x: &Value, y: &Value, z: &Value) -> VmrtRes<Value> {
    match spec.kernel {
        FinKernel::RPow => rpow_checked(x, y, z),
        _ => invalid_fin_spec(spec),
    }
}

// the value is must within u32
fn pow_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    let exp_u32 = |n: u128| -> VmrtRes<u32> {
        u32::try_from(n).map_err(|_| ItrErr::new(Arithmetic, &check_failed_tip("pow", x, y)))
    };
    match (x, y) {
        (U8(l), U8(r)) => <u8>::checked_pow(*l, *r as u32).map(Value::U8),
        (U16(l), U16(r)) => <u16>::checked_pow(*l, *r as u32).map(Value::U16),
        (U32(l), U32(r)) => <u32>::checked_pow(*l, *r).map(Value::U32),
        (U64(l), U64(r)) => <u64>::checked_pow(*l, exp_u32(*r as u128)?).map(Value::U64),
        (U128(l), U128(r)) => <u128>::checked_pow(*l, exp_u32(*r)?).map(Value::U128),
        (_, _) => {
            return itr_err_fmt!(
                Arithmetic,
                "cannot do pow arithmetic between {:?} and {:?}",
                x,
                y
            );
        }
    }
    .ok_or_else(|| ItrErr::new(Arithmetic, &check_failed_tip("pow", x, y)))
}

fn max_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    let a = x.extract_u128()?;
    let b = y.extract_u128()?;
    Ok(maybe!(a > b, x.clone(), y.clone()))
}

fn min_checked(x: &Value, y: &Value) -> VmrtRes<Value> {
    let a = x.extract_u128()?;
    let b = y.extract_u128()?;
    Ok(maybe!(a < b, x.clone(), y.clone()))
}

fn unary_inc(x: &mut Value, n: u8) -> VmrtErr {
    if !x.is_uint() {
        return itr_err_fmt!(Arithmetic, "cannot do arithmetic with {:?}", x);
    }
    x.inc(n)
        .map_err(|ItrErr(_, msg)| ItrErr::new(Arithmetic, &msg))
}

fn unary_dec(x: &mut Value, n: u8) -> VmrtErr {
    if !x.is_uint() {
        return itr_err_fmt!(Arithmetic, "cannot do arithmetic with {:?}", x);
    }
    x.dec(n)
        .map_err(|ItrErr(_, msg)| ItrErr::new(Arithmetic, &msg))
}

