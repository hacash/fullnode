
#[derive(Default, PartialEq, Copy, Clone)]
pub enum TxOrigin {
    #[default] UNKNOW,
    SYNC,
    BROADCAST, // other find
    SUBMIT, // mine miner find
}

