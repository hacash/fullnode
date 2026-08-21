//! P2P primitives for the node protocol: [`msg`] constants/services/magic,
//! [`codec`] framing, [`handshake`] VERSION/VERACK, [`peer`] `RemotePeer`, [`source`] sync batches.

pub(crate) mod codec;
pub(crate) mod handshake;
pub(crate) mod msg;
pub(crate) mod peer;
pub(crate) mod source;
pub(crate) mod syncwire;
