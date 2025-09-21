

fn buf_to_uint(buf: &[u8]) -> VmrtRes<Value> {
    let rlbts = buf_drop_left_zero(buf, 0);
    let sizen = rlbts.len();
    match sizen {
        1 => Ok(Value::U8(rlbts[0])),
        2 => {
            let v = u16::from_be_bytes(rlbts.try_into().unwrap());
            Ok(Value::U16(v))
        },
        3..=4 => {
            let bts = buf_fill_left_zero(buf, 4);
            let v = u32::from_be_bytes(bts.try_into().unwrap());
            Ok(Value::U32(v))
        },
        5..=8 => {
            let bts = buf_fill_left_zero(buf, 8);
            let v = u64::from_be_bytes(bts.try_into().unwrap());
            Ok(Value::U64(v))
        },
        9..=16 => {
            let bts = buf_fill_left_zero(buf, 16);
            let v = u128::from_be_bytes(bts.try_into().unwrap());
            Ok(Value::U128(v))
        },
        _ => itr_err_fmt!(CastFail, "cannot cast 0x{} to uint", 
            hex::encode(buf)),
    }
}



macro_rules! checked_uint {
    ($nty:ty) => (
        concat_idents::concat_idents!{ fname = checked_, $nty {
        pub fn fname(&self) -> VmrtRes<$nty> {
            let un = match self {
                U8(n)   => *n as u128,
                U16(n)  => *n as u128,
                U32(n)  => *n as u128,
                U64(n)  => *n as u128,
                U128(n) => *n as u128,
                _ => return itr_err_fmt!(CastParamFail, "cannot cast type {:?} to {}", self, stringify!($nty))
            };
            if un > <$nty>::MAX as u128 {
                return itr_err_fmt!(CastParamFail, "cannot cast param {:?} to {}", un, stringify!($nty))
            }
            Ok(un as $nty)
        }}
        }
    )
}



impl Value {

    checked_uint!{u8}
    checked_uint!{u16}
    checked_uint!{u32}
    checked_uint!{u64}
    checked_uint!{u128}

    pub fn checked_uint(&self) -> VmrtRes<u128> {
        self.checked_u128()
    }


    pub fn ___checked_bytes(&self) -> VmrtRes<Vec<u8>> {
        let canto = self.is_bytes() || self.is_addr() || self.is_uint();
        match canto {
            true => Ok(self.raw()),
            _ => itr_err_fmt!(CastParamFail, "cannot cast {:?} to buf", self)
        }
    }

    /*

    pub fn checked_bool(&self) -> VmrtRes<bool> {
        let canto = self.is_nil() || self.is_uint() || self.is_bytes();
        match canto {
            true => Ok(self.to_bool()),
            _ => itr_err_fmt!(CastParamFail, "cannot cast {:?} to bool", self)
        }
    }

    pub fn checked_bool_not(&self) -> VmrtRes<bool> {
        Ok(!self.checked_bool()?)
    }

    */

    pub fn type_from(ty: ValueTy, stuff: Vec<u8>) -> VmrtRes<Self> {
        let vlen = stuff.len();
        macro_rules! val {()=>{ Self::Bytes(stuff.clone()) }}
        macro_rules! cst {($c: ident)=>{ {let mut v = val!(); v.$c()?; v } }}
        macro_rules! err {()=>{ itr_err_fmt!(CastParamFail, "cannot cast 0x{} to type id {:?}", stuff.clone().to_hex(), ty) }}
        let cklen = |l, v| maybe!(vlen==l, Ok(v), err!());
        match ty {
            ValueTy::Nil       => cklen(0,  Self::Nil),
            ValueTy::Bool      => cklen(1,  Self::bool(stuff[0]==1)),
            ValueTy::U8        => cklen(1,  Self::u8(stuff[0])),
            ValueTy::U16       => cklen(2,  cst!(cast_u16) ),
            ValueTy::U32       => cklen(4,  cst!(cast_u32) ),
            ValueTy::U64       => cklen(8,  cst!(cast_u64) ),
            ValueTy::U128      => cklen(16, cst!(cast_u128) ),
            ValueTy::Bytes     => Ok(Self::Bytes(stuff)),
            ValueTy::Addr      => {
                if vlen != Address::SIZE {
                    return err!()
                }
                let addr = Address::must_vec(stuff);
                map_err_itr!(CastFail, addr.check_version())?;
                Ok(Self::Addr(addr))
            },
            _ => err!(),
        }
    }
    

    pub fn checked_address(&self) -> VmrtRes<Address> {
        match self {
            Bytes(adr) => map_err_itr!(CastParamFail, Address::from_bytes(adr)),
            _ => itr_err_fmt!(CastParamFail, "cannot cast {:?} to address", self)
        }
    }

    pub fn checked_contract_address(&self) -> VmrtRes<ContractAddress> {
        let addr = self.checked_address()?;
        map_err_itr!(ContractAddrErr, ContractAddress::from_addr(addr))
    }

    pub fn checked_fnsign(&self) -> VmrtRes<FnSign> {
        match self {
            U32(u) => Ok(u.to_be_bytes()),
            Bytes(b) => checked_func_sign(&b),
            _ => itr_err_fmt!(ContractAddrErr, "cannot cast {:?} to fn sign", self)
        }
    }


}

