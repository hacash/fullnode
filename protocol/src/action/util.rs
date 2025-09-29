
pub fn check_vaild_store_item_key(tip: &str, kobj: impl AsRef<[u8]>, ksize: usize) -> Rerr {
    let key = kobj.as_ref();
    if key.len() != ksize {
        return errf!("{} check key {} size fail.", tip, hex::encode(key))
    }
    if key[0] == 0 || key[ksize-1] == 0 {
        return errf!("{} check key {} format fail.", tip, hex::encode(key))
    }
    Ok(())
}
