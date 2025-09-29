

const H32: usize = 32;

// height, intro bytes
pub type FnExtendBlockHasherFunc = fn(_: u64, _: &[u8]) -> [u8; H32];


fn default_block_hasher(_: u64, stuff: &[u8]) -> [u8; H32] {
    sys::calculate_hash(stuff)
}


pub static mut EXTEND_BLOCK_HASHER_FUNC: FnExtendBlockHasherFunc = default_block_hasher;


pub fn setup_block_hasher(f: FnExtendBlockHasherFunc) {
    unsafe {
        EXTEND_BLOCK_HASHER_FUNC = f;
    }
}



