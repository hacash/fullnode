
#[derive(Default, PartialEq, Copy, Clone)]
pub enum TxOrigin {
    #[default] UNKNOWN,
    SYNC,
    BROADCAST, // other find
    SUBMIT, // mine miner find
}

