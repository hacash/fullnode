//! P2P primitives for both v2 and legacy (v1) protocols.
//!
//! - [`msg`]: shared + v2 message constants, services bits, magic numbers.
//! - [`codec`]: v2 frame codec (length + u8 ty + crc32c checksum).
//! - [`handshake`]: v2 VERSION/VERACK handshake.
//! - [`legacy`]: v1 codec, handshake, and constants (mainnet-compatible).
//! - [`peer`]: `RemotePeer` (carries its `ProtocolVersion` for version-aware
//!   message handling, e.g. BLOCKS count field).
//! - [`source`]: sync block batch source.

pub(crate) mod codec;
pub(crate) mod handshake;
pub(crate) mod legacy;
pub(crate) mod msg;
pub(crate) mod peer;
pub(crate) mod source;
pub(crate) mod syncwire;
