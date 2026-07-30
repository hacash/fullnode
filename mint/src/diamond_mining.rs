use crate::action::diamond::DiamondMint;

pub const HASH_WIDTH: usize = 32;

#[derive(Clone)]
pub struct DiamondMiningResult {
    pub number: u32,
    pub nonce_start: u64,
    pub nonce_space: u64,
    pub nonce: u64,
    pub diamond_string: [u8; 16],
    pub success: Option<DiamondMint>,
    pub elapsed_secs: f64,
}

pub fn check_diamond_success(
    number: u32,
    first_hash: [u8; HASH_WIDTH],
    result_hash: [u8; HASH_WIDTH],
    diamond_string: [u8; 16],
) -> Option<[u8; 6]> {
    let diamond_name = x16rs::check_diamond_hash_result(diamond_string)?;
    x16rs::check_diamond_difficulty(number, &first_hash, &result_hash).then_some(diamond_name)
}

pub fn diamond_more_power(dst: &[u8], src: &[u8]) -> bool {
    for i in 0..16 {
        let (l, r) = (dst[i], src[i]);
        if l == b'0' && r != b'0' {
            return true;
        }
        if l != b'0' && r == b'0' {
            return false;
        }
        if l != b'0' && r != b'0' {
            return false;
        }
    }
    false
}
