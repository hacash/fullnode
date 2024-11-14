
#[derive(PartialEq, Copy, Clone)]
pub enum TxOrigin {
    UNKNOW,
    SYNC,
    BROADCAST, // other find
    SUBMIT, // mine miner find
}

