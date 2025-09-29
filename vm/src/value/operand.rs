
macro_rules! inc_dec_opt_define {
    ($self:ident, $ty:ident, $ty2:ident, $f:ident, $n:expr) => {
        if let $ty(v) = $self {
            *$self = match v.$f($n as $ty2).map(Self::$ty) {
                Some(v) => v,
                _ => return itr_err_fmt!(CastFail, "do {} error with {} and {}", stringify!($f), v, $n),
            };
            return Ok(())
        }
    };
}



impl Value {

    // must uint
    #[inline(always)]
    pub fn inc(&mut self, n: u8) -> VmrtErr {
        inc_dec_opt_define!{self,   U8,   u8, checked_add, n}
        inc_dec_opt_define!{self,  U16,  u16, checked_add, n}
        inc_dec_opt_define!{self,  U32,  u32, checked_add, n}
        inc_dec_opt_define!{self,  U64,  u64, checked_add, n}
        inc_dec_opt_define!{self, U128, u128, checked_add, n}
        itr_err_fmt!(CastFail, "inst inc cannot cast {:?} to uint", self)
    }

    // must uint
    #[inline(always)]
    pub fn dec(&mut self, n: u8) -> VmrtErr {
        inc_dec_opt_define!{self,   U8,   u8, checked_sub, n}
        inc_dec_opt_define!{self,  U16,  u16, checked_sub, n}
        inc_dec_opt_define!{self,  U32,  u32, checked_sub, n}
        inc_dec_opt_define!{self,  U64,  u64, checked_sub, n}
        inc_dec_opt_define!{self, U128, u128, checked_sub, n}
        itr_err_fmt!(CastFail, "inst dec cannot cast {:?} to uint", self)
    }

    // ret u8
    #[inline(always)]
    pub fn cutbyte(&mut self, n: u16) -> VmrtErr {
        let buf = self.canbe_bytes_ec(BytesHandle)?;
        let idx = n as usize;
        if idx > buf.len() {
            return itr_err_fmt!(StackError, "read buf byte overflow")
        }
        *self = Self::U8(buf[idx]);
        Ok(())
    }

    #[inline(always)]
    pub fn cutleft(&mut self, n: u16) -> VmrtErr {
        if n == 0 {
            return itr_err_fmt!(StackError, "length cannot be 0")
        }
        let buf = self.canbe_bytes_ec(BytesHandle)?;
        let spx = n as usize;
        if spx > buf.len() {
            return itr_err_fmt!(StackError, "cut buf left overflow")
        }
        *self = Self::Bytes(buf[..spx].to_vec());
        Ok(())
    }
    
    #[inline(always)]
    pub fn cutright(&mut self, n: u16) -> VmrtErr {
        if n == 0 {
            return itr_err_fmt!(StackError, "length cannot be 0")
        }
        let buf = self.canbe_bytes_ec(BytesHandle)?;
        let spx = buf.len() as isize - n as isize;
        if spx < 0 {
            return itr_err_fmt!(StackError, "cut buf right overflow")
        }
        *self = Self::Bytes(buf[spx as usize..].to_vec());
        Ok(())
    }

    #[inline(always)]
    pub fn cutout(&mut self, len: Value, ost: Value) -> VmrtErr {
        let len = len.checked_u16()? as usize;
        let ost = ost.checked_u16()? as usize;
        if len == 0 {
            return itr_err_fmt!(StackError, "length cannot be 0")
        }
        let val = self.canbe_bytes_ec(BytesHandle)?;
        let end = len + ost;
        if end > val.len() {
            return itr_err_fmt!(StackError, "cutout buf overflow")
        }
        *self = Self::Bytes(val[ost..end].to_vec());
        Ok(())
    }

    #[inline(always)]
    pub fn dropleft(&mut self, n: u16) -> VmrtErr {
        if n == 0 {
            return itr_err_fmt!(StackError, "length cannot be 0")
        }
        let buf = self.canbe_bytes_ec(BytesHandle)?;
        let spx = n as usize;
        if spx > buf.len() {
            return itr_err_fmt!(StackError, "drop buf left overflow")
        }
        *self = Self::Bytes(buf[spx..].to_vec());
        Ok(())
    }

    pub fn dropright(&mut self, n: u16) -> VmrtErr {
        if n == 0 {
            return itr_err_fmt!(StackError, "length cannot be 0")
        }
        let buf = self.canbe_bytes_ec(BytesHandle)?;
        let spx = buf.len() as isize - n as isize;
        if spx < 0 {
            return itr_err_fmt!(StackError, "drop buf right overflow")
        }
        *self = Self::Bytes(buf[0..spx as usize].to_vec());
        Ok(())
    }

    /*
        return buf: b + a
    */
    #[inline(always)]
    pub fn concat(a: &Value, b: &Value, cap: &SpaceCap) -> VmrtRes<Value> {
        let e = BytesHandle;
        let v = vec![b.canbe_bytes_ec(e)?, a.canbe_bytes_ec(e)?].concat();
        Ok(Value::bytes(v).valid(cap)?)
    }

}