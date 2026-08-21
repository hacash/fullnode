//! `sys` — foundational primitives shared by all crates: error system, `Bytes`,
//! `Waiter`, `Account`, hash/hex/base64/time/ini utilities. Spine: `sys -> field -> base -> {protocol, chain, node, server, api, db}`.

mod account;
mod base64;
mod bytes;
mod error;
mod hash;
mod hex;
mod ini;
mod r#match;
mod string;
mod time;
mod waiter;

pub use account::*;
pub use base64::*;
pub use bytes::*;
pub use error::*;
pub use hash::*;
pub use hex::*;
pub use ini::*;
pub use string::*;
pub use time::*;
pub use waiter::*;

#[macro_export]
macro_rules! flush {
    ($($param:expr),+) => ({
        use std::io::Write;
        print!($($param),+);
        let _ = std::io::stdout().flush();
    })
}
