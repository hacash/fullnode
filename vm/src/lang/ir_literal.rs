// Shared literal inference and recovery for parser (`Syntax`) and decompiler (`Formater`).

use crate::rt::Bytecode::*;
use crate::value::{Value, ValueTy};

/// Static type of an IR subtree when it is a literal or an explicit cast wrapper.
pub(crate) fn ir_node_effective_ty(node: &dyn IRNode) -> Option<ValueTy> {
    if let Some(ty) = ir_literal_ty(node) {
        return Some(ty);
    }
    if let Some(single) = node.as_any().downcast_ref::<IRNodeSingle>() {
        return match single.inst {
            CU8 => Some(ValueTy::U8),
            CU16 => Some(ValueTy::U16),
            CU32 => Some(ValueTy::U32),
            CU64 => Some(ValueTy::U64),
            CU128 => Some(ValueTy::U128),
            CBYTES => Some(ValueTy::Bytes),
            _ => None,
        };
    }
    if let Some(single) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
        if single.inst == CTO {
            return ValueTy::build(single.para).ok();
        }
    }
    None
}

pub(crate) fn ir_literal_ty(node: &dyn IRNode) -> Option<ValueTy> {
    if let Some(leaf) = node.as_any().downcast_ref::<IRNodeLeaf>() {
        return match leaf.inst {
            P0 | P1 | P2 | P3 => Some(ValueTy::U8),
            PTRUE | PFALSE => Some(ValueTy::Bool),
            PNBUF => Some(ValueTy::Bytes),
            _ => None,
        };
    }
    if let Some(param1) = node.as_any().downcast_ref::<IRNodeParam1>() {
        if param1.inst == PU8 {
            return Some(ValueTy::U8);
        }
    }
    if let Some(param2) = node.as_any().downcast_ref::<IRNodeParam2>() {
        if param2.inst == PU16 {
            return Some(ValueTy::U16);
        }
    }
    if let Some(params) = node.as_any().downcast_ref::<IRNodeParams>() {
        return match params.inst {
            PBUF | PBUFL => Some(ValueTy::Bytes),
            _ => None,
        };
    }
    if let Some(single) = node.as_any().downcast_ref::<IRNodeSingle>() {
        return match single.inst {
            CU8 => Some(ValueTy::U8),
            CU16 => Some(ValueTy::U16),
            CU32 => Some(ValueTy::U32),
            CU64 => Some(ValueTy::U64),
            CU128 => Some(ValueTy::U128),
            CBYTES => Some(ValueTy::Bytes),
            _ => None,
        };
    }
    if let Some(single) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
        if single.inst == CTO {
            return ValueTy::build(single.para).ok();
        }
    }
    None
}

pub(crate) fn ir_literal_value(node: &dyn IRNode) -> Ret<Option<Value>> {
    if let Some(leaf) = node.as_any().downcast_ref::<IRNodeLeaf>() {
        return Ok(Some(match leaf.inst {
            P0 => Value::U8(0),
            P1 => Value::U8(1),
            P2 => Value::U8(2),
            P3 => Value::U8(3),
            PNIL => Value::Nil,
            PTRUE => Value::Bool(true),
            PFALSE => Value::Bool(false),
            PNBUF => Value::Bytes(vec![]),
            _ => return Ok(None),
        }));
    }
    if let Some(param1) = node.as_any().downcast_ref::<IRNodeParam1>() {
        return Ok(maybe!(
            param1.inst == PU8,
            Some(Value::U8(param1.para)),
            None
        ));
    }
    if let Some(param2) = node.as_any().downcast_ref::<IRNodeParam2>() {
        return Ok(maybe!(
            param2.inst == PU16,
            Some(Value::U16(u16::from_be_bytes(param2.para))),
            None
        ));
    }
    if let Some(params) = node.as_any().downcast_ref::<IRNodeParams>() {
        return Ok(decode_pbuf_payload(params).map(Value::Bytes));
    }
    if let Some(single) = node.as_any().downcast_ref::<IRNodeSingle>() {
        let Some(mut value) = ir_literal_value(&*single.subx)? else {
            return Ok(None);
        };
        let cast = match single.inst {
            CU8 => value.cast_u8(),
            CU16 => value.cast_u16(),
            CU32 => value.cast_u32(),
            CU64 => value.cast_u64(),
            CU128 => value.cast_u128(),
            CBYTES => value.cast_bytes(),
            _ => return Ok(None),
        };
        if cast.is_err() {
            return Ok(None);
        }
        return Ok(Some(value));
    }
    if let Some(single) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
        if single.inst != CTO {
            return Ok(None);
        }
        let Some(mut value) = ir_literal_value(&*single.subx)? else {
            return Ok(None);
        };
        if value.cast_to(single.para).is_err() {
            return Ok(None);
        }
        return Ok(Some(value));
    }
    Ok(None)
}

#[derive(Clone)]
pub(crate) struct IrRecoveredLiteral {
    pub text: String,
    pub ty: Option<ValueTy>,
}

impl IrRecoveredLiteral {
    fn numeric(text: String, ty: ValueTy) -> Self {
        Self {
            text,
            ty: Some(ty),
        }
    }

    fn text_lit(text: String) -> Self {
        Self {
            text,
            ty: Some(ValueTy::Bytes),
        }
    }

    fn addr(text: String) -> Self {
        Self {
            text,
            ty: Some(ValueTy::Address),
        }
    }
}

pub(crate) fn escape_lang_string_literal(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('"', "\\\"")
}

pub(crate) fn ascii_decode_show_string(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return Some(String::new());
    }
    if data
        .iter()
        .all(|&b| (b >= 0x20 && b <= 0x7E) || b == 0x0a || b == 0x0d || b == 0x09)
    {
        std::str::from_utf8(data).ok().map(|s| s.to_string())
    } else {
        None
    }
}

pub(crate) fn ir_recover_quoted_bytes_literal(data: &[u8]) -> Option<String> {
    let raw = ascii_decode_show_string(data)?;
    Some(format!("\"{}\"", escape_lang_string_literal(&raw)))
}

/// Sole conversion from scalar/container literals to decompiler text (`IrRecoveredLiteral`).
/// All recovery paths must end here after constructing a suitable [`Value`].
fn ir_recovered_literal_from_value(v: Value) -> Option<IrRecoveredLiteral> {
    use Value::*;
    match v {
        // Historical `literal_from_node` did not recover PNIL / empty bytes leaves.
        Nil => None,
        Bool(b) => Some(IrRecoveredLiteral::numeric(
            if b { "true" } else { "false" }.to_string(),
            ValueTy::Bool,
        )),
        U8(n) => Some(IrRecoveredLiteral::numeric(n.to_string(), ValueTy::U8)),
        U16(n) => Some(IrRecoveredLiteral::numeric(n.to_string(), ValueTy::U16)),
        U32(n) => Some(IrRecoveredLiteral::numeric(n.to_string(), ValueTy::U32)),
        U64(n) => Some(IrRecoveredLiteral::numeric(n.to_string(), ValueTy::U64)),
        U128(n) => Some(IrRecoveredLiteral::numeric(n.to_string(), ValueTy::U128)),
        Bytes(bs) => {
            if bs.is_empty() {
                return None;
            }
            let text = ir_recover_quoted_bytes_literal(&bs)?;
            Some(IrRecoveredLiteral::text_lit(text))
        }
        Address(a) => Some(IrRecoveredLiteral::addr(a.to_readable())),
        _ => None,
    }
}

fn decode_be_uint_payload_u128(data: &[u8]) -> Option<u128> {
    if data.len() > 16 {
        return None;
    }
    let mut value = 0u128;
    for &b in data {
        value = (value << 8) | b as u128;
    }
    Some(value)
}

fn u128_to_scalar_value(n: u128, ty: ValueTy) -> Option<Value> {
    use ValueTy::*;
    match ty {
        U8 => u8::try_from(n).ok().map(Value::U8),
        U16 => u16::try_from(n).ok().map(Value::U16),
        U32 => u32::try_from(n).ok().map(Value::U32),
        U64 => u64::try_from(n).ok().map(Value::U64),
        U128 => Some(Value::U128(n)),
        _ => None,
    }
}

/// Operand IR for `CU8`…`CU128` without applying the outer cast (matches legacy `numeric_literal_from`).
/// Integer PBUF payloads are narrowed to `cast_target` width so `Value::ty` matches the cast for recovery.
fn ir_cast_operand_recovery_value(inner: &dyn IRNode, cast_target: ValueTy) -> Option<Value> {
    if let Some(leaf) = inner.as_any().downcast_ref::<IRNodeLeaf>() {
        match leaf.inst {
            P0 => return Some(Value::U8(0)),
            P1 => return Some(Value::U8(1)),
            P2 => return Some(Value::U8(2)),
            P3 => return Some(Value::U8(3)),
            PTRUE => return Some(Value::Bool(true)),
            PFALSE => return Some(Value::Bool(false)),
            _ => {}
        }
    }
    if let Some(param1) = inner.as_any().downcast_ref::<IRNodeParam1>() {
        if param1.inst == PU8 {
            return Some(Value::U8(param1.para));
        }
    }
    if let Some(param2) = inner.as_any().downcast_ref::<IRNodeParam2>() {
        if param2.inst == PU16 {
            return Some(Value::U16(u16::from_be_bytes(param2.para)));
        }
    }
    if let Some(params) = inner.as_any().downcast_ref::<IRNodeParams>() {
        if matches!(params.inst, PBUF | PBUFL) {
            let data = decode_pbuf_params_borrow(params)?;
            let n = decode_be_uint_payload_u128(data)?;
            return u128_to_scalar_value(n, cast_target);
        }
    }
    None
}

pub(crate) fn ir_recovered_literal(node: &dyn IRNode) -> Option<IrRecoveredLiteral> {
    use crate::rt::Bytecode::*;

    if let Some(single) = node.as_any().downcast_ref::<IRNodeSingle>() {
        let cast_target = match single.inst {
            CU8 => Some(ValueTy::U8),
            CU16 => Some(ValueTy::U16),
            CU32 => Some(ValueTy::U32),
            CU64 => Some(ValueTy::U64),
            CU128 => Some(ValueTy::U128),
            _ => None,
        };
        if let Some(ct) = cast_target {
            let v = ir_cast_operand_recovery_value(&*single.subx, ct)?;
            if v.ty() != ct {
                return None;
            }
            return ir_recovered_literal_from_value(v);
        }
    }

    let v = ir_literal_value(node).ok()??;
    ir_recovered_literal_from_value(v)
}
