



fn amount_to_zhu(v: &Value) -> VmrtRes<Value> {
    let buf = v.to_bytes();
    let hacash = Amount::build(&buf).map_err(|e|{
        ItrErr::new(NativeCallError, e.as_str())
    })?;
    let Some(zhu) = hacash.to_zhu_u64() else {
        return itr_err_fmt!(NativeCallError, "call amount_to_zhu overflow")
    };
    Ok(Value::U64( zhu ))
}


