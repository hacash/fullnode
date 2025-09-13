


pub const CALL_EXTEND_ENV_DEFS: [(&'static str, u8); 3] = [
    ("_unknown_",               Value::TID_NIL),
    ("block_height",            Value::TID_U64),
    ("tx_main_address",         Value::TID_BYTES)
];


pub const CALL_EXTEND_FUNC_DEFS: [(&'static str, u8); 2] = [
    ("_unknown_",               Value::TID_NIL),
    ("check_signature",         Value::TID_BOOL)
];


