

fn mei_to_hac(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let num = buf_to_uint(buf)?.to_uint();
    if num > u64::MAX as u128 {
        return itr_err_fmt!(NativeCallError, "call mei_to_hac amount too big")
    }
    Ok(Value::Bytes( Amount::mei(num as u64).serialize() ))
}

fn hac_to_mei(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let hacash = Amount::build(buf).map_ire(NativeCallError)?;
    let Some(mei) = hacash.to_mei_u64() else {
        return itr_err_fmt!(NativeCallError, "call hac_to_mei overflow")
    };
    Ok(Value::U64( mei ))
}


fn hac_to_zhu(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let hacash = Amount::build(buf).map_ire(NativeCallError)?;
    let Some(zhu) = hacash.to_zhu_u128() else {
        return itr_err_fmt!(NativeCallError, "call hac_to_zhu overflow")
    };
    Ok(Value::U128( zhu ))
}


fn zhu_to_hac(_: u64, buf: &[u8]) -> VmrtRes<Value> {
    let num = buf_to_uint(buf)?.to_uint();
    if num > u64::MAX as u128 {
        return itr_err_fmt!(NativeCallError, "call zhu_to_hac overflow")
    }
    Ok(Value::Bytes( Amount::zhu(num as u64).serialize() ))
}


