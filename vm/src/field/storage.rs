
pub const STORAGE_PERIOD: u64 = 100; // 100 block = 8hour = 0.3day
pub const STORAGE_PERIOD_MAX: u32 = 256*256*256 - 1; // max = 62*256 = 15960 years


combi_struct!{ ValueZip,
    length: Uint2 // old data len
    hashdt: HashHalf // sha256[0..16]
}

combi_option!{ ValueData,
    Value, ValueZip
}

#[allow(dead_code)]
impl ValueData {
    fn cast_zip(&mut self) {
        let vbuf = self.clone().must_value().raw();
        let length = vbuf.len();
        let hxh = Hash::from(sys::sha2(vbuf)).half();
        *self = Self::Val2(ValueZip{
            length: Uint2::from(length as u16),
            hashdt: hxh
        });
    }
    fn do_restore(&mut self, v: Value) -> VmrtErr {
        if v.val_size() != self.val_size() {
            return itr_err_code!(StorageValSizeErr)
        }
        match self {
            Self::Val1(v1) => {
                if v != *v1 {
                    return itr_err_code!(StorageRestoreNotMatch)
                }
            },
            Self::Val2(_) => {
                let mut zip = Self::Val1(v.clone());
                zip.cast_zip();
                if zip != *self {
                    return itr_err_code!(StorageRestoreNotMatch)
                }
                // data value recover
                *self = Self::Val1(v); // update
            }
        }
        Ok(())
    }
    fn must_value(self) -> Value {
        match self {
            Self::Val1(v) => v,
            _ => unreachable!()
        }
    }
    fn must_zip(self) -> ValueZip {
        match self {
            Self::Val2(v) => v,
            _ => unreachable!()
        }
    }
    fn is_zip(self) -> bool {
        match self {
            Self::Val2(..) => true,
            _ => false,
        }
    }
    fn val_size(&self) -> usize {
        match self {
            Self::Val1(v) => v.val_size(),
            Self::Val2(v) => v.length.uint() as usize,
        }

    }
}

combi_struct!{ ValueSto,
    start: BlockHeight
    period: Uint3      // 62*256 = 15960years
    data: ValueData
}

impl ValueSto {

    fn add_period(&mut self, period: u16) -> VmrtErr {
        let Some(np) = (*self.period).checked_add(period as u32) else {
            return itr_err_code!(StoragePeriodErr)
        };
        if np > STORAGE_PERIOD_MAX {
            return itr_err_code!(StoragePeriodErr)
        }
        self.period = Uint3::from(np);
        Ok(())
    }
}


/*
* 
*/
inst_state_define!{ VMState,

    201, contract,  ContractAddress  :  ContractSto
    202, ctrtkvdb,  ValueKey         :  ValueSto

}






/*
    state storage
*/
#[allow(dead_code)]
impl VMState<'_> {

    fn key(cadr: &Address, key: &Value) -> VmrtRes<ValueKey> {
        if ! cadr.is_contract() {
            return itr_err_fmt!(StorageError, "storage use must in contract")
        }
        let ks = key.checked_bytes()?;
        if ks.is_empty() {
            return itr_err_code!(StorageKeyInvalid)
        }
        let mut k = vec![cadr.to_vec(), ks].concat();
        if k.len() > Hash::SIZE {
            k = sys::sha3(k).to_vec();
        }
        Ok(ValueKey::from(k))
    }

    fn period(v: &ValueSto) -> (u64, u64) {
        let period = *v.period as u64;
        let mut recovr = period / 10;
        set_in_range!(recovr, 1, 2222);
        let expire = *v.start + period * STORAGE_PERIOD;
        let delete =   expire + recovr * STORAGE_PERIOD;
        (expire, delete)
    }

    fn expire(height: u64, v: &ValueSto) -> (bool, bool) {
        let (e, d) = Self::period(v);
        (height>e, height>d)
    }
    
    fn cast_zip(v: &mut ValueSto) -> bool {
        let ValueData::Val1(ref val) = v.data else {
            return false
        };
        if val.val_size() <= HashHalf::SIZE {
            return false // not need zip
        }
        v.data.cast_zip(); // change data
        true // yes do
    }


    /*
        if not find return Nil  
    */
    fn load(&mut self, height: u64, cadr: &ContractAddress, k: &Value) -> VmrtRes<Value> {
        let k = Self::key(cadr, k)?;
        let Some(mut v) = self.ctrtkvdb(&k) else {
            return Ok(Value::Nil) // not find
        };
        let (is_expire, is_delete) = Self::expire(height, &v);
        if is_delete {
            self.ctrtkvdb_del(&k);
            return Ok(Value::Nil) // over delete
        }
        if is_expire {
            if Self::cast_zip(&mut v) {
                self.ctrtkvdb_set(&k, &v); // save zip
            }
            return Ok(Value::Nil) // time expire
        }
        Ok(v.data.must_value())
    }

    /*
        read old value 
    */
    fn save(&mut self, height: u64, period: u16, cadr: &ContractAddress, k: Value, v: Value) -> VmrtErr {
        if period < 1 {
            return itr_err_code!(StoragePeriodErr)
        }
        let mut period = period as u32;
        let vl = v.val_size();
        let k = Self::key(cadr, &k)?;
        if let Some(vold) = self.ctrtkvdb(&k) {
            let (exp, _) = Self::period(&vold);
            if height <= exp {
                let vl_old = vold.data.val_size();
                let leftoverhei = (exp - height) as u128 * vl_old as u128 / vl as u128;
                period += (leftoverhei / STORAGE_PERIOD as u128) as u32
            }
        }
        if period > STORAGE_PERIOD_MAX {
            return itr_err_code!(StoragePeriodErr)
        }
        let vsto = ValueSto {
            start: BlockHeight::from(height),
            period: Uint3::from(period),
            data: ValueData::Val1(v),
        };
        self.ctrtkvdb_set(&k, &vsto);
        Ok(())
    }

    // return gas use
    fn renew(&mut self, gst: &GasExtra, height: u64, period: u16, cadr: &ContractAddress, k: Value) -> VmrtRes<i64> {
        if period < 1 {
            return itr_err_code!(StoragePeriodErr)
        }
        let k = Self::key(cadr, &k)?;
        let Some(mut v) = self.ctrtkvdb(&k) else {
            return itr_err_code!(StorageKeyNotFind)
        };
        let (is_expire, _) = Self::expire(height, &v);
        if is_expire {
            return itr_err_code!(StorageExpired)
        }
        // update sto
        v.add_period(period)?;
        // gas = (42 + vl) * period
        let gas = period as i64 * (gst.storage_save_base + v.data.val_size() as i64);
        Ok(gas)
    }

    fn restore(&mut self, height: u64, cadr: &ContractAddress, k: Value, v: Value) -> VmrtErr {
        let k = Self::key(cadr, &k)?;
        let Some(mut oldv) = self.ctrtkvdb(&k) else {
            return itr_err_code!(StorageKeyNotFind)
        };
        let (is_expire, is_delete) = Self::expire(height, &oldv);
        if is_delete {
            self.ctrtkvdb_del(&k);
            return itr_err_code!(StorageKeyNotFind)
        }
        if ! is_expire {
            return itr_err_code!(StorageNotExpired)
        }
        // do re store
        oldv.data.do_restore(v)?;
        oldv.start = BlockHeight::from(height);
        oldv.period = Uint3::from(1);
        Ok(())
    }

    fn delete(&mut self, cadr: &ContractAddress, k: Value) -> VmrtErr {
        let k = Self::key(cadr, &k)?;
        self.ctrtkvdb_del(&k);
        Ok(())
    }


}



