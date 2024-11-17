
#[derive(Default, PartialEq, Copy, Clone)]
pub enum BlkOrigin {
    #[default] UNKNOWN,
    REBUILD,
    SYNC,
    DISCOVER, // other find
    MINT, // mine miner find
}

