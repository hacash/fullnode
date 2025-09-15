


pub const CALL_EXTEND_ENV_DEFS: [(&'static str, ValueTy); 3] = [
    ("_unknown_",               ValueTy::Nil),
    ("block_height",            ValueTy::U64),
    ("tx_main_address",         ValueTy::Bytes)
];


pub const CALL_EXTEND_FUNC_DEFS: [(&'static str, ValueTy); 2] = [
    ("_unknown_",               ValueTy::Nil),
    ("check_signature",         ValueTy::Bool)
];


