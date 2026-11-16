macro_rules! inc_dec_opt_define {
    ($self:ident, $ty:ident, $ty2:ident, $f:ident, $n:expr) => {
        if let $ty(v) = $self {
            *$self = match v.$f($n as $ty2).map(Self::$ty) {
                Some(v) => v,
                _ => {
                    return itr_err_fmt!(CastFail, "{} failed for {} and {}", stringify!($f), v, $n);
                }
            };
            return Ok(());
        }
    };
}

impl Value {
    // must uint
    #[inline(always)]
    pub fn inc(&mut self, n: u8) -> VmrtErr {
        inc_dec_opt_define! {self,   U8,   u8, checked_add, n}
        inc_dec_opt_define! {self,  U16,  u16, checked_add, n}
        inc_dec_opt_define! {self,  U32,  u32, checked_add, n}
        inc_dec_opt_define! {self,  U64,  u64, checked_add, n}
        inc_dec_opt_define! {self, U128, u128, checked_add, n}
        itr_err_fmt!(CastFail, "inst inc cannot cast {:?} to uint", self)
    }

    // must uint
    #[inline(always)]
    pub fn dec(&mut self, n: u8) -> VmrtErr {
        inc_dec_opt_define! {self,   U8,   u8, checked_sub, n}
        inc_dec_opt_define! {self,  U16,  u16, checked_sub, n}
        inc_dec_opt_define! {self,  U32,  u32, checked_sub, n}
        inc_dec_opt_define! {self,  U64,  u64, checked_sub, n}
        inc_dec_opt_define! {self, U128, u128, checked_sub, n}
        itr_err_fmt!(CastFail, "inst dec cannot cast {:?} to uint", self)
    }

    // ret u8
    #[inline(always)]
    pub fn cutbyte(&mut self, n: u16) -> VmrtErr {
        let buf = self.extract_bytes_with_error_code(BytesHandle)?;
        let idx = n as usize;
        if idx >= buf.len() {
            return itr_err_fmt!(StackError, "read buf byte overflow");
        }
        *self = Self::U8(buf[idx]);
        Ok(())
    }

    #[inline(always)]
    pub fn cutleft(&mut self, n: u16) -> VmrtErr {
        let buf = self.extract_bytes_with_error_code(BytesHandle)?;
        let spx = n as usize;
        if spx > buf.len() {
            return itr_err_fmt!(StackError, "cut buf left overflow");
        }
        *self = Self::Bytes(buf[..spx].to_vec());
        Ok(())
    }

    #[inline(always)]
    pub fn cutright(&mut self, n: u16) -> VmrtErr {
        let buf = self.extract_bytes_with_error_code(BytesHandle)?;
        let spx = buf.len().checked_sub(n as usize).ok_or_else(|| ItrErr::new(StackError, "cut buf right overflow"))?;
        *self = Self::Bytes(buf[spx as usize..].to_vec());
        Ok(())
    }

    #[inline(always)]
    pub fn cutout(&mut self, len: Value, ost: Value) -> VmrtErr {
        let len = len.extract_u16()? as usize;
        let ost = ost.extract_u16()? as usize;
        let val = self.extract_bytes_with_error_code(BytesHandle)?;
        let end = len.checked_add(ost).ok_or_else(|| ItrErr::new(StackError, "cutout buf index overflow"))?;
        if end > val.len() {
            return itr_err_fmt!(StackError, "cutout buf overflow");
        }
        *self = Self::Bytes(val[ost..end].to_vec());
        Ok(())
    }

    #[inline(always)]
    pub fn dropleft(&mut self, n: u16) -> VmrtErr {
        let buf = self.extract_bytes_with_error_code(BytesHandle)?;
        let spx = n as usize;
        if spx > buf.len() {
            return itr_err_fmt!(StackError, "drop buf left overflow");
        }
        *self = Self::Bytes(buf[spx..].to_vec());
        Ok(())
    }

    pub fn dropright(&mut self, n: u16) -> VmrtErr {
        let buf = self.extract_bytes_with_error_code(BytesHandle)?;
        let spx = buf.len().checked_sub(n as usize).ok_or_else(|| ItrErr::new(StackError, "drop buf right overflow"))?;
        *self = Self::Bytes(buf[0..spx as usize].to_vec());
        Ok(())
    }

    /// BUFSET: in-place bytes write. The receiver is `self`; on the operand
    /// stack the source sits on top and the start below it, so the checks run
    /// source first, then start, then receiver: source bytes (`BytesHandle`,
    /// `extract_bytes_with_error_code` rejects Nil), start `extract_u16`
    /// (`CastParamFail`), receiver bytes (`BytesHandle`). A `Bytes` receiver is
    /// mutated in place without cloning its `Vec`; every other receiver goes
    /// through `extract_bytes_with_error_code` and is converted to
    /// `Value::Bytes` exactly as before (Nil/Compo/Tuple/Handle -> 91;
    /// Bool/uints/Address -> canonical bytes). The boundary is `cutout`'s rule:
    /// `start + src.len()` via `checked_add`, overflow or `> buf.len()` ->
    /// `StackError`, `== buf.len()` legal (write ending at the end). The length
    /// never changes, and every other stack slot keeps its old copy.
    #[inline(always)]
    pub fn bufset(&mut self, src: Value, ost: Value) -> VmrtErr {
        let src = src.extract_bytes_with_error_code(BytesHandle)?;
        let ost = ost.extract_u16()? as usize;
        let end_of = |buf_len: usize| -> VmrtRes<usize> {
            let end = ost.checked_add(src.len()).ok_or_else(|| {
                ItrErr::new(StackError, "bufset buf index overflow")
            })?;
            if end > buf_len {
                return itr_err_fmt!(StackError, "bufset buf overflow");
            }
            Ok(end)
        };
        match self {
            Self::Bytes(buf) => {
                let end = end_of(buf.len())?;
                buf[ost..end].copy_from_slice(&src);
            }
            _ => {
                // Non-Bytes receivers normalize through the canonical scalar
                // extract (Nil/Compo/Tuple/Handle -> 91; Bool/uints/Address ->
                // bytes), then become `Value::Bytes` as before.
                let mut buf = self.extract_bytes_with_error_code(BytesHandle)?;
                let end = end_of(buf.len())?;
                buf[ost..end].copy_from_slice(&src);
                *self = Self::Bytes(buf);
            }
        }
        Ok(())
    }

    /* return buf: a + b */
    #[inline(always)]
    pub fn concat(a: &Value, b: &Value, cap: &SpaceCap) -> VmrtRes<Value> {
        let e = BytesHandle;
        let av = a.extract_bytes_with_error_code(e)?;
        let bv = b.extract_bytes_with_error_code(e)?;
        let total = checked_value_output_add(cap, av.len(), bv.len())?;
        let mut v = Vec::with_capacity(total);
        v.extend_from_slice(&av);
        v.extend_from_slice(&bv);
        Ok(Value::bytes(v).valid(cap)?)
    }
}


fn bit_shift_overflow(op: &str, x: &Value, y: &Value) -> ItrErr {
    ItrErr::new(
        Arithmetic,
        &format!("bit {} shift overflow between {:?} and {:?}", op, x, y),
    )
}

/// Checked left shift on matched-width uint values. Lives in the codec-side
/// `value` module (not `interpreter`) because the fitsh front-end folds
/// constant shifts at parse time — the `syntax` feature must not depend on
/// the execution engine.
pub(crate) fn bit_shl(x: &Value, y: &Value) -> VmrtRes<Value> {
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

pub(crate) fn bit_shr(x: &Value, y: &Value) -> VmrtRes<Value> {
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

#[cfg(test)]
mod bufset_tests {
    use super::*;

    /// BUFSET mutates a `Value::Bytes` receiver in place: the Vec pointer does
    /// not move (no clone, no reallocation) and the bytes change. A non-Bytes
    /// receiver still normalizes to `Value::Bytes` on a valid write, and stays
    /// untouched when the write is rejected.
    #[test]
    fn bufset_mutates_bytes_in_place_and_normalizes_non_bytes() {
        let mut v = Value::Bytes(vec![1, 2, 3]);
        let ptr = match &v {
            Value::Bytes(buf) => buf.as_ptr(),
            _ => unreachable!(),
        };
        v.bufset(Value::U8(9), Value::U8(1)).unwrap();
        match &v {
            Value::Bytes(buf) => {
                assert_eq!(buf.as_ptr(), ptr, "in-place write must not move the Vec");
                assert_eq!(buf, &vec![1, 9, 3]);
            }
            other => panic!("expected Value::Bytes, got {other:?}"),
        }

        let mut v = Value::U16(0x0102);
        v.bufset(Value::U8(9), Value::U8(1)).unwrap();
        assert_eq!(v, Value::Bytes(vec![1, 9]));

        let mut v = Value::U16(0x0102);
        let err = v.bufset(Value::Nil, Value::U8(0)).unwrap_err();
        assert_eq!(err.0, crate::rt::ItrErrCode::BytesHandle);
        assert_eq!(v, Value::U16(0x0102), "failed write must not normalize");
    }
}
