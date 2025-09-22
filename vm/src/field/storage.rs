
pub const STORAGE_PERIOD: u64 = 300; // 300 block = 24hour = 1day
pub const STORAGE_PERIOD_MAX: u64  = 3600; // about 10 years
pub const STORAGE_SAVE_MAX: u64    = STORAGE_PERIOD * STORAGE_PERIOD_MAX; // about 10 years
pub const STORAGE_RETAIN: u64 = STORAGE_PERIOD * 10;   // about 30 days


combi_struct!{ ValueSto,
    expire: BlockHeight
    data: Value
}


impl ValueSto {

    fn new(chei: u64, v: Value) -> Self {
        Self {
            expire: BlockHeight::from(chei + STORAGE_PERIOD),
            data: v,
        }
    }

    // return (expire, delete, height)
    fn check(&self, chei: u64) -> (bool, bool, u64) {
        let due = self.expire.uint();
        let isexp = chei > due;
        let isdel = chei > due + STORAGE_RETAIN;
        (isexp, isdel, due)
    }

    fn update(mut self, chei: u64, v: Value) -> Self {
        let (is_expire, _, due) = self.check(chei);
        // save new
        if is_expire || due <= STORAGE_PERIOD {
            self.data = v;
            self.expire = BlockHeight::from( chei + STORAGE_PERIOD );
            return self
        }
        // update
        let (ml, vl) = (v.can_get_size().unwrap() as u64, self.data.can_get_size().unwrap() as u64);
        if ml != vl {
            let mut rest = (due - chei) * ml / vl;
            up_in_range!(rest, STORAGE_PERIOD, STORAGE_SAVE_MAX); // at least 1 day, less than 10 years
            self.expire = BlockHeight::from(chei + rest);
        }
        self.data = v;
        self
    }

    // return gas cost
    fn rent(&mut self, gst: &GasExtra, chei: u64, v: Value) -> VmrtRes<i64> {
        let ( _, is_delete, _) = self.check(chei);
        if is_delete {
            return itr_err_fmt!(StorageError, "renewal failed, data invalid")
        }
        if ! v.is_uint() {
            return itr_err_fmt!(StorageError, "period value {:?} is not uint type", v)
        }
        let period = v.to_uint();
        if period < 1 {
            return itr_err_fmt!(StorageError, "period min is 1")
        }
        if period > u16::MAX as u128 {
            return itr_err_fmt!(StorageError, "period value overflow")
        }
        let period = period as u64;
        if period > STORAGE_PERIOD_MAX {
            return itr_err_fmt!(StorageError, "period value max is {} but got {}",
                STORAGE_PERIOD_MAX, period)
        }
        // save
        let exp = self.expire.uint() + (period * STORAGE_PERIOD);
        self.expire = BlockHeight::from(exp);
        // gas
        let vbasesz = gst.storege_value_base_size;
        let gas = (self.data.can_get_size().unwrap() as i64 + vbasesz) * period as i64;
        Ok(gas)
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

    fn skey(cadr: &Address, key: &Value) -> VmrtRes<ValueKey> {
        if ! cadr.is_contract() {
            return itr_err_fmt!(StorageError, "storage use must in contract")
        }
        let k = key.canbe_key()?;
        if k.is_empty() {
            return itr_err_code!(StorageKeyInvalid)
        }
        let mut k = vec![cadr.to_vec(), k].concat();
        if k.len() > Hash::SIZE {
            k = sys::sha3(k).to_vec();
        }
        Ok(ValueKey::from(k))
    }

    /*
        if not find return Nil  
    */
    fn sread(&mut self, curhei: u64, cadr: &ContractAddress, k: &Value) -> VmrtRes<Option<ValueSto>> {
        let k = Self::skey(cadr, k)?;
        let Some(v) = self.ctrtkvdb(&k) else {
            return Ok(None) // not find
        };
        let (is_expire, is_delete, _) = v.check(curhei);
        if is_delete {
            self.ctrtkvdb_del(&k);
            return Ok(None) // over delete
        }
        if is_expire {
            return Ok(None) // time expire
        }
        Ok(Some(v))
    }


    /*
        if not find return Nil  
    */
    fn sload(&mut self, curhei: u64, cadr: &ContractAddress, k: &Value) -> VmrtRes<Value> {
        let Some(v) = self.sread(curhei, cadr, k)? else {
            return Ok(Value::Nil)
        };
        Ok(v.data)
    }

    /*
        if not find return Nil  
    */
    fn stime(&mut self, curhei: u64, cadr: &ContractAddress, k: &Value) -> VmrtRes<Value> {
        let Some(v) = self.sread(curhei, cadr, k)? else {
            return Ok(Value::Nil)
        };
        Ok(Value::U64(v.expire.uint() - curhei))
    }

    /*
        read old value 
    */
    fn ssave(&mut self, curhei: u64, cadr: &ContractAddress, k: Value, v: Value) -> VmrtErr {
        v.canbe_store()?; // check can store
        let k = Self::skey(cadr, &k)?;
        let vobj = match self.ctrtkvdb(&k) {
            Some(vold) => vold.update(curhei, v), // update
            _ => ValueSto::new(curhei, v) // new
        };
        self.ctrtkvdb_set(&k, &vobj);
        Ok(())
    }

    // return gas use
    fn srent(&mut self, gst: &GasExtra, curhei: u64, cadr: &ContractAddress, k: Value,  p: Value) -> VmrtRes<i64> {
        let k = Self::skey(cadr, &k)?;
        let Some(mut v) = self.ctrtkvdb(&k) else {
            return itr_err_code!(StorageKeyNotFind)
        };
        let (_, is_delete, _) = v.check(curhei);
        if is_delete {
            return itr_err_fmt!(StorageExpired, "data deleted")
        }
        let gas = v.rent(gst, curhei, p)?;
        self.ctrtkvdb_set(&k, &v);
        Ok(gas)
    }

    fn sdel(&mut self, cadr: &ContractAddress, k: Value) -> VmrtErr {
        let k = Self::skey(cadr, &k)?;
        self.ctrtkvdb_del(&k);
        Ok(())
    }


}



