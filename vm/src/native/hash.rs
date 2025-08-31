

// sha3
fn sha3(v: &Value) -> VmrtRes<Value> {
    let stuff = v.checked_bytes()?;
    if stuff.is_empty() {
        return itr_err_fmt!(NativeCallError, "cannot do sha3 with empty bytes")
    }
    let result = sys::sha3(stuff);
    Ok(Value::bytes(result.to_vec()))
}


// sha2
#[allow(dead_code)]
fn sha2(v: &Value) -> VmrtRes<Value> {
    let stuff = v.checked_bytes()?;
    if stuff.is_empty() {
        return itr_err_fmt!(NativeCallError, "cannot do sha2 with empty bytes")
    }
    let result = sys::sha2(stuff);
    Ok(Value::bytes(result.to_vec()))
}


// ripemd160
#[allow(dead_code)]
fn ripemd160(v: &Value) -> VmrtRes<Value> {
    let stuff = v.checked_bytes()?;
    if stuff.is_empty() {
        return itr_err_fmt!(NativeCallError, "cannot do ripemd160 with empty bytes")
    }
    let result = sys::ripemd160(stuff);
    Ok(Value::bytes(result.to_vec()))
}
