//! Block reward curve (moved from mint's minter; pure constant table, no consensus-state dependency).

pub const BLOCK_REWARD_STEP_BLOCK: u64 = 100_000;
pub const BLOCK_REWARD_DEF_LIST: [u8; 66] = [
    1, 1, 2, 3, 5, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1,
];

pub fn block_reward_number(block_height: u64) -> u8 {
    let curstp = block_height / BLOCK_REWARD_STEP_BLOCK;
    if curstp >= BLOCK_REWARD_DEF_LIST.len() as u64 {
        return 1;
    }
    BLOCK_REWARD_DEF_LIST[curstp as usize]
}
