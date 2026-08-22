//! TxMessage / TxBlob actions.

use field::{BytesW1, BytesW2, Uint2};

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full", blob)]
pub struct TxMessage {
    pub kind: Uint2,
    pub data: BytesW1,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full", blob)]
pub struct TxBlob {
    pub kind: Uint2,
    pub data: BytesW2,
}

impl TxMessage {
    pub const KIND: u16 = 0x0401;

    pub fn new(data: BytesW1) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            data,
        }
    }
}

impl TxBlob {
    pub const KIND: u16 = 0x0402;

    pub fn new(data: BytesW2) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            data,
        }
    }
}

base::impl_action_facts! {
    TxMessage {
        name: "tx_message",
        scope: base::ActScope::GUARD,
        min_tx_type: 2,
        description: |_: &TxMessage| "Transaction message".to_owned(),
    }
}

base::impl_action_facts! {
    TxBlob {
        name: "tx_blob",
        scope: base::ActScope::GUARD,
        min_tx_type: 2,
        description: |_: &TxBlob| "Transaction blob data".to_owned(),
    }
}
