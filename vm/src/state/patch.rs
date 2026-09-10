use crate::rt::{ItrErr, ItrErrCode::*, SpaceCap, VmrtRes};

const PATCH_COUNT_MIN: u8 = 1;
const PATCH_COUNT_MAX: u8 = 16;
const PATCH_HEADER_LEN: usize = 6;

/// Encode `[(offset, delete_len, insert)]` as a canonical SPATCH `patch_set`.
pub fn encode_patch_set(patches: &[(u16, u16, &[u8])]) -> VmrtRes<Vec<u8>> {
    let n = patches.len();
    if n < PATCH_COUNT_MIN as usize || n > PATCH_COUNT_MAX as usize {
        return itr_err_code!(StoragePatchInvalid);
    }
    let mut out = Vec::new();
    out.push(n as u8);
    for (offset, delete_len, insert) in patches {
        let insert_len = u16::try_from(insert.len())
            .map_err(|_| ItrErr::new(StoragePatchInvalid, "insert too long"))?;
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&delete_len.to_be_bytes());
        out.extend_from_slice(&insert_len.to_be_bytes());
        out.extend_from_slice(insert);
    }
    Ok(out)
}

/// Decode a canonical SPATCH `patch_set` into `(offset, delete_len, insert)`.
pub fn decode_patch_set(bytes: &[u8]) -> VmrtRes<Vec<(u16, u16, Vec<u8>)>> {
    if bytes.is_empty() {
        return itr_err_code!(StoragePatchInvalid);
    }
    let count = bytes[0];
    if !(PATCH_COUNT_MIN..=PATCH_COUNT_MAX).contains(&count) {
        return itr_err_code!(StoragePatchInvalid);
    }
    let mut i = 1usize;
    let mut patches = Vec::with_capacity(count as usize);
    let mut prev_offset: Option<u16> = None;
    let mut prev_end: u32 = 0;
    for _ in 0..count {
        let header_end = i
            .checked_add(PATCH_HEADER_LEN)
            .ok_or_else(|| ItrErr::code(StoragePatchInvalid))?;
        if header_end > bytes.len() {
            return itr_err_code!(StoragePatchInvalid);
        }
        let offset = u16::from_be_bytes([bytes[i], bytes[i + 1]]);
        let delete_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]);
        let insert_len = u16::from_be_bytes([bytes[i + 4], bytes[i + 5]]);
        i = header_end;
        let insert_end = i
            .checked_add(insert_len as usize)
            .ok_or_else(|| ItrErr::code(StoragePatchInvalid))?;
        if insert_end > bytes.len() {
            return itr_err_code!(StoragePatchInvalid);
        }
        let insert = bytes[i..insert_end].to_vec();
        i = insert_end;
        if let Some(prev) = prev_offset {
            if (offset as u32) <= prev as u32 || (offset as u32) < prev_end {
                return itr_err_code!(StoragePatchInvalid);
            }
        }
        prev_offset = Some(offset);
        prev_end = (offset as u32) + (delete_len as u32);
        patches.push((offset, delete_len, insert));
    }
    if i != bytes.len() {
        return itr_err_code!(StoragePatchInvalid);
    }
    Ok(patches)
}

/// Decode `patch_set` and apply it to `original` using original coordinates.
pub fn decode_and_apply(original: &[u8], patch_set: &[u8], cap: &SpaceCap) -> VmrtRes<Vec<u8>> {
    let patches = decode_patch_set(patch_set)?;
    let orig_len = original.len() as u32;
    for (offset, delete_len, _) in &patches {
        let end = (*offset as u32) + (*delete_len as u32);
        if end > orig_len {
            return itr_err_code!(StoragePatchInvalid);
        }
    }
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (offset, delete_len, insert) in &patches {
        let off = *offset as usize;
        out.extend_from_slice(&original[cursor..off]);
        out.extend_from_slice(insert);
        cursor = off + (*delete_len as usize);
    }
    out.extend_from_slice(&original[cursor..]);
    if out.is_empty() || out.len() > cap.value_size {
        return itr_err_code!(StoragePatchInvalid);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::{Bytecode, GasTable, ItrErrCode};

    fn cap() -> SpaceCap {
        SpaceCap::new(0)
    }

    fn apply(original: &[u8], patch_set: &[u8]) -> VmrtRes<Vec<u8>> {
        decode_and_apply(original, patch_set, &cap())
    }

    fn apply_patches(original: &[u8], patches: &[(u16, u16, &[u8])]) -> VmrtRes<Vec<u8>> {
        apply(original, &encode_patch_set(patches)?)
    }

    fn invalid<T: std::fmt::Debug>(r: VmrtRes<T>) {
        assert_eq!(r.unwrap_err().0, ItrErrCode::StoragePatchInvalid);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let patches: Vec<(u16, u16, &[u8])> = vec![
            (0, 1, b"A".as_slice()),
            (4, 0, b"xy".as_slice()),
            (10, 2, b"".as_slice()),
        ];
        let bytes = encode_patch_set(&patches).unwrap();
        let decoded = decode_patch_set(&bytes).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0], (0, 1, b"A".to_vec()));
        assert_eq!(decoded[1], (4, 0, b"xy".to_vec()));
        assert_eq!(decoded[2], (10, 2, vec![]));
        assert_eq!(encode_patch_set(&patches).unwrap(), bytes);
    }

    #[test]
    fn decode_rejects_count_zero() {
        invalid(decode_patch_set(&[0]));
        invalid(encode_patch_set(&[]));
    }

    #[test]
    fn decode_rejects_count_seventeen() {
        invalid(decode_patch_set(&[17]));
        let many: Vec<(u16, u16, &[u8])> =
            (0..17).map(|i| (i as u16 * 2, 0, b"".as_slice())).collect();
        invalid(encode_patch_set(&many));
    }

    #[test]
    fn decode_rejects_trailing_garbage() {
        let mut bytes = encode_patch_set(&[(0, 0, b"x")]).unwrap();
        bytes.push(0xff);
        invalid(decode_patch_set(&bytes));
        invalid(apply(b"abc", &bytes));
    }

    #[test]
    fn decode_rejects_insert_len_mismatch() {
        let mut bytes = vec![1];
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&5u16.to_be_bytes());
        bytes.extend_from_slice(b"xy");
        invalid(decode_patch_set(&bytes));
    }

    #[test]
    fn apply_rejects_offset_oob() {
        let bytes = encode_patch_set(&[(5, 0, b"x")]).unwrap();
        invalid(apply(b"abcd", &bytes));
    }

    #[test]
    fn apply_rejects_offset_plus_delete_oob() {
        let bytes = encode_patch_set(&[(3, 2, b"")]).unwrap();
        invalid(apply(b"abcd", &bytes));
    }

    #[test]
    fn apply_rejects_u16_add_overflow() {
        let bytes = encode_patch_set(&[(u16::MAX, 1, b"")]).unwrap();
        invalid(apply(b"hello", &bytes));
    }

    #[test]
    fn decode_rejects_overlap_and_same_offset() {
        let overlap = encode_patch_set(&[(0, 4, b""), (2, 1, b"x")]).unwrap();
        invalid(decode_patch_set(&overlap));
        invalid(apply(b"abcdefghij", &overlap));
        let same = encode_patch_set(&[(5, 0, b""), (5, 0, b"Y")]).unwrap();
        invalid(decode_patch_set(&same));
        invalid(apply(b"abcdefghij", &same));
    }

    #[test]
    fn decode_allows_adjacent_original_ranges() {
        let bytes = encode_patch_set(&[(0, 4, b"AB"), (4, 2, b"CD")]).unwrap();
        assert_eq!(apply(b"abcdefgh", &bytes).unwrap(), b"ABCDgh");
    }

    #[test]
    fn apply_rejects_empty_final() {
        let bytes = encode_patch_set(&[(0, 3, b"")]).unwrap();
        invalid(apply(b"abc", &bytes));
    }

    #[test]
    fn apply_rejects_final_over_1280() {
        let original = vec![0u8; 1280];
        let bytes = encode_patch_set(&[(0, 0, b"Z")]).unwrap();
        invalid(apply(&original, &bytes));
        let huge = vec![0u8; 1281];
        let bytes = encode_patch_set(&[(0, 0, huge.as_slice())]).unwrap();
        invalid(apply(b"", &bytes));
    }

    #[test]
    fn apply_overwrite_insert_delete_mix_original_coords() {
        assert_eq!(apply_patches(b"abc", &[(1, 1, b"X")]).unwrap(), b"aXc");
        assert_eq!(apply_patches(b"abc", &[(1, 0, b"XY")]).unwrap(), b"aXYbc");
        assert_eq!(apply_patches(b"abc", &[(1, 1, b"")]).unwrap(), b"ac");
        // p2 uses original index 6 ('g'), not the post-insert index.
        let original = b"abcdefghij";
        let out = apply_patches(original, &[(2, 2, b"XYZ"), (6, 1, b"Q")]).unwrap();
        assert_eq!(out, b"abXYZefQhij");
    }

    #[test]
    fn opcode_numeric_values() {
        assert_eq!(Bytecode::SSTAT as u8, 0x98);
        assert_eq!(Bytecode::SLOAD as u8, 0x99);
        assert_eq!(Bytecode::SPATCH as u8, 0x9a);
        assert_eq!(Bytecode::SEDIT as u8, 0x9b);
        assert_eq!(Bytecode::try_from_u8(0x98).unwrap(), Bytecode::SSTAT);
        assert_eq!(Bytecode::try_from_u8(0x99).unwrap(), Bytecode::SLOAD);
        assert_eq!(Bytecode::try_from_u8(0x9a).unwrap(), Bytecode::SPATCH);
        assert_eq!(Bytecode::try_from_u8(0x9b).unwrap(), Bytecode::SEDIT);
        assert!(Bytecode::SSTAT.metadata().valid);
        assert!(Bytecode::SPATCH.metadata().valid);
        let meta = Bytecode::SPATCH.metadata();
        assert_eq!((meta.param, meta.input, meta.output), (0, 3, 1));
        let gst = GasTable::new(0);
        assert_eq!(gst.gas(Bytecode::SSTAT as u8), 32);
        assert_eq!(gst.gas(Bytecode::SLOAD as u8), 32);
        assert_eq!(gst.gas(Bytecode::SPATCH as u8), 64);
        assert_eq!(gst.gas(Bytecode::SEDIT as u8), 64);
        assert_eq!(ItrErrCode::StoragePatchExpected as u8, 111);
        assert_eq!(ItrErrCode::StoragePatchInvalid as u8, 112);
        let Some((_, bc, pms, args, rs)) = crate::rt::pick_ir_func("storage_patch") else {
            panic!("storage_patch irfn missing");
        };
        assert_eq!(bc, Bytecode::SPATCH);
        assert_eq!((pms, args, rs), (0, 3, 1));
    }
}
