

impl MemTxPool {
    pub const GROUP: usize = 2;

    pub const NORMAL: usize = 0;
    pub const DIAMINT: usize = 1;

    pub const TIPS: [&str; Self::GROUP] = [
        "normal", 
        "diamond mint"
    ];

}



///////////////////



fn check_group_id(wgi: usize) -> Rerr {
    if wgi > MemTxPool::GROUP {
        return errf!("tx pool group overflow")
    }
    Ok(())
}