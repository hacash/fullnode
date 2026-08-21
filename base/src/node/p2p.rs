use sys::Rerr;

/// Wire type for new-transaction push (`peer.send_msg(MSG_TX_SUBMIT, body)`), shared by
/// `node` (frame codec) and `mint` (`on_p2p_connect`) — kept in `base` to avoid a dependency.
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
}
