use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

// use concat_idents::concat_idents;

pub type Error = String;

pub mod leveldb;

include!{"panic.rs"}
include!{"stdout.rs"}
include!{"buffer.rs"}
include!{"string.rs"}
include!{"error.rs"}
include!{"number.rs"}
include!{"slice.rs"}
include!{"hex.rs"}
include!{"base64.rs"}
include!{"ini.rs"}
include!{"time.rs"}
include!{"account.rs"}
include!{"config.rs"}

