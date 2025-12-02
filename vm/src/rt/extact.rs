
use protocol::action::*;
use super::action::*;
use ValueTy::*;

type ExtDefTy = (u8, &'static str, ValueTy);

const CALL_EXTEND_UNKNOWN_NAME: &'static str = "__unknown__";


/********************************************/


pub const CALL_EXTEND_ACTION_DEFS: [ExtDefTy; 4] = [
    (HacToTrs::IDX,   "transfer_hac_to",      Nil),
    (SatToTrs::IDX,   "transfer_sat_to",      Nil),
    (HacFromTrs::IDX, "transfer_hac_from",    Nil),
    (SatFromTrs::IDX, "transfer_sat_from",    Nil),
];


pub const CALL_EXTEND_ENV_DEFS: [ExtDefTy; 2] = [
    (EnvHeight::IDX,   "block_height",             U64),
    (EnvMainAddr::IDX, "tx_main_address", ValueTy::Address),
];


pub const CALL_EXTEND_FUNC_DEFS: [ExtDefTy; 2] = [
    (FuncCheckSign::IDX, "check_signature",         Bool),
    (FuncBalance::IDX,   "balance",                 Bytes),
];


/********************************************/


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