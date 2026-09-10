fn is_scalar_value(value: &Value) -> bool {
    matches!(
        value,
        Nil | Bool(..) | U8(..) | U16(..) | U32(..) | U64(..) | U128(..) | Bytes(..) | Address(..)
    )
}

fn check_scalar_as(value: &Value, ec: ItrErrCode) -> VmrtErr {
    if is_scalar_value(value) {
        Ok(())
    } else {
        itr_err_code!(ec)
    }
}

fn check_func_tuple_item(value: &Value, ec: ItrErrCode) -> VmrtErr {
    match value {
        Tuple(..) => itr_err_code!(ec),
        Compo(..) | Handle(..) => Ok(()),
        _ => check_scalar_as(value, ec),
    }
}

fn check_func_boundary(value: &Value, ec: ItrErrCode) -> VmrtErr {
    match value {
        Tuple(tuple) => {
            for item in tuple.as_slice() {
                check_func_tuple_item(item, ec)?;
            }
            Ok(())
        }
        Compo(..) | Handle(..) => Ok(()),
        _ => check_scalar_as(value, ec),
    }
}

fn check_vm_boundary_compo(compo: &CompoItem, ec: ItrErrCode) -> VmrtErr {
    if let Ok(list) = compo.list_ref() {
        for item in &*list {
            check_scalar_as(item, ec)?;
        }
        return Ok(());
    }
    for value in compo.map_ref()?.values() {
        check_scalar_as(value, ec)?;
    }
    Ok(())
}

fn check_vm_tuple_item(value: &Value, ec: ItrErrCode) -> VmrtErr {
    match value {
        Tuple(..) | Handle(..) => itr_err_code!(ec),
        Compo(compo) => check_vm_boundary_compo(compo, ec),
        _ => check_scalar_as(value, ec),
    }
}

fn check_vm_boundary(value: &Value, ec: ItrErrCode) -> VmrtErr {
    match value {
        Tuple(tuple) => {
            for item in tuple.as_slice() {
                check_vm_tuple_item(item, ec)?;
            }
            Ok(())
        }
        Compo(compo) => check_vm_boundary_compo(compo, ec),
        Handle(..) => itr_err_code!(ec),
        _ => check_scalar_as(value, ec),
    }
}

impl Value {
    pub fn check_non_nil_scalar(&self, nil_ec: ItrErrCode) -> VmrtErr {
        if matches!(self, Nil) {
            return itr_err_code!(nil_ec);
        }
        check_scalar_as(self, CastBeValueFail)
    }

    pub fn check_boundary_value_cap(&self, cap: &SpaceCap) -> VmrtErr {
        match self {
            Tuple(tuple) => {
                for item in tuple.as_slice() {
                    item.check_boundary_value_cap(cap)?;
                }
                Ok(())
            }
            Compo(compo) => {
                if let Ok(list) = compo.list_ref() {
                    for item in &*list {
                        item.check_boundary_value_cap(cap)?;
                    }
                    return Ok(());
                }
                for (key, value) in &*compo.map_ref()? {
                    if key.len() > cap.value_size {
                        return itr_err_code!(OutOfValueSize);
                    }
                    value.check_boundary_value_cap(cap)?;
                }
                Ok(())
            }
            _ => {
                self.clone().valid(cap)?;
                Ok(())
            }
        }
    }

    pub(crate) fn extract_bytes_len_with_error_code(&self, ec: ItrErrCode) -> VmrtRes<usize> {
        match self {
            Bool(..) | U8(..) => Ok(1),
            U16(..) => Ok(2),
            U32(..) => Ok(4),
            U64(..) => Ok(8),
            U128(..) => Ok(16),
            Bytes(b) => Ok(b.len()),
            Address(..) => Ok(field::Address::SIZE),
            _ => itr_err_code!(ec),
        }
    }

    /// Runtime byte normalization (`extract_bytes_ec` in `vm/doc/value-cast.md`). `Nil` rejected here;
    /// field serialization uses [`Value::scalar_bytes`]; only [`Self::extract_call_data`] maps `Nil` to `[]`.
    fn extract_bytes_with_error_code(&self, ec: ItrErrCode) -> VmrtRes<Vec<u8>> {
        if matches!(self, Nil) {
            return itr_err_code!(ec);
        }
        match self.scalar_bytes() {
            Some(bytes) => Ok(bytes),
            None => itr_err_code!(ec),
        }
    }

    pub fn extract_bytes(&self) -> VmrtRes<Vec<u8>> {
        self.extract_bytes_with_error_code(CastBeBytesFail)
    }

    /// Derive map key bytes from a value. Uint keys use minimal big-endian `uint_key_bytes` (equal uints
    /// share a slot); `Bytes`/`Address` use raw bytes. Bool, Nil, empty `Bytes` rejected. See `vm/doc/value-cast.md` §9.
    pub(crate) fn extract_key_bytes_with_error_code(&self, ec: ItrErrCode) -> VmrtRes<Vec<u8>> {
        match self {
            Bool(..) => itr_err_code!(ec),
            Nil => itr_err_code!(ec),
            U8(..) | U16(..) | U32(..) | U64(..) | U128(..) => {
                Ok(uint_key_bytes(self.extract_u128()?))
            }
            _ => {
                let key = self.extract_bytes_with_error_code(ec)?;
                if key.is_empty() {
                    return itr_err_code!(ec);
                }
                Ok(key)
            }
        }
    }

    pub fn extract_key_bytes(&self) -> VmrtRes<Vec<u8>> {
        self.extract_key_bytes_with_error_code(CastBeKeyFail)
    }

    pub fn check_scalar(&self) -> VmrtErr {
        check_scalar_as(self, CastBeValueFail)
    }

    pub fn check_tuple_item(&self) -> VmrtErr {
        match self {
            Tuple(..) => itr_err_code!(CastBeValueFail),
            Compo(..) | Handle(..) => Ok(()),
            _ => check_scalar_as(self, CastBeValueFail),
        }
    }

    /// Materialize ACTION / Concat-NTFUNC call-data bytes (Concat/ACTION only).
    ///
    /// Concat list = byte fragments (deferred CAT). Packed list = argv vector.
    /// Packed NTFUNC must not call this function.
    ///
    /// - `Nil` → `[]` (no `call_data_size` check).
    /// - One-layer `Compo` list: each element uses CAT's `extract_bytes` rules
    ///   (`extract_bytes_with_error_code`, `Nil` rejected) with `CastBeCallDataFail`;
    ///   concatenate in order; running length ≤ `cap.call_data_size`.
    /// - Other scalars: same extract, then length ≤ `call_data_size`.
    /// - `Tuple` / `Handle` / Map / nested `Compo` / element `Nil` → `CastBeCallDataFail`.
    pub fn extract_call_data(&self, cap: &SpaceCap) -> VmrtRes<Vec<u8>> {
        let ec = CastBeCallDataFail;
        match self {
            Nil => Ok(vec![]),
            Compo(c) => {
                let Ok(list) = c.list_ref() else {
                    return itr_err_code!(ec);
                };
                let mut out = Vec::new();
                for item in list.iter() {
                    let chunk = item.extract_bytes_with_error_code(ec)?;
                    if out.len() + chunk.len() > cap.call_data_size {
                        return itr_err_code!(OutOfValueSize);
                    }
                    out.extend_from_slice(&chunk);
                }
                Ok(out)
            }
            _ => {
                let bytes = self.extract_bytes_with_error_code(ec)?;
                if bytes.len() > cap.call_data_size {
                    return itr_err_code!(OutOfValueSize);
                }
                Ok(bytes)
            }
        }
    }

    pub fn check_func_argv(&self) -> VmrtErr {
        check_func_boundary(self, CastBeFnArgvFail)?;
        if let Tuple(tuple) = self {
            if tuple.len() > crate::MAX_FUNC_PARAM_LEN {
                return itr_err_fmt!(
                    CastBeFnArgvFail,
                    "func argv length cannot more than {}",
                    crate::MAX_FUNC_PARAM_LEN
                );
            }
        }
        Ok(())
    }

    pub fn check_func_retv(&self) -> VmrtErr {
        check_func_boundary(self, CastBeFnRetvFail)
    }

    pub fn check_vm_boundary_argv(&self) -> VmrtErr {
        check_vm_boundary(self, CastBeFnArgvFail)?;
        if let Tuple(tuple) = self {
            if tuple.len() > crate::MAX_FUNC_PARAM_LEN {
                return itr_err_fmt!(
                    CastBeFnArgvFail,
                    "func argv length cannot more than {}",
                    crate::MAX_FUNC_PARAM_LEN
                );
            }
        }
        Ok(())
    }

    pub fn check_vm_boundary_retv(&self) -> VmrtErr {
        match self {
            Value::Handle(..) => {
                itr_err_fmt!(CastBeFnRetvFail, "return type Handle is not supported")
            }
            _ => check_vm_boundary(self, CastBeFnRetvFail),
        }
    }

    pub fn check_container_cap(&self, cap: &SpaceCap) -> VmrtErr {
        match self {
            Tuple(tuple) => {
                if tuple.len() > cap.tuple_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                for item in tuple.as_slice() {
                    item.check_container_cap(cap)?;
                }
                Ok(())
            }
            Compo(compo) => {
                if compo.len() > cap.compo_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
fn extract_call_data_list(items: Vec<Value>) -> Value {
    Value::Compo(CompoItem::list(VecDeque::from(items)).unwrap())
}

#[cfg(test)]
#[test]
fn extract_call_data_nil_is_empty() {
    assert_eq!(
        Value::Nil.extract_call_data(&SpaceCap::new(0)).unwrap(),
        Vec::<u8>::new()
    );
}

#[cfg(test)]
#[test]
fn extract_call_data_scalars_match_extract_bytes() {
    let cap = SpaceCap::new(0);
    let bytes = Value::bytes(b"abc".to_vec());
    assert_eq!(
        bytes.extract_call_data(&cap).unwrap(),
        bytes.extract_bytes().unwrap()
    );
    let u8v = Value::U8(7);
    assert_eq!(
        u8v.extract_call_data(&cap).unwrap(),
        u8v.extract_bytes().unwrap()
    );
    let addr = Value::Address(field::Address::from([7u8; 21]));
    assert_eq!(
        addr.extract_call_data(&cap).unwrap(),
        addr.extract_bytes().unwrap()
    );
}

#[cfg(test)]
#[test]
fn extract_call_data_concat_list_of_bytes() {
    let a = b"aa".to_vec();
    let b = b"bbb".to_vec();
    let c = b"c".to_vec();
    let v = extract_call_data_list(vec![
        Value::bytes(a.clone()),
        Value::bytes(b.clone()),
        Value::bytes(c.clone()),
    ]);
    let mut want = a;
    want.extend_from_slice(&b);
    want.extend_from_slice(&c);
    assert_eq!(v.extract_call_data(&SpaceCap::new(0)).unwrap(), want);
}

#[cfg(test)]
#[test]
fn extract_call_data_concat_list_u8_and_bytes() {
    let v = extract_call_data_list(vec![Value::U8(1), Value::bytes(b"ab".to_vec())]);
    let mut want = Value::U8(1).extract_bytes().unwrap();
    want.extend_from_slice(b"ab");
    assert_eq!(v.extract_call_data(&SpaceCap::new(0)).unwrap(), want);
}

#[cfg(test)]
#[test]
fn extract_call_data_empty_list_is_empty() {
    assert_eq!(
        extract_call_data_list(vec![])
            .extract_call_data(&SpaceCap::new(0))
            .unwrap(),
        Vec::<u8>::new()
    );
}

#[cfg(test)]
#[test]
fn extract_call_data_rejects_nested_nil_tuple_handle_map() {
    let cap = SpaceCap::new(0);
    let inner = extract_call_data_list(vec![Value::bytes(b"x".to_vec())]);
    let nested = CompoItem::new_list();
    nested.list_mut().unwrap().push_back(inner);
    assert_eq!(
        Value::Compo(nested).extract_call_data(&cap).unwrap_err().0,
        CastBeCallDataFail
    );
    assert_eq!(
        extract_call_data_list(vec![Value::Nil])
            .extract_call_data(&cap)
            .unwrap_err()
            .0,
        CastBeCallDataFail
    );
    let tuple = Value::Tuple(TupleItem::new(vec![Value::bytes(b"x".to_vec())]).unwrap());
    assert_eq!(
        tuple.extract_call_data(&cap).unwrap_err().0,
        CastBeCallDataFail
    );
    assert_eq!(
        Value::Handle(HandleItem::new(0u8))
            .extract_call_data(&cap)
            .unwrap_err()
            .0,
        CastBeCallDataFail
    );
    let map = Value::Compo(CompoItem::map(BTreeMap::new()).unwrap());
    assert_eq!(
        map.extract_call_data(&cap).unwrap_err().0,
        CastBeCallDataFail
    );
}

#[cfg(test)]
#[test]
fn extract_call_data_call_data_size_cap() {
    let cap = SpaceCap::new(0);
    let ok = extract_call_data_list(vec![
        Value::bytes(vec![1u8; 1280]),
        Value::bytes(vec![2u8; 1280]),
        Value::bytes(vec![3u8; 1280]),
        Value::bytes(vec![4u8; 768]),
    ]);
    assert_eq!(ok.extract_call_data(&cap).unwrap().len(), 4608);
    let over = extract_call_data_list(vec![
        Value::bytes(vec![1u8; 1280]),
        Value::bytes(vec![2u8; 1280]),
        Value::bytes(vec![3u8; 1280]),
        Value::bytes(vec![4u8; 769]),
    ]);
    assert_eq!(over.extract_call_data(&cap).unwrap_err().0, OutOfValueSize);
}
