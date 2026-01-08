use std::path::*;


use protocol::interface::*;


/*****************************/

#[cfg(all(feature = "db-sled", feature = "db-rusty-leveldb"))]
compile_error!("db cannot be enabled at the same time");

#[cfg(all(feature = "db-sled", feature = "db-leveldb-sys"))]
compile_error!("db cannot be enabled at the same time");

#[cfg(all(feature = "db-sled", feature = "db-rocksdb"))]
compile_error!("db cannot be enabled at the same time");

#[cfg(all(feature = "db-leveldb-sys", feature = "db-rusty-leveldb"))]
compile_error!("db cannot be enabled at the same time");

#[cfg(all(feature = "db-leveldb-sys", feature = "db-rocksdb"))]
compile_error!("db cannot be enabled at the same time");

/*****************************/

#[cfg(feature = "db-sled")]
include!{"disk_sled.rs"}

#[cfg(feature = "db-rusty-leveldb")]
include!{"disk_rusty_leveldb.rs"}

#[cfg(feature = "db-leveldb-sys")]
include!{"disk_leveldb_sys.rs"}

#[cfg(feature = "db-rocksdb")]
include!{"disk_rocksdb.rs"}

/*****************************/

include!{"batch.rs"}


