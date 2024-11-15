
#[derive(Default, PartialEq, Copy, Clone)]
pub enum BlkOrigin {
    #[default] UNKNOW,
    SYNC,
    DISCOVER, // other find
    MINT, // mine miner find
}

