
fn address_ptr(buf: &[u8]) -> VmrtRes<Value> {
    if buf.len() != 1 {
        return itr_err_fmt!(NativeCallError, "param error")
    }
    const DVN: u8 = ADDR_OR_PTR_DIV_NUM;
    let idx = buf[0];
    let max = u8::MAX - DVN;
    if idx > max {
        return itr_err_fmt!(NativeCallError, "address_ptr param max {} but got {}", max, idx)
    }
    Ok(Value::U8( idx + DVN ))
}

fn context_address(buf: &[u8]) -> VmrtRes<Value> {
    let ctxadr = field::Address::from_bytes(buf).unwrap();
    Ok(Value::Address(ctxadr))
}



