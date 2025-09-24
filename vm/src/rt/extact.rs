
use protocol::action::*;
use ValueTy::*;

pub const CALL_EXTEND_ACTION_DEFS: [(u8, &'static str, ValueTy); 1] = [
    (HacToTrs::KIND as u8, "transfer_hac_to",    Nil),
];


pub const CALL_EXTEND_ENV_DEFS: [(u8, &'static str, ValueTy); 2] = [
    (1, "block_height",            U64),
    (2, "tx_main_address",         Addr)
];


pub const CALL_EXTEND_FUNC_DEFS: [(u8, &'static str, ValueTy); 1] = [
    (1, "check_signature",         Bool)
];

