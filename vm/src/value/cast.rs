fn cannot_cast_err(v: &Value, ty: &str) -> ItrErr {
    ItrErr::new(CastFail, &format!("cannot cast {:?} to {}", v, ty))
}

fn cast_uint_name(bits: u16) -> &'static str {
    match bits {
        8 => "U8",
        16 => "U16",
        32 => "U32",
        64 => "U64",
        128 => "U128",
        _ => "UINT",
    }
}

fn ensure_active_uint_bits(bits: u16) -> VmrtErr {
    if ACTIVE_UINT_BITS.contains(&bits) {
        return Ok(());
    }
    itr_err_code!(CastFail)
}

fn bytes_width_err(buf: &[u8], bits: u16) -> ItrErr {
    ItrErr::new(
        CastFail,
        &format!(
            "cannot cast {:?} to {}",
            Value::Bytes(buf.to_vec()),
            cast_uint_name(bits)
        ),
    )
}

fn bytes_to_fixed_width<const N: usize>(buf: &[u8], bits: u16) -> VmrtRes<[u8; N]> {
    fit_be_bytes::<N>(buf).ok_or_else(|| bytes_width_err(buf, bits))
}

/// Convert raw bytes to a uint `Value` of a **fixed** target width. PITFALL: `buf_to_uint`
/// (convert.rs) picks the **minimal** width; map keys use `uint_key_bytes`, so equal uints share a slot.
fn bytes_to_uint_width(buf: &[u8], bits: u16) -> VmrtRes<Value> {
    ensure_active_uint_bits(bits)?;
    Ok(match bits {
        8 => Value::U8(u8::from_be_bytes(bytes_to_fixed_width::<1>(buf, bits)?)),
        16 => Value::U16(u16::from_be_bytes(bytes_to_fixed_width::<2>(buf, bits)?)),
        32 => Value::U32(u32::from_be_bytes(bytes_to_fixed_width::<4>(buf, bits)?)),
        64 => Value::U64(u64::from_be_bytes(bytes_to_fixed_width::<8>(buf, bits)?)),
        128 => Value::U128(u128::from_be_bytes(bytes_to_fixed_width::<16>(buf, bits)?)),
        _ => return itr_err_code!(CastFail),
    })
}

fn arith_uint_bits(v: &Value) -> Option<u16> {
    v.ty().uint_bits()
}

fn arithmetic_cast_err(values: &[&Value]) -> ItrErr {
    ItrErr::new(
        CastFail,
        &format!(
            "cannot do arithmetic cast between {}",
            values
                .iter()
                .map(|v| format!("{:?}", v))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

impl Value {
    pub(crate) fn cast_same_uint_width2(x: &mut Value, y: &mut Value) -> VmrtErr {
        let (Some(lb), Some(rb)) = (arith_uint_bits(x), arith_uint_bits(y)) else {
            return Err(arithmetic_cast_err(&[x, y]));
        };
        let tb = lb.max(rb);
        if lb < tb {
            x.cast_to_uint_width(tb)?;
        }
        if rb < tb {
            y.cast_to_uint_width(tb)?;
        }
        Ok(())
    }

    pub(crate) fn cast_same_uint_width3(x: &mut Value, y: &mut Value, z: &mut Value) -> VmrtErr {
        let (Some(xb), Some(yb), Some(zb)) =
            (arith_uint_bits(x), arith_uint_bits(y), arith_uint_bits(z))
        else {
            return Err(arithmetic_cast_err(&[x, y, z]));
        };
        let tb = xb.max(yb).max(zb);
        if xb < tb {
            x.cast_to_uint_width(tb)?;
        }
        if yb < tb {
            y.cast_to_uint_width(tb)?;
        }
        if zb < tb {
            z.cast_to_uint_width(tb)?;
        }
        Ok(())
    }

    pub(crate) fn cast_same_uint_width4(
        x: &mut Value,
        y: &mut Value,
        z: &mut Value,
        w: &mut Value,
    ) -> VmrtErr {
        let (Some(xb), Some(yb), Some(zb), Some(wb)) = (
            arith_uint_bits(x),
            arith_uint_bits(y),
            arith_uint_bits(z),
            arith_uint_bits(w),
        ) else {
            return Err(arithmetic_cast_err(&[x, y, z, w]));
        };
        let tb = xb.max(yb).max(zb).max(wb);
        if xb < tb {
            x.cast_to_uint_width(tb)?;
        }
        if yb < tb {
            y.cast_to_uint_width(tb)?;
        }
        if zb < tb {
            z.cast_to_uint_width(tb)?;
        }
        if wb < tb {
            w.cast_to_uint_width(tb)?;
        }
        Ok(())
    }

    pub(crate) fn arithmetic_args2(x: &Value, y: &Value) -> VmrtRes<(Value, Value)> {
        let mut lx = x.to_uint()?;
        let mut ry = y.to_uint()?;
        Self::cast_same_uint_width2(&mut lx, &mut ry)?;
        Ok((lx, ry))
    }

    pub(crate) fn arithmetic_args3(
        x: &Value,
        y: &Value,
        z: &Value,
    ) -> VmrtRes<(Value, Value, Value)> {
        let mut lx = x.to_uint()?;
        let mut my = y.to_uint()?;
        let mut rz = z.to_uint()?;
        Self::cast_same_uint_width3(&mut lx, &mut my, &mut rz)?;
        Ok((lx, my, rz))
    }

    pub(crate) fn arithmetic_args4(
        x: &Value,
        y: &Value,
        z: &Value,
        w: &Value,
    ) -> VmrtRes<(Value, Value, Value, Value)> {
        let mut lx = x.to_uint()?;
        let mut my = y.to_uint()?;
        let mut rz = z.to_uint()?;
        let mut qw = w.to_uint()?;
        Self::cast_same_uint_width4(&mut lx, &mut my, &mut rz, &mut qw)?;
        Ok((lx, my, rz, qw))
    }

    pub fn cast_bool(&mut self) -> VmrtErr {
        *self = Value::Bool(self.extract_bool()?);
        Ok(())
    }

    pub fn cast_bool_not(&mut self) -> VmrtErr {
        self.cast_bool()?;
        let Bool(b) = self else { never!() };
        *b = !*b;
        Ok(())
    }

    pub fn cast_to_uint_width(&mut self, bits: u16) -> VmrtErr {
        ensure_active_uint_bits(bits)?;
        let name = cast_uint_name(bits);
        if let Bytes(buf) = self {
            *self = bytes_to_uint_width(buf, bits)?;
            return Ok(());
        }
        let v = self.to_u128().map_err(|_| cannot_cast_err(self, name))?;
        *self = match bits {
            8 => Value::U8(u8::try_from(v).map_err(|_| cannot_cast_err(self, name))?),
            16 => Value::U16(u16::try_from(v).map_err(|_| cannot_cast_err(self, name))?),
            32 => Value::U32(u32::try_from(v).map_err(|_| cannot_cast_err(self, name))?),
            64 => Value::U64(u64::try_from(v).map_err(|_| cannot_cast_err(self, name))?),
            128 => Value::U128(v),
            _ => return itr_err_code!(CastFail),
        };
        Ok(())
    }

    pub fn cast_u8(&mut self) -> VmrtErr {
        self.cast_to_uint_width(8)
    }

    pub fn cast_u16(&mut self) -> VmrtErr {
        self.cast_to_uint_width(16)
    }

    pub fn cast_u32(&mut self) -> VmrtErr {
        self.cast_to_uint_width(32)
    }

    pub fn cast_u64(&mut self) -> VmrtErr {
        self.cast_to_uint_width(64)
    }

    pub fn cast_u128(&mut self) -> VmrtErr {
        self.cast_to_uint_width(128)
    }

    pub fn cast_bytes(&mut self) -> VmrtErr {
        if matches!(self, Bytes(..)) {
            return Ok(());
        }
        *self = Bytes(self.extract_bytes_with_error_code(CastFail)?);
        Ok(())
    }

    pub fn cast_addr(&mut self) -> VmrtErr {
        if matches!(self, Address(..)) {
            return Ok(());
        }
        self.cast_bytes()?;
        let Bytes(buf) = self else {
            never!()
        };
        let adr = address_from_bytes(buf).map_ire(CastFail)?;
        *self = Address(adr);
        Ok(())
    }

    fn cast_to_ty(&mut self, ty: ValueTy) -> VmrtErr {
        use ValueTy::*;
        match ty {
            Bool => self.cast_bool(),
            U8 => self.cast_u8(),
            U16 => self.cast_u16(),
            U32 => self.cast_u32(),
            U64 => self.cast_u64(),
            U128 => self.cast_u128(),
            Bytes => self.cast_bytes(),
            Address => self.cast_addr(),
            _ => itr_err_code!(CastFail),
        }
    }

    pub fn cast_to(&mut self, ty: u8) -> VmrtErr {
        let ty = ValueTy::build(ty).map_ire(CastFail)?;
        self.cast_to_ty(ty)
    }

    fn fn_boundary_type_fail(expect: ValueTy, actual: ValueTy) -> ItrErr {
        ItrErr::new(
            CallArgvTypeFail,
            &format!("expected {:?} but got {:?}", expect, actual),
        )
    }

    fn map_boundary_cast_error(expect: ValueTy, actual: ValueTy, err: ItrErr) -> ItrErr {
        let ItrErr(_, msg) = err;
        if msg.is_empty() {
            Self::fn_boundary_type_fail(expect, actual)
        } else {
            ItrErr::new(CallArgvTypeFail, &msg)
        }
    }

    pub fn cast_param(&mut self, ty: ValueTy) -> VmrtErr {
        let actual = self.ty();
        if ty == actual {
            return Ok(());
        }
        if ty.is_uint() && actual.is_uint() {
            return self
                .cast_to_ty(ty)
                .map_err(|err| Self::map_boundary_cast_error(ty, actual, err));
        }
        // Address and 21-byte Bytes share one value domain: implicit both ways at call
        // boundaries. Address->Bytes is lossless (`scalar_bytes`); Bytes->Address reuses
        // `cast_addr`'s `address_from_bytes` (length + `is_supported` version check), so a
        // bytes value that could never come from a real address still fails here with
        // CallArgvTypeFail instead of becoming an undecodable stored Address.
        if (ty == ValueTy::Address && actual == ValueTy::Bytes)
            || (ty == ValueTy::Bytes && actual == ValueTy::Address)
        {
            return self
                .cast_to_ty(ty)
                .map_err(|err| Self::map_boundary_cast_error(ty, actual, err));
        }
        Err(Self::fn_boundary_type_fail(ty, actual))
    }

    pub fn check_param_type(&self, ty: ValueTy) -> VmrtErr {
        let mut tmp = self.clone();
        tmp.cast_param(ty)
    }
}

#[cfg(test)]
mod cast_param_tests {
    use super::*;
    use field::Address;

    fn supported_addr(version: u8, tail: u8) -> Address {
        let mut raw = [0u8; Address::SIZE];
        raw[0] = version;
        raw[Address::SIZE - 1] = tail;
        Address::from(raw)
    }

    fn bytes21(a: Address) -> Value {
        Value::Bytes(a.as_bytes().to_vec())
    }

    // Boundary implicit conversion: Bytes(21) <-> Address both ways, other shapes fail.
    #[test]
    fn bytes21_to_address_implicit_at_boundary() {
        let a = supported_addr(Address::VERSION_CONTRACT, 9);
        let mut v = bytes21(a);
        v.cast_param(ValueTy::Address).unwrap();
        assert_eq!(v, Value::Address(a));

        // wrong length cannot implicitly convert
        let mut v = Value::Bytes(vec![0u8; 20]);
        assert!(v.check_param_type(ValueTy::Address).is_err());

        // unsupported version byte cannot implicitly convert: the value domain of
        // Address is validated addresses, not arbitrary 21-byte strings
        let mut raw = [0u8; Address::SIZE];
        raw[0] = 0x02;
        raw[20] = 1;
        let mut v = Value::Bytes(raw.to_vec());
        let err = v.check_param_type(ValueTy::Address).unwrap_err();
        assert_eq!(err.0, CallArgvTypeFail);
    }

    #[test]
    fn address_to_bytes_implicit_at_boundary() {
        let a = supported_addr(Address::VERSION_PRIVAKEY, 3);
        let mut v = Value::Address(a);
        v.cast_param(ValueTy::Bytes).unwrap();
        assert_eq!(v, bytes21(a));
    }

    #[test]
    fn address_bytes21_content_eq_and_error_paths() {
        let a = supported_addr(Address::VERSION_SCRIPTMH, 5);
        assert!(value_content_eq(&Value::Address(a), &bytes21(a)).unwrap());
        assert!(value_content_eq(&bytes21(a), &Value::Address(a)).unwrap());

        let other = supported_addr(Address::VERSION_SCRIPTMH, 6);
        assert!(!value_content_eq(&Value::Address(a), &bytes21(other)).unwrap());

        // non-21-byte Bytes stays a cross-type error, not a silent false
        assert!(value_content_eq(&Value::Bytes(vec![1, 2, 3]), &Value::Address(a)).is_err());

        // content comparison does not version-validate: an invalid-version 21-byte
        // string still compares by raw bytes
        let mut raw = [0u8; Address::SIZE];
        raw[0] = 0x02;
        raw[20] = 1;
        assert!(!value_content_eq(&Value::Bytes(raw.to_vec()), &Value::Address(a)).unwrap());
    }
}

