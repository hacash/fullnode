//! `sys` -- foundational primitives shared by all crates.
//!
//! - `Error { Decode, Revert, Fault }` + `Ret<T>` / `Rerr`: unified error system
//! - `Bytes`: refcounted byte buffer (`Arc<Vec<u8>>` + slice range)
//! - `Waiter`: graceful-shutdown coordination token (sync + async + barrier)
//! - `Account`: secp256k1 keypair + address generation
//! - hash/hex/base64/time/ini/string: utility functions
//!
//! Dependency spine: `sys -> field -> base -> {protocol, chain, node, server, api, db}`;
//! `protocol -> {vm, mint}`; `app` assembles all. (`chain` does not depend on `protocol`.)

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
