use crate::action::diamond::HacdMint;

pub const HASH_WIDTH: usize = 32;

#[derive(Clone)]
pub struct DiamondMiningResult {
    pub number: u32,
    pub nonce_start: u64,
    pub nonce_space: u64,
    pub nonce: u64,
    pub diamond_string: [u8; 16],
    pub success: Option<HacdMint>,
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

// The canonical implementation now lives in x16rs (single home of all
// x16rs algorithm implementations: C, Rust, OpenCL).
pub use x16rs::diamond_more_power;
