/// Convert raw bytes to a uint `Value` of the **minimal** active width.
/// Leading zeros are trimmed; the effective length determines the target variant.
///
/// Map keys normalize via `uint_key_bytes` on the numeric value, so `U8(1)` and `U64(1)`
/// share one slot. See `vm/doc/value-cast.md` §9.1.
pub(crate) fn buf_to_uint(buf: &[u8]) -> VmrtRes<Value> {
    let raw = trim_leading_zero_bytes(buf);
    let sz = raw.len();
    if sz == 0 {
        return Ok(Value::U8(0));
    }
    let Some(width) = minimal_active_uint_bytes(sz) else {
        return itr_err_fmt!(CastFail, "cannot cast 0x{} to uint", hex::encode(buf));
    };
    match width {
        1 => Ok(Value::U8(raw[0])),
        2 => Ok(Value::U16(u16::from_be_bytes([raw[0], raw[1]]))),
        4 => {
            let mut out = [0u8; 4];
            out[4 - sz..].copy_from_slice(raw);
            Ok(Value::U32(u32::from_be_bytes(out)))
        }
        8 => {
            let mut out = [0u8; 8];
            out[8 - sz..].copy_from_slice(raw);
            Ok(Value::U64(u64::from_be_bytes(out)))
        }
        16 => {
            let mut out = [0u8; 16];
            out[16 - sz..].copy_from_slice(raw);
            Ok(Value::U128(u128::from_be_bytes(out)))
        }
        _ => itr_err_fmt!(CastFail, "cannot cast 0x{} to uint", hex::encode(buf)),
    }
}

fn buf_to_u128(buf: &[u8]) -> VmrtRes<u128> {
    const U128_BYTES: usize = std::mem::size_of::<u128>();
    let raw = trim_leading_zero_bytes(buf);
    let Some(out) = fit_be_bytes::<U128_BYTES>(raw) else {
        return itr_err_fmt!(CastFail, "cannot cast 0x{} to u128", hex::encode(buf));
    };
    Ok(u128::from_be_bytes(out))
}

#[inline(always)]
fn has_non_zero_byte(b: &[u8]) -> bool {
    b.iter().any(|x| *x != 0)
}

impl Value {
    fn uint_u128_opt(&self) -> Option<u128> {
        match self {
            U8(n) => Some(*n as u128),
            U16(n) => Some(*n as u128),
            U32(n) => Some(*n as u128),
            U64(n) => Some(*n as u128),
            U128(n) => Some(*n),
            _ => None,
        }
    }

    fn extract_uint_cast<T>(&self, ty: &str) -> VmrtRes<T>
    where
        T: TryFrom<u128>,
    {
        let Some(un) = self.uint_u128_opt() else {
            return itr_err_fmt!(CastParamFail, "cannot cast type {:?} to {}", self, ty);
        };
        T::try_from(un).map_err(|_| {
            ItrErr::new(
                CastParamFail,
                &format!("cannot cast param {:?} to {}", un, ty),
            )
        })
    }

    pub fn extract_u8(&self) -> VmrtRes<u8> {
        self.extract_uint_cast("u8")
    }

    pub fn extract_u16(&self) -> VmrtRes<u16> {
        self.extract_uint_cast("u16")
    }

    pub fn extract_u32(&self) -> VmrtRes<u32> {
        self.extract_uint_cast("u32")
    }

    pub fn extract_u64(&self) -> VmrtRes<u64> {
        self.extract_uint_cast("u64")
    }

    pub fn extract_u128(&self) -> VmrtRes<u128> {
        // Strict extractor for typed params: only uint variants are accepted.
        self.extract_uint_cast("u128")
    }

    /// Runtime truthiness used by control flow, logical ops, and explicit `as bool` casts.
    /// This is intentionally broader than canonical bool byte decoding used by
    /// `Value::type_from(ValueTy::Bool, ..)` and `Parse for Value`.
    pub fn extract_bool(&self) -> VmrtRes<bool> {
        if let Some(n) = self.uint_u128_opt() {
            return Ok(n != 0);
        }
        match self {
            Bool(b) => Ok(*b),
            Nil => Ok(false),
            Bytes(b) => Ok(has_non_zero_byte(b)),
            Address(a) => Ok(has_non_zero_byte(a.as_bytes())),
            _ => itr_err_fmt!(CastFail, "cannot cast {:?} to bool", self),
        }
    }

    pub(crate) fn to_uint(&self) -> VmrtRes<Value> {
        if self.ty().is_uint() {
            return Ok(self.clone());
        }
        itr_err_fmt!(CastFail, "cannot cast {:?} to uint", self)
    }

    fn to_u128(&self) -> VmrtRes<u128> {
        // Relaxed converter for explicit casts: bool/nil/bytes/address are also accepted.
        if let Some(n) = self.uint_u128_opt() {
            return Ok(n);
        }
        match self {
            Nil => Ok(0),
            Bool(b) => Ok(maybe!(*b, 1, 0)),
            Bytes(buf) => buf_to_u128(buf),
            Address(a) => buf_to_u128(a.as_bytes()),
            _ => itr_err_fmt!(CastFail, "cannot cast {:?} to u128", self),
        }
    }

    pub fn type_from(ty: ValueTy, stuff: Vec<u8>) -> VmrtRes<Self> {
        let vlen = stuff.len();
        macro_rules! cast_err {
            () => {
                itr_err_fmt!(
                    CastParamFail,
                    "cannot cast 0x{} to type id {:?}",
                    stuff.clone().to_hex(),
                    ty
                )
            };
        }
        macro_rules! ensure_len {
            ($l:expr) => {
                if vlen != $l {
                    return cast_err!();
                }
            };
        }
        macro_rules! uint {
            ($ty:ty, $n:expr) => {{
                let mut out = [0u8; $n];
                out.copy_from_slice(&stuff[..$n]);
                <$ty>::from_be_bytes(out)
            }};
        }
        match ty {
            ValueTy::Nil => {
                ensure_len!(0);
                Ok(Self::Nil)
            }
            ValueTy::Bool => {
                ensure_len!(1);
                let Some(b) = decode_canonical_bool_byte(stuff[0]) else {
                    return cast_err!();
                };
                Ok(Self::bool(b))
            }
            ValueTy::U8 => {
                ensure_len!(1);
                Ok(Self::u8(stuff[0]))
            }
            ValueTy::U16 => {
                ensure_len!(2);
                Ok(Self::U16(uint!(u16, 2)))
            }
            ValueTy::U32 => {
                ensure_len!(4);
                Ok(Self::U32(uint!(u32, 4)))
            }
            ValueTy::U64 => {
                ensure_len!(8);
                Ok(Self::U64(uint!(u64, 8)))
            }
            ValueTy::U128 => {
                ensure_len!(16);
                Ok(Self::U128(uint!(u128, 16)))
            }
            ValueTy::Bytes => Ok(Self::Bytes(stuff)),
            ValueTy::Address => {
                ensure_len!(field::Address::SIZE);
                let Ok(addr) = address_from_bytes(&stuff[0..field::Address::SIZE]) else {
                    return cast_err!();
                };
                Ok(Self::Address(addr))
            }
            _ => cast_err!(),
        }
    }

    pub fn extract_address(&self) -> VmrtRes<field::Address> {
        match self {
            Address(adr) => Ok(adr.clone()),
            Bytes(adr) => address_from_bytes(adr).map_ire(CastParamFail),
            _ => itr_err_fmt!(CastParamFail, "cannot cast {:?} to address", self),
        }
    }

    pub fn extract_contract_address(&self) -> VmrtRes<ContractAddress> {
        let addr = self.extract_address()?;
        ContractAddress::from_addr(addr).map_ire(ContractAddrErr)
    }

    pub fn extract_fnsign(&self) -> VmrtRes<FnSign> {
        match self {
            U32(u) => Ok(u.to_be_bytes()),
            Bytes(b) => checked_func_sign(&b),
            _ => itr_err_fmt!(CastParamFail, "cannot cast {:?} to fn sign", self),
        }
    }
}

