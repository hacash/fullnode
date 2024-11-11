/*
*
*/
pub fn block_hash_repeat(height: u64) -> i32 {
    let mut repeat = height / 50000 + 1;
    if repeat > 16 {
        repeat = 16;
    }
    return repeat as i32;
}

/*
*
*/
pub fn block_hash(height: u64, stuff: impl AsRef<[u8]>) -> [u8; H32S] {
    let repeat = block_hash_repeat(height);
    let reshash = calculate_hash(stuff); // sha3
    x16rs_hash(repeat, &reshash)
}
