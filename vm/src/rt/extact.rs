
use protocol::action::*;
use ValueTy::*;

type ExtDefTy = (u8, &'static str, ValueTy);

const CALL_EXTEND_UNKNOWN_NAME: &'static str = "__unknown__";

pub const CALL_EXTEND_ACTION_DEFS: [ExtDefTy; 2] = [
    (HacToTrs::KIND as u8, "transfer_hac_to",    Nil),
    (SatToTrs::KIND as u8, "transfer_sat_to",    Nil),
];


pub const CALL_EXTEND_ENV_DEFS: [ExtDefTy; 2] = [
    (1, "block_height",            U64),
    (2, "tx_main_address",         Address)
];


pub const CALL_EXTEND_FUNC_DEFS: [ExtDefTy; 1] = [
    (1, "check_signature",         Bool)
];


pub fn search_ext_by_id<'a>(id: u8, exts: &'a[ExtDefTy]) -> Option<&'a ExtDefTy> {
    for a in exts {
        if a.0 == id {
            return Some(a)
        }
    }
    // not find
    None
}

pub fn search_ext_name_by_id(id: u8, exts: &[ExtDefTy]) -> &'static str {
     match search_ext_by_id(id, exts) {
        Some(a) => a.1,
        _ => CALL_EXTEND_UNKNOWN_NAME
    }
}