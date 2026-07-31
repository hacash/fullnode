use sys::Rerr;

/// Wire type for new-transaction push (`peer.send_msg(MSG_TX_SUBMIT, body)`).
///
/// Shared by `node` (frame codec) and `mint` (consensus `on_p2p_connect`).
/// Kept in `base` so neither crate depends on the other.
pub const MSG_TX_SUBMIT: u16 = 7;

pub trait Peer: Send + Sync {
    fn id(&self) -> String;
    /// Human-readable name announced during the P2P handshake. Fall back to
    /// the stable connection id for implementations that do not expose one.
    fn name(&self) -> String {
        self.id()
    }
    fn send_msg(&self, _ty: u16, _body: Vec<u8>) -> Rerr {
        Ok(())
    }
    fn disconnect(&self) {}
    /// Wire protocol version this peer speaks: `1` = legacy (v1),
    /// `2` = v2 (crc32c frame, flat u16 namespace). Default `1` for
    /// backwards compat with any `Peer` impl that predates the dual
    /// protocol split. Used to pick the correct BLOCKS wire layout
    /// (v2 carries a block count field that v1 lacks) and similar
    /// version-aware encodings.
    fn protocol_version(&self) -> u8 {
        1
    }
}
