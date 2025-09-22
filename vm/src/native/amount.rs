



fn amount_to_mei(buf: &[u8]) -> VmrtRes<Value> {
    let hacash = map_err_itr!(NativeCallError, Amount::build(buf))?;
    let Some(mei) = hacash.to_mei_u64() else {
        return itr_err_fmt!(NativeCallError, "call amount_to_mei overflow")
    };
    Ok(Value::U64( mei ))
}


fn amount_to_zhu(buf: &[u8]) -> VmrtRes<Value> {
    let hacash = map_err_itr!(NativeCallError, Amount::build(buf))?;
    let Some(zhu) = hacash.to_zhu_u128() else {
        return itr_err_fmt!(NativeCallError, "call amount_to_zhu overflow")
    };
    Ok(Value::U128( zhu ))
}


