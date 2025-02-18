use std::path::Path;
use std::sync::Mutex;


include!{"interface.rs"}


#[cfg(feature = "db-sled")]
include!{"disk_sled.rs"}


#[cfg(feature = "db-rusty-leveldb")]
include!{"disk_rusty_leveldb.rs"}


#[cfg(feature = "db-leveldb-sys")]
include!{"disk_leveldb_sys.rs"}


include!{"memkv.rs"}
