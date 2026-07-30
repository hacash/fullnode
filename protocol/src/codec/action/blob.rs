//! TxMessage / TxBlob actions.

use std::any::Any;
use std::sync::Arc;

use base::{ActOut, ActScope, Action, ActionRef, Context};
use field::{BytesW1, BytesW2, Encode, Reader, Uint2};
use sys::Ret;

#[derive(Debug, Clone)]
pub struct TxMessage {
    pub kind: Uint2,
    pub data: BytesW1,
}

#[derive(Debug, Clone)]
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

impl Encode for TxMessage {
    fn size(&self) -> usize {
        self.kind.size() + self.data.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.data.encode_to(out);
    }
}

impl Encode for TxBlob {
    fn size(&self) -> usize {
        self.kind.size() + self.data.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.data.encode_to(out);
    }
}

impl Action for TxMessage {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::GUARD
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn execute(&self, _ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for TxBlob {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::GUARD
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn execute(&self, _ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_blob_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind_field: Uint2 = r.read()?;
    if kind_field.uint() != kind {
        return sys::decodef!(
            "action kind mismatch: expected {} got {}",
            kind,
            kind_field.uint()
        );
    }
    match kind {
        TxMessage::KIND => {
            let data: BytesW1 = r.read()?;
            Ok((
                Arc::new(TxMessage {
                    kind: kind_field,
                    data,
                }),
                r.used(),
            ))
        }
        TxBlob::KIND => {
            let data: BytesW2 = r.read()?;
            Ok((
                Arc::new(TxBlob {
                    kind: kind_field,
                    data,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("blob action kind {} not registered", kind),
    }
}
