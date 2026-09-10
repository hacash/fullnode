use super::*;

/// Canonical `patch_set` bytes. Count/overlap are checked here so encode
/// failures (insert too long) still surface as `NativeFuncError`, not storage codes.
fn encode_patches_bytes(records: &[(u16, u16, Vec<u8>)]) -> VmrtRes<Vec<u8>> {
    let recs: Vec<(u16, u16, &[u8])> = records
        .iter()
        .map(|(offset, delete_len, insert)| (*offset, *delete_len, insert.as_slice()))
        .collect();
    crate::state::patch::encode_patch_set(&recs).map_err(|e| ItrErr(NativeFuncError, e.1))
}

fn patches_u16(v: &Value, label: &str) -> VmrtRes<u16> {
    if !v.is_uint() {
        return itr_err_fmt!(NativeFuncError, "patches {} must be an unsigned int", label);
    }
    let n = v.extract_u128()?;
    if n > u16::MAX as u128 {
        return itr_err_fmt!(NativeFuncError, "patches {} exceeds u16::MAX", label);
    }
    Ok(n as u16)
}

fn patches_insert(v: &Value) -> VmrtRes<Vec<u8>> {
    let bytes = match v {
        Value::Bytes(b) => b.clone(),
        Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_)
        | Value::U128(_)
        | Value::Address(_) => v
            .scalar_bytes()
            .expect("uint and address have scalar bytes"),
        _ => {
            return itr_err_fmt!(NativeFuncError, "patches insert type invalid");
        }
    };
    if bytes.len() > u16::MAX as usize {
        return itr_err_fmt!(NativeFuncError, "patches insert exceeds u16::MAX length");
    }
    Ok(bytes)
}

pub(super) fn patches(_height: u64, argv: Value) -> VmrtRes<Value> {
    let Value::Compo(compo) = argv else {
        return itr_err_fmt!(NativeFuncError, "patches expects a list");
    };
    if !compo.is_list() {
        return itr_err_fmt!(NativeFuncError, "patches expects a list");
    }
    let items = compo.list_ref()?;
    let len = items.len();
    if len % 3 != 0 {
        return itr_err_fmt!(
            NativeFuncError,
            "patches list length {} is not a multiple of 3",
            len
        );
    }
    let count = len / 3;
    if !(1..=16).contains(&count) {
        return itr_err_fmt!(
            NativeFuncError,
            "patches count must be in 1..=16, got {}",
            count
        );
    }

    let mut records = Vec::with_capacity(count);
    let mut prev_offset: Option<u16> = None;
    let mut prev_delete: u16 = 0;
    for i in 0..count {
        let offset = patches_u16(&items[i * 3], "offset")?;
        let delete_len = patches_u16(&items[i * 3 + 1], "delete_len")?;
        let insert = patches_insert(&items[i * 3 + 2])?;
        if let Some(prev) = prev_offset {
            let prev_end = (prev as u32) + (prev_delete as u32);
            if (offset as u32) <= (prev as u32) || (offset as u32) < prev_end {
                return itr_err_fmt!(
                    NativeFuncError,
                    "patches offsets must be strictly increasing and non-overlapping"
                );
            }
        }
        prev_offset = Some(offset);
        prev_delete = delete_len;
        records.push((offset, delete_len, insert));
    }
    Ok(Value::bytes(encode_patches_bytes(&records)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::NativeArgvPack;
    use std::collections::{BTreeMap, VecDeque};

    fn list(items: Vec<Value>) -> Value {
        Value::Compo(CompoItem::list(VecDeque::from(items)).unwrap())
    }

    fn assert_native_func_error<T: std::fmt::Debug>(r: VmrtRes<T>) {
        match r {
            Err(ItrErr(NativeFuncError, _)) => {}
            other => panic!("expected NativeFuncError, got {:?}", other),
        }
    }

    fn call_patches(argv: Value) -> VmrtRes<Value> {
        NativeFunc::call_packed(0, NativeFunc::patches as u8, argv).map(|(v, gas)| {
            assert_eq!(gas, 24);
            v
        })
    }

    #[test]
    fn argv_pack_sha2_concat_and_patches_packed() {
        assert_eq!(
            NativeFunc::argv_pack(NativeFunc::sha2 as u8).unwrap(),
            NativeArgvPack::Concat
        );
        assert_eq!(
            NativeFunc::argv_pack(NativeFunc::patches as u8).unwrap(),
            NativeArgvPack::Packed
        );
        assert_eq!(
            NativeFunc::argv_pack(NativeFunc::pack_asset as u8).unwrap(),
            NativeArgvPack::Concat
        );
        assert_eq!(
            NativeFunc::argv_pack(NativeFunc::verify_signature as u8).unwrap(),
            NativeArgvPack::Concat
        );
        assert_eq!(NativeFunc::argv_len(NativeFunc::patches as u8), Some(1));
    }

    #[test]
    fn patches_pause_matches_hand_encode() {
        let pause = 1u8;
        let argv = list(vec![Value::U8(0), Value::U8(1), Value::U8(pause)]);
        let got = call_patches(argv).unwrap();
        let mut expect = vec![1u8];
        expect.extend_from_slice(&0u16.to_be_bytes());
        expect.extend_from_slice(&1u16.to_be_bytes());
        expect.extend_from_slice(&1u16.to_be_bytes());
        expect.push(pause);
        assert_eq!(got, Value::bytes(expect));
    }

    #[test]
    fn patches_u64_insert_and_empty_bytes() {
        let argv = list(vec![
            Value::U8(0),
            Value::U8(0),
            Value::U64(0x0102_0304_0506_0708),
        ]);
        let got = call_patches(argv).unwrap();
        let Value::Bytes(buf) = got else {
            panic!("expected bytes");
        };
        assert_eq!(buf[0], 1);
        assert_eq!(&buf[1..3], &0u16.to_be_bytes());
        assert_eq!(&buf[3..5], &0u16.to_be_bytes());
        assert_eq!(&buf[5..7], &8u16.to_be_bytes());
        assert_eq!(&buf[7..], &[1, 2, 3, 4, 5, 6, 7, 8]);

        let argv = list(vec![Value::U8(0), Value::U8(1), Value::bytes(vec![])]);
        let got = call_patches(argv).unwrap();
        let Value::Bytes(buf) = got else {
            panic!("expected bytes");
        };
        assert_eq!(&buf[5..7], &0u16.to_be_bytes());
        assert_eq!(buf.len(), 7);
    }

    #[test]
    fn patches_rejects_non_triples_count_and_overlap() {
        assert_native_func_error(call_patches(list(vec![
            Value::U8(0),
            Value::U8(1),
            Value::U8(1),
            Value::U8(2),
        ])));
        assert_native_func_error(call_patches(list(vec![])));
        let mut seventeen = Vec::new();
        for i in 0..17u16 {
            seventeen.push(Value::U16(i * 10));
            seventeen.push(Value::U8(0));
            seventeen.push(Value::bytes(vec![]));
        }
        assert_native_func_error(call_patches(list(seventeen)));
        assert_native_func_error(call_patches(list(vec![
            Value::U32(u16::MAX as u32 + 1),
            Value::U8(0),
            Value::bytes(vec![]),
        ])));
        assert_native_func_error(call_patches(list(vec![
            Value::U8(0),
            Value::U8(0),
            Value::bytes(vec![]),
            Value::U8(0),
            Value::U8(0),
            Value::bytes(vec![]),
        ])));
        assert_native_func_error(call_patches(list(vec![
            Value::U8(0),
            Value::U8(10),
            Value::bytes(vec![]),
            Value::U8(5),
            Value::U8(1),
            Value::bytes(vec![]),
        ])));
    }

    #[test]
    fn patches_rejects_map_tuple_bytes_argv() {
        assert_native_func_error(call_patches(Value::Compo(
            CompoItem::map(BTreeMap::new()).unwrap(),
        )));
        assert_native_func_error(call_patches(Value::Tuple(
            TupleItem::new(vec![Value::U8(0), Value::U8(1), Value::U8(1)]).unwrap(),
        )));
        assert_native_func_error(call_patches(Value::bytes(vec![0, 1, 1])));
        assert_native_func_error(call_patches(Value::U8(1)));
    }

    #[test]
    fn concat_sha2_list_equals_concatenated_bytes() {
        let argv = list(vec![
            Value::bytes(b"ab".to_vec()),
            Value::bytes(b"c".to_vec()),
        ]);
        let raw = argv.extract_call_data(&SpaceCap::new(0)).unwrap();
        let (h, _) = NativeFunc::call(0, NativeFunc::sha2 as u8, &raw).unwrap();
        assert_eq!(
            h,
            NativeFunc::call(0, NativeFunc::sha2 as u8, b"abc")
                .unwrap()
                .0
        );
    }

    #[test]
    fn concat_sha2_pack_asset_verify_signature_not_patches_packed() {
        let (hash, gas) = NativeFunc::call(0, NativeFunc::sha2 as u8, b"abc").unwrap();
        assert_eq!(gas, 32);
        assert_eq!(hash.ty(), ValueTy::Bytes);
        assert!(
            NativeFunc::call_packed(0, NativeFunc::sha2 as u8, Value::bytes(b"abc".to_vec()))
                .is_err()
        );
        let mut pack = [0u8; 16];
        pack[7] = 1;
        pack[15] = 1;
        let (asset, gas) = NativeFunc::call(0, NativeFunc::pack_asset as u8, &pack).unwrap();
        assert_eq!(gas, 8);
        assert_eq!(asset.ty(), ValueTy::Bytes);
        assert_eq!(
            NativeFunc::argv_pack(NativeFunc::verify_signature as u8).unwrap(),
            NativeArgvPack::Concat
        );
        assert!(NativeFunc::call(0, NativeFunc::patches as u8, &[]).is_err());
        assert!(NativeFunc::call_packed(
            0,
            NativeFunc::pack_asset as u8,
            Value::bytes(pack.to_vec())
        )
        .is_err());
    }
}
