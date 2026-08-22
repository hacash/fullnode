// Strict PBUF/PBUFL length-prefixed blob decoding (matches parser; rejects trailing junk).

use crate::ir::IRNodeParams;

pub(crate) fn decode_pbuf_params_borrow<'b>(params: &'b IRNodeParams) -> Option<&'b [u8]> {
    let para = &params.para;
    let header_len = match params.inst {
        crate::rt::Bytecode::PBUF => 1,
        crate::rt::Bytecode::PBUFL => 2,
        _ => return None,
    };
    if para.len() < header_len {
        return None;
    }
    let len = match header_len {
        1 => para[0] as usize,
        2 => u16::from_be_bytes([para[0], para[1]]) as usize,
        _ => return None,
    };
    if para.len() != header_len + len {
        return None;
    }
    Some(&para[header_len..])
}

pub(crate) fn decode_pbuf_payload(params: &IRNodeParams) -> Option<Vec<u8>> {
    decode_pbuf_params_borrow(params).map(|s| s.to_vec())
}
