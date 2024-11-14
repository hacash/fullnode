
#[derive(PartialEq, Copy, Clone)]
pub enum BlkOrigin {
    UNKNOW,
    SYNC,
    DISCOVER, // other find
    MINT, // mine miner find
}

