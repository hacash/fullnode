//! TxMessage / TxBlob actions.

use std::sync::Arc;

use base::{Action, ActionRef};
use field::{BytesW1, BytesW2, Decode, Encode, Uint2};
use sys::Ret;

use super::common::check_action_kind;

#[derive(Debug, Clone, base::ActionCodec)]
pub struct TxMessage {
    pub kind: Uint2,
    pub data: BytesW1,
}

#[derive(Debug, Clone, base::ActionCodec)]
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

base::impl_action! {
    TxMessage {
        name: "tx_message",
        scope: base::ActScope::GUARD,
        min_tx_type: 2,
        description: |_: &TxMessage| "Transaction message".to_owned(),
        execute: (self, _ctx) { Ok(vec![]) }
    }
}

base::impl_action! {
    TxBlob {
        name: "tx_blob",
        scope: base::ActScope::GUARD,
        min_tx_type: 2,
        description: |_: &TxBlob| "Transaction blob data".to_owned(),
        execute: (self, _ctx) { Ok(vec![]) }
    }
}

pub fn create_blob_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    match kind {
        TxMessage::KIND => decode_blob_action::<TxMessage>(buf),
        TxBlob::KIND => decode_blob_action::<TxBlob>(buf),
        _ => sys::decodef!("blob action kind {} not registered", kind),
    }
}

fn decode_blob_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}
