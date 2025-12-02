

impl Value {

    pub fn canbe_bytes_ec(&self, ec: ItrErrCode) -> VmrtRes<Vec<u8>> {
        Ok(match self {
            Bool(b)    => vec![maybe!(b, 1, 0)],
            U8(n)      => n.to_be_bytes().into(),
            U16(n)     => n.to_be_bytes().into(),
            U32(n)     => n.to_be_bytes().into(),
            U64(n)     => n.to_be_bytes().into(),
            U128(n)    => n.to_be_bytes().into(),
            Bytes(b)   => b.clone(),
            Address(a) => a.to_vec(),
            _ => return itr_err_code!(ec)
        })
    }

    pub fn canbe_bytes(&self) -> VmrtRes<Vec<u8>> {
        self.canbe_bytes_ec(CastBeBytesFail)
    }

    pub fn canbe_key(&self) -> VmrtRes<Vec<u8>> {
        let ec = CastBeKeyFail;
        match self {
            Bool(..) => itr_err_code!(ec),
            _ => self.canbe_bytes_ec(ec),
        }
    }

    pub fn canbe_value(&self) -> VmrtErr {
        let ec = CastBeValueFail;
        match self {
            Nil |
            Bool(..)|
            U8(..)    |
            U16(..)   |
            U32(..)   |
            U64(..)   |
            U128(..)  |
            Bytes(..) |
            Address(..) => Ok(()),
            _ => itr_err_code!(ec)
        }
    }

    pub fn canbe_store(&self) -> VmrtErr {
        self.canbe_value()
    }

    pub fn canbe_uint(&self) -> VmrtErr {
        match self {
            U8(..)   |
            U16(..)  |
            U32(..)  |
            U64(..)  |
            U128(..) => Ok(()),
            _ => itr_err_code!(CastBeUintFail)
        }
    }

    pub fn canbe_ext_call_data(&self, heap: &Heap) -> VmrtRes<Vec<u8>> {
        let ec = CastBeCallDataFail;
        match self {
            Nil => Ok(vec![]),
            HeapSlice((s, l)) => {
                match heap.do_read(*s as usize, *l as usize)? {
                    Bytes(buf) => Ok(buf),
                    _ => never!()
                }
            },
            _ => self.canbe_bytes_ec(ec),
        }
    }

    pub fn canbe_func_argv(&self) -> VmrtErr {
        match self {
            HeapSlice(..) => itr_err_code!(CastBeFnArgvFail),
            _ => Ok(())
        }
    }





}