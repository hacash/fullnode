

// sha3
fn sha3(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    if buf.is_empty() {
        return itr_err_fmt!(NativeCallError, "cannot do sha3 with empty bytes")
    }
    let result = sys::sha3(buf);
    Ok(Value::bytes(result.to_vec()))
}


// sha2
#[allow(dead_code)]
fn sha2(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    if buf.is_empty() {
        return itr_err_fmt!(NativeCallError, "cannot do sha2 with empty bytes")
    }
    let result = sys::sha2(buf);
    Ok(Value::bytes(result.to_vec()))
}


// ripemd160
#[allow(dead_code)]
fn ripemd160(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    if buf.is_empty() {
        return itr_err_fmt!(NativeCallError, "cannot do ripemd160 with empty bytes")
    }
    let result = sys::ripemd160(buf);
    Ok(Value::bytes(result.to_vec()))
}
