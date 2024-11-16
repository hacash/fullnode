


pub const TXPOOL_GROUP_MAX_SIZE: usize = 2;

pub const TXPOOL_GROUP_NORMAL: usize = 0;
pub const TXPOOL_GROUP_DIAMOND_MINT: usize = 1;

pub const TXPOOL_GROUP_TIPS: [&str; TXPOOL_GROUP_MAX_SIZE] = [
    "normal", 
    "diamond mint"
];



///////////////////



fn check_group_id(wgi: usize) -> Rerr {
    if wgi > TXPOOL_GROUP_MAX_SIZE {
        return errf!("tx pool group overflow")
    }
    Ok(())
}