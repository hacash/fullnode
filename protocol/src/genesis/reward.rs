
const BLOCK_REWARD_SEC_TWO: usize = 66;
const BLOCK_REWARD_STEP_BLOCK: u64 = 10_0000;
const BLOCK_REWARD_DEF_LIST: [u8; BLOCK_REWARD_SEC_TWO] = [
    1, 1, 2, 3, 5, 8,
    8,8,8,8,8,8,8,8,8,8,
    5,5,5,5,5,5,5,5,5,5,
    3,3,3,3,3,3,3,3,3,3,
    2,2,2,2,2,2,2,2,2,2,
    1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,
];

/*
* Currency release algorithm: 22 million in the first 66 years
*/
pub fn block_reward_number(block_height: u64) -> u8 {
    let stp = BLOCK_REWARD_STEP_BLOCK;
    let lis = BLOCK_REWARD_DEF_LIST;
    let sct = BLOCK_REWARD_SEC_TWO as u64;
    let curstp = block_height / stp;
    if curstp >= sct {
        return 1 // after 66 years
    }
    // before 66 years
    lis[curstp as usize]
}

pub fn block_reward(block_height: u64) -> Amount {
	let num = block_reward_number(block_height);
	return Amount::mei(num as u64)
}

pub fn cumulative_block_reward(block_height: u64) -> u64 {
    let stp = BLOCK_REWARD_STEP_BLOCK;
    let mut cbhei = block_height + 1;
    let mut ttcoin = 0u64;
    for (_, v) in BLOCK_REWARD_DEF_LIST.iter().enumerate() {
        let v = *v as u64;
        if cbhei < stp {
            ttcoin += cbhei * v;
            break // finish
        }
        ttcoin += stp * v;
        cbhei -= stp; // next
    }
    ttcoin - 1
}



/////////////////////



 /*
 pub fn block_reward_number(block_height: u64) -> u8 {
    let part1 = [1u8, 1, 2, 3, 5, 8];
    let part2 = [8u8, 5, 3, 2, 1, 1];
    let part3 = 1u8;
    let tbn1: u64 =  10_0000;
    let tbn2: u64 = 100_0000;
    let spx1: u64 = part1.len() as u64 * tbn1;
    let spx2: u64 = part2.len() as u64 * tbn2 + spx1;
    let mut basenum = block_height;
    if block_height <= spx1 {
        return part1[(basenum/tbn1) as usize]
    }
    if block_height <= spx2 {
        basenum -= spx1;
        return part2[(basenum/tbn2) as usize]
    }
    return part3
}
*/
