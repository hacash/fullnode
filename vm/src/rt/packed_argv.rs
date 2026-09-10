use crate::value::Value;

/// Packed NTFUNC argv payload size for `nt_bytes` (frozen).
/// Does not use `Value::val_size()`, `REF_DUP_SIZE`, or `stack_move_items`.
pub fn packed_payload_bytes(v: &Value) -> usize {
    match v {
        Value::Nil => 0,
        Value::Bool(_) | Value::U8(_) => 1,
        Value::U16(_) => 2,
        Value::U32(_) => 4,
        Value::U64(_) => 8,
        Value::U128(_) => 16,
        Value::Bytes(b) => b.len(),
        Value::Address(_) => field::Address::SIZE,
        Value::Tuple(t) => {
            let mut sum = 0usize;
            for item in t.as_slice() {
                sum = sum.saturating_add(packed_payload_bytes(item));
            }
            sum
        }
        Value::Compo(c) => {
            if let Ok(list) = c.list_ref() {
                let mut sum = 0usize;
                for item in &*list {
                    sum = sum.saturating_add(packed_payload_bytes(item));
                }
                sum
            } else if let Ok(map) = c.map_ref() {
                let mut sum = 0usize;
                for item in map.values() {
                    sum = sum.saturating_add(packed_payload_bytes(item));
                }
                sum
            } else {
                0
            }
        }
        Value::Handle(_) => 0,
    }
}

/// Packed NTFUNC argv item count for `compo_items_read` (frozen).
pub fn packed_item_count(v: &Value) -> usize {
    match v {
        Value::Tuple(t) => t.len(),
        Value::Compo(c) if c.is_list() => c.list_ref().map(|list| list.len()).unwrap_or(1),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{CompoItem, TupleItem, Value};
    use std::collections::VecDeque;

    fn list(items: Vec<Value>) -> Value {
        Value::Compo(CompoItem::list(VecDeque::from(items)).unwrap())
    }

    #[test]
    fn packed_payload_bytes_scalars_and_patches_list() {
        assert_eq!(packed_payload_bytes(&Value::Nil), 0);
        assert_eq!(packed_payload_bytes(&Value::Bool(true)), 1);
        assert_eq!(packed_payload_bytes(&Value::U8(1)), 1);
        assert_eq!(packed_payload_bytes(&Value::U16(1)), 2);
        assert_eq!(packed_payload_bytes(&Value::U32(1)), 4);
        assert_eq!(packed_payload_bytes(&Value::U64(1)), 8);
        assert_eq!(packed_payload_bytes(&Value::U128(1)), 16);
        assert_eq!(packed_payload_bytes(&Value::bytes(vec![1, 2, 3])), 3);
        assert_eq!(
            packed_payload_bytes(&Value::Address(field::Address::from([0u8; 21]))),
            21
        );
        let argv = list(vec![Value::U8(0), Value::U8(1), Value::U8(1)]);
        assert_eq!(packed_payload_bytes(&argv), 3);
        assert_eq!(packed_item_count(&argv), 3);
        assert_eq!(packed_item_count(&Value::U8(1)), 1);
        let tup = Value::Tuple(TupleItem::new(vec![Value::U8(1), Value::U16(2)]).unwrap());
        assert_eq!(packed_payload_bytes(&tup), 3);
        assert_eq!(packed_item_count(&tup), 2);
        let nils = list(vec![Value::Nil, Value::Nil, Value::Nil]);
        assert_eq!(packed_payload_bytes(&nils), 0);
        assert_eq!(packed_item_count(&nils), 3);
    }
}
