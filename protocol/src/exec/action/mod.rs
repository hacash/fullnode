//! Action execute bodies (`execute` feature only). Wire codecs stay in
//! `protocol::codec::action`; this module only attaches `ActionExecute` impls so codec files do not compile `Context`/ledger mutations.

mod ast;
mod blob;
mod envfunc;
mod guard;
mod tex;
mod transfer;
