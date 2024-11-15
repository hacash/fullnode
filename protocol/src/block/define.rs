
#[derive(Default, PartialEq, Copy, Clone)]
pub enum BlkOrigin {
    #[default] UNKNOW,
    REBUILD,
    SYNC,
    DISCOVER, // other find
    MINT, // mine miner find
}

