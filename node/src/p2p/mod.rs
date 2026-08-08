//! P2P primitives for the node protocol.
//!
//! - [`msg`]: message constants, services bits, and magic number.
//! - [`codec`]: frame codec (length + u8 ty + crc32c checksum).
//! - [`handshake`]: VERSION/VERACK handshake.
//! - [`peer`]: `RemotePeer` and its single writer queue.
//! - [`source`]: sync block batch source.

pub(crate) mod codec;
pub(crate) mod handshake;
pub(crate) mod msg;
pub(crate) mod peer;
pub(crate) mod source;
pub(crate) mod syncwire;
