//! Compatibility helpers while standard actions use the field-level codec.

use field::{AddrOrPtr, Decode, Encode, Uint2};
use sys::Ret;

pub(super) fn check_action_kind(kind: u16, buf: &[u8]) -> Ret<()> {
    let (wire_kind, _) = Uint2::decode(buf)?;
    if wire_kind.uint() != kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            kind,
            wire_kind.uint()
        );
    }
    Ok(())
}

pub(super) fn addr_or_ptr_size(ptr: &AddrOrPtr) -> usize {
    ptr.size()
}

pub(super) fn addr_or_ptr_readable(ptr: &AddrOrPtr) -> String {
    match ptr {
        AddrOrPtr::Addr(addr) => addr.to_readable(),
        AddrOrPtr::Ptr(index) => format!("<address pointer {}>", index),
    }
}

pub(super) fn encode_addr_or_ptr(ptr: &AddrOrPtr, out: &mut Vec<u8>) {
    ptr.encode_to(out)
}

pub(super) fn decode_addr_or_ptr(buf: &[u8]) -> Ret<(AddrOrPtr, usize)> {
    AddrOrPtr::decode(buf)
}
