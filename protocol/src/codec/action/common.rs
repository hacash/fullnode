//! Compatibility helpers while standard actions use the field-level codec.

use field::{AddrOrPtr, Decode, Encode};
use sys::Ret;

pub(super) fn addr_or_ptr_size(ptr: &AddrOrPtr) -> usize {
    ptr.size()
}

pub(super) fn encode_addr_or_ptr(ptr: &AddrOrPtr, out: &mut Vec<u8>) {
    ptr.encode_to(out)
}

pub(super) fn decode_addr_or_ptr(buf: &[u8]) -> Ret<(AddrOrPtr, usize)> {
    AddrOrPtr::decode(buf)
}
