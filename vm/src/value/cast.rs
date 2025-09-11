

macro_rules! cast_buf_to_tar_uint {
    ($self:ident, $ty:ty, $l:expr, $t:ident) => {
        if let Bytes(buf) = $self {
            let bl = buf.len();
            let mut buf = buf.clone();
            if bl < $l {
                buf = buf_fill_left_zero(&buf, $l);
            }else if bl > $l {
                let bf = buf_drop_left_zero(&buf, $l);
                if bf.len() > $l {
                    return cannot_cast_err($self,"UINT")
                }
                buf = bf;
            }
            *$self = $t(<$ty>::from_be_bytes(buf.try_into().unwrap())); // false
            return Ok(())
        }
    };
}




macro_rules! cast_up_to_low {
    ($self: expr, $t1: ty, $t11: ident, $t2: ty, $t22: ident) => {
        if let $t22(n) = $self {
            if *n <= <$t1>::MAX as $t2 {
                *$self = $t11(*n as $t1);
                return Ok(())
            }
        }
    }
}


macro_rules! cast_low_to_up {
    ($self: expr, $t11: ident, $t2: ty, $t22: ident) => {
        if let $t11(n) = $self { 
            *$self = $t22(*n as $t2); 
            return Ok(()) 
        }
    }
}


fn cannot_cast_err(v: &Value, ty: &str) -> VmrtErr {
    itr_err_fmt!(CastFail, "cannot cast {:?} to {}", v, ty)
}


impl Value {

    pub fn cast_bool(&mut self) {
        *self = Bool(maybe!(self.check_true(), true, false));
    }

    pub fn cast_bool_not(&mut self) {
        *self = Bool(maybe!(self.check_true(), false, true));
    }

    pub fn cast_u8(&mut self) -> VmrtErr {
        cast_buf_to_tar_uint!{self, u8, 1, U8}
        if let U8(_) = self { return Ok(()) }
        cast_up_to_low!{self, u8, U8, u16, U16}
        cast_up_to_low!{self, u8, U8, u32, U32}
        cast_up_to_low!{self, u8, U8, u64, U64}
        cast_up_to_low!{self, u8, U8, u128, U128}
        cannot_cast_err(self, "U8") // error
    }

    pub fn cast_u16(&mut self) -> VmrtErr {
        cast_buf_to_tar_uint!{self, u16, 2, U16}
        cast_low_to_up!{self, U8, u16, U16}
        if let U16(_) = self { return Ok(()) }
        cast_up_to_low!{self, u16, U16, u32, U32}
        cast_up_to_low!{self, u16, U16, u64, U64}
        cast_up_to_low!{self, u16, U16, u128, U128}
        cannot_cast_err(self, "U16") // error
    }

    pub fn cast_u32(&mut self) -> VmrtErr {
        cast_buf_to_tar_uint!{self, u32, 4, U32}
        cast_low_to_up!{self, U8, u32, U32}
        cast_low_to_up!{self, U16, u32, U32}
        if let U32(_) = self { return Ok(()) }
        cast_up_to_low!{self, u32, U32, u64, U64}
        cast_up_to_low!{self, u32, U32, u128, U128}
        cannot_cast_err(self, "U32") // error
    }

    pub fn cast_u64(&mut self) -> VmrtErr {
        cast_buf_to_tar_uint!{self, u64, 8, U64}
        cast_low_to_up!{self, U8, u64, U64}
        cast_low_to_up!{self, U16, u64, U64}
        cast_low_to_up!{self, U32, u64, U64}
        if let U64(_) = self { return Ok(()) }
        cast_up_to_low!{self, u64, U64, u128, U128}
        cannot_cast_err(self, "U64") // error
    }

    pub fn cast_u128(&mut self) -> VmrtErr {
        cast_buf_to_tar_uint!{self, u128, 16, U128}
        cast_low_to_up!{self, U8, u128, U128}
        cast_low_to_up!{self, U16, u128, U128}
        cast_low_to_up!{self, U32, u128, U128}
        cast_low_to_up!{self, U64, u128, U128}
        if let U128(_) = self { return Ok(()) }
        cannot_cast_err(self, "U128") // ERROR
    }

    pub fn cast_buf(&mut self) -> VmrtErr {
        match &self {
            U8(n) =>   *self = Bytes(n.to_be_bytes().into()),
            U16(n) =>  *self = Bytes(n.to_be_bytes().into()),
            U32(n) =>  *self = Bytes(n.to_be_bytes().into()),
            U64(n) =>  *self = Bytes(n.to_be_bytes().into()),
            U128(n) => *self = Bytes(n.to_be_bytes().into()),
            Bytes(_buf) => {},
            a => return itr_err_fmt!(CastFail, "cannot cast {} to bytes", a)
        };
        Ok(())
    }






}






