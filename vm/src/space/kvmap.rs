


macro_rules! memory_kvmap_define {
    ($class:ident, $er1:expr, $er2:expr) => {
                
        #[allow(dead_code)]
        #[derive(Default)]
        pub struct $class {
            limit: usize,
            datas: HashMap<Vec<u8>, Value>,
        }

        impl $class {

            pub fn new(limit: usize) -> Self {
                Self {
                    limit,
                    datas: HashMap::new(),
                }
            }

            pub fn reset(&mut self, lmt: usize) {
                self.limit = lmt;
                self.clear();
            }

            pub fn clear(&mut self) {
                self.datas.clear();
            }

            fn key(k: &Value) -> VmrtRes<Vec<u8>> {
                let key = k.canbe_key()?;
                if key.is_empty() {
                    return itr_err_fmt!($er1, "key {} cannot empty", k)
                }
                Ok(key)
            }

            pub fn put(&mut self, k: Value, v: Value) -> VmrtErr {
                v.canbe_value()?;
                self.datas.insert(Self::key(&k)?, v);
                if self.datas.len() > self.limit {
                    return itr_err_code!($er2) // out of limit
                }
                Ok(())
            }

            pub fn get(&self, k: &Value) -> VmrtRes<Value> {
                Ok(match self.datas.get(&Self::key(k)?) {
                    Some(v) => v.clone(),
                    None => Value::Nil,
                })
            }

        }
    };
}



/*

*/
memory_kvmap_define!{ GKVMap, GlobalError, OutOfGlobal }
memory_kvmap_define!{ MKVMap, MemoryError, OutOfMemory }


/*

*/


#[derive(Default)]
pub struct CtcKVMap {
    limit: usize,
    datas: HashMap<Address, MKVMap>
}


impl CtcKVMap {

    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            datas: HashMap::new(),
        }
    }

    pub fn reset(&mut self, lmt: usize) {
        self.limit = lmt;
    }

    pub fn clear(&mut self) {
        self.datas.clear();
    }

    pub fn entry(&mut self, addr: &Address) -> VmrtRes<&mut MKVMap> {
        addr.check_version().map_ires(MemoryError, format!("memory use must in dffective address but in {}", addr.readable()))?;
        Ok(self.datas.entry(addr.clone()).or_insert_with(||MKVMap::new(self.limit)))
    }

}

