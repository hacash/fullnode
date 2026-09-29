use super::*;
use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, VerifyingKey};
use std::collections::VecDeque;

fn hash_pair(kind: u8, a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (left, right) = if a <= b { (a, b) } else { (b, a) };
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(left);
    input[32..].copy_from_slice(right);
    if kind == 0 {
        Sha256::digest(input).into()
    } else {
        let mut keccak = Keccak::v256();
        let mut out = [0u8; 32];
        keccak.update(&input);
        keccak.finalize(&mut out);
        out
    }
}

fn nodes(bytes: &[u8], label: &str, cty: NativeFunc) -> VmrtRes<Vec<[u8; 32]>> {
    if bytes.len() % 32 != 0 {
        return itr_err_fmt!(
            NativeFuncError,
            "{} {} length must be a multiple of 32",
            cty.name(),
            label
        );
    }
    Ok(bytes
        .chunks_exact(32)
        .map(|chunk| chunk.try_into().expect("chunk length is 32"))
        .collect())
}

pub(crate) fn merkle_multi_root(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::merkle_multi_root;
    let args = func_argv(argv, cty)?;
    let hash_kind = func_u8(&args[0], cty, "hash_kind")?;
    let leaves_bytes = func_bytes(&args[1], cty, "leaves")?;
    let proof_bytes = func_bytes(&args[2], cty, "proof")?;
    let flags = func_bytes(&args[3], cty, "flags")?;
    if hash_kind > 1 {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root invalid hash_kind {}",
            hash_kind
        );
    }
    let leaves = nodes(&leaves_bytes, "leaves", cty)?;
    let proof = nodes(&proof_bytes, "proof", cty)?;
    if leaves.is_empty() {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root requires at least one leaf"
        );
    }
    let total = leaves.len() + proof.len();
    if total > 32 {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root leaves plus proof exceeds 32 nodes"
        );
    }
    let hashes = total - 1;
    let flags_len = hashes.div_ceil(8);
    if flags.len() != flags_len {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root flags length must be {}",
            flags_len
        );
    }
    if hashes % 8 != 0 && flags.last().is_some_and(|last| last >> (hashes % 8) != 0) {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root flags has nonzero unused bits"
        );
    }

    let mut queue = VecDeque::from(leaves);
    let mut proof_at = 0;
    for op in 0..hashes {
        let Some(a) = queue.pop_front() else {
            return itr_err_fmt!(
                NativeFuncError,
                "merkle_multi_root queue exhausted at operation {}",
                op
            );
        };
        let flag = (flags[op / 8] >> (op % 8)) & 1 != 0;
        let b = if flag {
            queue.pop_front().ok_or_else(|| {
                ItrErr::new(
                    NativeFuncError,
                    "merkle_multi_root queue exhausted for flagged node",
                )
            })?
        } else {
            let Some(node) = proof.get(proof_at).copied() else {
                return itr_err_fmt!(NativeFuncError, "merkle_multi_root proof exhausted");
            };
            proof_at += 1;
            node
        };
        queue.push_back(hash_pair(hash_kind, &a, &b));
    }
    if proof_at != proof.len() {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root proof nodes were not fully consumed"
        );
    }
    if queue.len() != 1 {
        return itr_err_fmt!(
            NativeFuncError,
            "merkle_multi_root expected one final node, got {}",
            queue.len()
        );
    }
    Ok(Value::bytes(queue.pop_front().unwrap().to_vec()))
}

pub(crate) fn p256_verify(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::p256_verify;
    let args = func_argv(argv, cty)?;
    let digest = func_bytes(&args[0], cty, "digest")?;
    let signature = func_bytes(&args[1], cty, "signature")?;
    let public_key = func_bytes(&args[2], cty, "public_key")?;
    if digest.len() != 32 {
        return itr_err_fmt!(NativeFuncError, "p256_verify digest must be 32 bytes");
    }
    if signature.len() != 64 {
        return itr_err_fmt!(NativeFuncError, "p256_verify signature must be 64 bytes");
    }
    if public_key.len() != 64 {
        return itr_err_fmt!(NativeFuncError, "p256_verify public_key must be 64 bytes");
    }
    let mut digest32 = [0u8; 32];
    digest32.copy_from_slice(&digest);
    let Ok(signature) = Signature::from_slice(&signature) else {
        return Ok(Value::Bool(false));
    };
    let mut sec1 = [0u8; 65];
    sec1[0] = 4;
    sec1[1..].copy_from_slice(&public_key);
    let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&sec1) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(
        verifying_key.verify_prehash(&digest32, &signature).is_ok(),
    ))
}

pub(crate) fn bitmap_find(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::bitmap_find;
    let args = func_argv(argv, cty)?;
    let bitmap = func_bytes(&args[0], cty, "bitmap")?;
    let start = func_u64(&args[1], cty, "start")?;
    let end = func_u64(&args[2], cty, "end")?;
    let target = func_u8(&args[3], cty, "target")?;
    let Value::Bool(reverse) = args[4] else {
        return itr_err_fmt!(NativeFuncError, "bitmap_find reverse must be bool");
    };
    let bit_len = u64::try_from(bitmap.len())
        .ok()
        .and_then(|len| len.checked_mul(8))
        .ok_or_else(|| ItrErr::new(NativeFuncError, "bitmap_find bitmap length overflow"))?;
    if start > end || end > bit_len {
        return itr_err_fmt!(
            NativeFuncError,
            "bitmap_find range must satisfy start <= end <= bitmap bits"
        );
    }
    if target > 1 {
        return itr_err_fmt!(NativeFuncError, "bitmap_find target must be 0 or 1");
    }
    let mut found = None;
    if start < end {
        let first_byte = (start / 8) as usize;
        let last_byte = ((end - 1) / 8) as usize;
        for byte_at in first_byte..=last_byte {
            let byte_start = (byte_at as u64) * 8;
            let lo = start.saturating_sub(byte_start).min(8) as u8;
            let hi = end.saturating_sub(byte_start).min(8) as u8;
            let range_mask = (u8::MAX << lo) & (u8::MAX >> (8 - hi));
            let candidates = if target == 1 {
                bitmap[byte_at] & range_mask
            } else {
                !bitmap[byte_at] & range_mask
            };
            if candidates != 0 {
                let bit = if reverse {
                    7 - candidates.leading_zeros() as u8
                } else {
                    candidates.trailing_zeros() as u8
                };
                found = Some(byte_start + bit as u64);
                if !reverse {
                    break;
                }
            }
        }
    }
    let (ok, index) = found.map_or((false, 0), |index| (true, index));
    Ok(Value::Tuple(TupleItem::new(vec![
        Value::Bool(ok),
        Value::U64(index),
    ])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(cty: NativeFunc, args: Vec<Value>) -> VmrtRes<(Value, i64)> {
        let cap = SpaceCap::new(0);
        let argv = Value::Tuple(TupleItem::new(args).unwrap());
        NativeFunc::call_packed(NativeFnEnv::new(&cap), cty as u8, argv)
    }

    fn flatten(nodes: &[[u8; 32]]) -> Vec<u8> {
        nodes.iter().flatten().copied().collect()
    }

    fn multi_args(kind: u8, leaves: &[[u8; 32]], proof: &[[u8; 32]], flags: &[u8]) -> Vec<Value> {
        vec![
            Value::U8(kind),
            Value::bytes(flatten(leaves)),
            Value::bytes(flatten(proof)),
            Value::bytes(flags.to_vec()),
        ]
    }

    fn non_bytes_values() -> Vec<Value> {
        vec![
            Value::Nil,
            Value::Bool(false),
            Value::U8(0),
            Value::U16(0),
            Value::U32(0),
            Value::U64(0),
            Value::U128(0),
            Value::Address(field::Address::default()),
            Value::Compo(CompoItem::list(std::collections::VecDeque::new()).unwrap()),
            Value::handle(()),
        ]
    }

    fn uint_values(value: u64) -> [Value; 5] {
        [
            Value::U8(value as u8),
            Value::U16(value as u16),
            Value::U32(value as u32),
            Value::U64(value),
            Value::U128(value as u128),
        ]
    }

    fn assert_native_error(result: VmrtRes<(Value, i64)>, text: &str) {
        match result {
            Err(ItrErr(NativeFuncError, message)) => {
                assert!(
                    message.contains(text),
                    "{message:?} does not contain {text:?}"
                );
            }
            other => panic!("expected NativeFuncError containing {text:?}, got {other:?}"),
        }
    }

    fn oz_pair(kind: u8, a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
        let (left, right) = if a <= b { (a, b) } else { (b, a) };
        let mut pair = [0; 64];
        pair[..32].copy_from_slice(left);
        pair[32..].copy_from_slice(right);
        if kind == 0 {
            Sha256::digest(pair).into()
        } else {
            let mut hasher = Keccak::v256();
            let mut output = [0; 32];
            hasher.update(&pair);
            hasher.finalize(&mut output);
            output
        }
    }

    fn oz_multi_root(kind: u8, leaves: &[[u8; 32]], proof: &[[u8; 32]], flags: &[u8]) -> [u8; 32] {
        let total_hashes = leaves.len() + proof.len() - 1;
        let mut hashes = Vec::with_capacity(total_hashes);
        let (mut leaf_pos, mut hash_pos, mut proof_pos) = (0, 0, 0);
        for op in 0..total_hashes {
            let a = if leaf_pos < leaves.len() {
                let node = leaves[leaf_pos];
                leaf_pos += 1;
                node
            } else {
                let node = hashes[hash_pos];
                hash_pos += 1;
                node
            };
            let flag = (flags[op / 8] >> (op % 8)) & 1 != 0;
            let b = if flag {
                if leaf_pos < leaves.len() {
                    let node = leaves[leaf_pos];
                    leaf_pos += 1;
                    node
                } else {
                    let node = hashes[hash_pos];
                    hash_pos += 1;
                    node
                }
            } else {
                let node = proof[proof_pos];
                proof_pos += 1;
                node
            };
            hashes.push(oz_pair(kind, &a, &b));
        }
        if total_hashes > 0 {
            hashes[total_hashes - 1]
        } else if !leaves.is_empty() {
            leaves[0]
        } else {
            proof[0]
        }
    }

    #[test]
    fn merkle_multi_root_matches_oz_queue_vectors_and_work_gas() {
        assert_eq!(NativeFunc::merkle_multi_root as u8, 44);
        assert_eq!(NativeFunc::argv_len(44), Some(4));
        assert_eq!(
            NativeFunc::merkle_multi_root.argv_pack_of(),
            NativeArgvPack::Packed
        );
        assert_eq!(NativeFunc::merkle_multi_root.rty_of(), ValueTy::Bytes);
        for kind in 0..=1 {
            let leaf = [std::array::from_fn(|i| (i + 3) as u8)];
            let (single, gas) = call(
                NativeFunc::merkle_multi_root,
                multi_args(kind, &leaf, &[], &[]),
            )
            .unwrap();
            assert_eq!(single, Value::bytes(leaf[0].to_vec()));
            assert_eq!(gas, 16);

            let leaves: [[u8; 32]; 3] =
                std::array::from_fn(|n| std::array::from_fn(|i| (i * 3 + n * 11 + 1) as u8));
            let proof: [[u8; 32]; 2] =
                std::array::from_fn(|n| std::array::from_fn(|i| (i * 5 + n * 19 + 7) as u8));
            let flags = [0b0000_1001];
            let expected = oz_multi_root(kind, &leaves, &proof, &flags);
            let reference_root = if kind == 0 {
                "e3199390e1536f0e6d7b2cf8bc21911dd7022fa28ae69a3dac0774fcbb0bb139"
            } else {
                "d2a8325bf33e3750974057d4a555498bcc4e18907659224f4f0d26844a5abc7f"
            };
            assert_eq!(hex::encode(expected), reference_root);
            let (got, gas) = call(
                NativeFunc::merkle_multi_root,
                multi_args(kind, &leaves, &proof, &flags),
            )
            .unwrap();
            assert_eq!(got, Value::bytes(expected.to_vec()));
            assert_eq!(gas, 16 + 4 * if kind == 0 { 16 } else { 20 });

            let leaves: Vec<[u8; 32]> = (0..16)
                .map(|n| std::array::from_fn(|i| (i * 7 + n * 13 + 9) as u8))
                .collect();
            let proof: Vec<[u8; 32]> = (0..16)
                .map(|n| std::array::from_fn(|i| (i * 11 + n * 17 + 5) as u8))
                .collect();
            let flags = [0xff, 0x7f, 0, 0];
            let expected = oz_multi_root(kind, &leaves, &proof, &flags);
            if kind == 1 {
                assert_eq!(
                    hex::encode(expected),
                    "165eae39d5857d21493bc5d87bfbeb6f7d87b9c5ebdb379a07ea8673a81d8a63"
                );
            }
            let (got, gas) = call(
                NativeFunc::merkle_multi_root,
                multi_args(kind, &leaves, &proof, &flags),
            )
            .unwrap();
            assert_eq!(got, Value::bytes(expected.to_vec()));
            assert_eq!(gas, 16 + 31 * if kind == 0 { 16 } else { 20 });
        }
    }

    #[test]
    fn merkle_multi_root_rejects_empty_bad_flags_and_queue_proof_errors() {
        let leaf = [[1u8; 32]];
        let proof = [[2u8; 32], [3u8; 32]];
        for args in [
            multi_args(2, &leaf, &[], &[]),
            multi_args(0, &[], &[], &[]),
            multi_args(0, &leaf, &[], &[0]),
            multi_args(0, &leaf, &proof, &[0b0000_0011]),
            multi_args(0, &[[1; 32], [2; 32]], &[[3; 32]], &[0]),
            multi_args(0, &leaf, &[[2; 32]], &[0b1000_0000]),
            multi_args(0, &leaf, &[[2; 32]; 32], &[]),
        ] {
            assert!(matches!(
                call(NativeFunc::merkle_multi_root, args),
                Err(ItrErr(NativeFuncError, _))
            ));
        }
    }

    #[test]
    fn merkle_multi_root_exhausts_parameter_types_and_boundaries() {
        let leaf = [[1u8; 32]];
        let base = multi_args(0, &leaf, &[], &[]);

        for value in uint_values(0) {
            let mut args = base.clone();
            args[0] = value;
            assert_eq!(call(NativeFunc::merkle_multi_root, args).unwrap().1, 16);
        }
        for value in [
            Value::U16(256),
            Value::U32(256),
            Value::U64(256),
            Value::U128(256),
        ] {
            let mut args = base.clone();
            args[0] = value;
            assert_native_error(call(NativeFunc::merkle_multi_root, args), "hash_kind");
        }
        for value in non_bytes_values() {
            for index in 1..=3 {
                let mut args = base.clone();
                args[index] = value.clone();
                assert_native_error(call(NativeFunc::merkle_multi_root, args), "bytes");
            }
        }

        let max_leaves: Vec<[u8; 32]> = (0..32).map(|n| [n as u8; 32]).collect();
        let flags = [0xff, 0xff, 0xff, 0x7f];
        let (value, gas) = call(
            NativeFunc::merkle_multi_root,
            multi_args(0, &max_leaves, &[], &flags),
        )
        .unwrap();
        assert_eq!(value.ty(), ValueTy::Bytes);
        assert_eq!(gas, 16 + 31 * 16);
        assert_native_error(
            call(
                NativeFunc::merkle_multi_root,
                multi_args(0, &max_leaves, &[], &[0, 0, 0, 0, 0]),
            ),
            "flags length",
        );
    }

    #[test]
    fn merkle_multi_root_matches_openzeppelin_simple_tree_fixture() {
        // Generated with @openzeppelin/merkle-tree 1.0.8 SimpleMerkleTree.of
        // and getMultiProof([1, 3, 6]); proofFlags packed low-bit-first.
        let leaves: Vec<[u8; 32]> = [
            "0c0f1215181b1e2124272a2d303336393c3f4245484b4e5154575a5d60636669",
            "2225282b2e3134373a3d404346494c4f5255585b5e6164676a6d707376797c7f",
            "4346494c4f5255585b5e6164676a6d707376797c7f8285888b8e9194979a9da0",
        ]
        .map(|hex| hex::decode(hex).unwrap().try_into().unwrap())
        .into();
        let proof: Vec<[u8; 32]> = [
            "0104070a0d101316191c1f2225282b2e3134373a3d404346494c4f5255585b5e",
            "171a1d202326292c2f3235383b3e4144474a4d505356595c5f6265686b6e7174",
            "4e5154575a5d606366696c6f7275787b7e8184878a8d909396999c9fa2a5a8ab",
            "595c5f6265686b6e7174777a7d808386898c8f9295989b9ea1a4a7aaadb0b3b6",
            "00fd27162572889faba806dc55eb4ecb455e44d9becd655d9991d653cadac123",
        ]
        .map(|hex| hex::decode(hex).unwrap().try_into().unwrap())
        .into();
        let (value, gas) = call(
            NativeFunc::merkle_multi_root,
            multi_args(1, &leaves, &proof, &[0x60]),
        )
        .unwrap();
        assert_eq!(
            value,
            Value::bytes(
                hex::decode("ca59e4bbb516b8ea39ed52260ffb18951b081917c8177d7a923f8b39a55080f9")
                    .unwrap()
            )
        );
        assert_eq!(gas, 16 + 7 * 20);
    }

    fn bitmap_args(bitmap: &[u8], start: u64, end: u64, target: u8, reverse: bool) -> Vec<Value> {
        vec![
            Value::bytes(bitmap.to_vec()),
            Value::U64(start),
            Value::U64(end),
            Value::U8(target),
            Value::Bool(reverse),
        ]
    }

    fn naive_bitmap(bitmap: &[u8], start: u64, end: u64, target: u8, reverse: bool) -> (bool, u64) {
        let indices: Box<dyn Iterator<Item = u64>> = if reverse {
            Box::new((start..end).rev())
        } else {
            Box::new(start..end)
        };
        indices
            .into_iter()
            .find(|i| (bitmap[(i / 8) as usize] >> (i % 8)) & 1 == target)
            .map_or((false, 0), |i| (true, i))
    }

    fn check_bitmap(bitmap: &[u8], start: u64, end: u64, target: u8, reverse: bool) {
        let (value, gas) = call(
            NativeFunc::bitmap_find,
            bitmap_args(bitmap, start, end, target, reverse),
        )
        .unwrap();
        let expected = naive_bitmap(bitmap, start, end, target, reverse);
        assert_eq!(
            value,
            Value::Tuple(
                TupleItem::new(vec![Value::Bool(expected.0), Value::U64(expected.1),]).unwrap()
            )
        );
        let bytes = if start == end {
            0
        } else {
            ((end - 1) / 8 - start / 8 + 1) as i64
        };
        assert_eq!(gas, 4 + bytes * 2);
    }

    #[test]
    fn bitmap_find_direction_ranges_and_randomized_differential() {
        assert_eq!(NativeFunc::bitmap_find.argv_len_of(), 5);
        assert_eq!(NativeFunc::bitmap_find.rty_of(), ValueTy::Tuple);
        for (bitmap, start, end) in [
            (vec![0b1010_0101], 0, 8),
            (vec![0; 4], 3, 27),
            (vec![0xff; 3], 1, 23),
            (vec![0b1000_0001, 0b0000_0010, 0x5a], 5, 19),
            (vec![0x35, 0xa7, 0x0c, 0xe1], 0, 32),
        ] {
            for target in 0..=1 {
                for reverse in [false, true] {
                    check_bitmap(&bitmap, start, end, target, reverse);
                }
            }
        }
        let mut state = 0x9e37_79b9u32;
        for len in 0..=24 {
            let mut bitmap = vec![0; len];
            for byte in &mut bitmap {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            for _ in 0..24 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let bit_len = (len * 8) as u64;
                let start = if bit_len == 0 {
                    0
                } else {
                    (state as u64) % (bit_len + 1)
                };
                state = state.rotate_left(9);
                let end = start
                    + if bit_len == start {
                        0
                    } else {
                        (state as u64) % (bit_len - start + 1)
                    };
                for target in 0..=1 {
                    for reverse in [false, true] {
                        check_bitmap(&bitmap, start, end, target, reverse);
                    }
                }
            }
        }
    }

    #[test]
    fn bitmap_find_rejects_bad_bounds_and_target() {
        for args in [
            bitmap_args(&[0], 2, 1, 0, false),
            bitmap_args(&[0], 0, 9, 0, false),
            bitmap_args(&[0], 0, 1, 2, false),
            vec![
                Value::bytes(vec![0]),
                Value::U64(0),
                Value::U64(1),
                Value::U8(0),
                Value::U8(0),
            ],
        ] {
            assert!(matches!(
                call(NativeFunc::bitmap_find, args),
                Err(ItrErr(NativeFuncError, _))
            ));
        }
    }

    #[test]
    fn bitmap_find_exhausts_parameter_types_and_half_open_boundaries() {
        let base = bitmap_args(&[0b0000_0010, 0b1000_0000], 0, 16, 1, false);
        for value in non_bytes_values() {
            let mut args = base.clone();
            args[0] = value;
            assert_native_error(call(NativeFunc::bitmap_find, args), "bitmap");
        }
        for index in [1usize, 2] {
            for value in uint_values(if index == 1 { 7 } else { 16 }) {
                let mut args = base.clone();
                args[index] = value;
                assert_eq!(
                    call(NativeFunc::bitmap_find, args).unwrap().0.ty(),
                    ValueTy::Tuple
                );
            }
        }
        for value in uint_values(1) {
            let mut args = base.clone();
            args[3] = value;
            assert_eq!(
                call(NativeFunc::bitmap_find, args).unwrap().0.ty(),
                ValueTy::Tuple
            );
        }
        for value in non_bytes_values()
            .into_iter()
            .filter(|value| !matches!(value, Value::Bool(_)))
        {
            let mut args = base.clone();
            args[4] = value;
            assert_native_error(call(NativeFunc::bitmap_find, args), "reverse");
        }
        let mut overflow = base.clone();
        overflow[1] = Value::U128(u64::MAX as u128 + 1);
        assert_native_error(call(NativeFunc::bitmap_find, overflow), "start");
        let mut invalid_target = base.clone();
        invalid_target[3] = Value::U128(2);
        assert_native_error(call(NativeFunc::bitmap_find, invalid_target), "target");

        check_bitmap(&[0b0000_0010, 0b1000_0000], 0, 0, 0, false);
        check_bitmap(&[0b0000_0010, 0b1000_0000], 0, 16, 1, true);
        check_bitmap(&[0b0000_0010, 0b1000_0000], 15, 16, 1, false);
        assert_native_error(
            call(NativeFunc::bitmap_find, bitmap_args(&[0], 0, 9, 0, false)),
            "range",
        );
    }

    fn p256_call(digest: &[u8], signature: &[u8], public_key: &[u8]) -> VmrtRes<(Value, i64)> {
        call(
            NativeFunc::p256_verify,
            vec![
                Value::bytes(digest.to_vec()),
                Value::bytes(signature.to_vec()),
                Value::bytes(public_key.to_vec()),
            ],
        )
    }

    #[test]
    fn p256_verify_accepts_eip7951_high_s_vector_and_fixed_gas() {
        assert_eq!(NativeFunc::p256_verify.argv_len_of(), 3);
        assert_eq!(NativeFunc::p256_verify.rty_of(), ValueTy::Bool);
        assert_eq!(NativeFunc::p256_verify.gas_of(), 40);
        // EIP-7951 test vector #1 (Wycheproof P1363), a valid high-S signature.
        let input = hex::decode(concat!(
            "bb5a52f42f9c9261ed4361f59422a1e30036e7c32b270c8807a419feca605023",
            "2ba3a8be6b94d5ec80a6d9d1190a436effe50d85a1eee859b8cc6af9bd5c2e18",
            "4cd60b855d442f5b3c7b11eb6c4e0ae7525fe710fab9aa7c77a67f79e6fadd7",
            "62927b10512bae3eddcfe467828128bad2903269919f7086069c8c4df6c73283",
            "8c7787964eaac00e5921fb1498a60f4606766b3d9685001558d1a974e7341513e"
        ))
        .unwrap();
        assert_eq!(input.len(), 160);
        let digest = &input[..32];
        let signature = &input[32..96];
        let public_key = &input[96..];
        let (value, gas) = p256_call(digest, signature, public_key).unwrap();
        assert_eq!(value, Value::Bool(true));
        assert_eq!(gas, NativeFunc::p256_verify.gas_of());

        let (invalid, invalid_gas) = p256_call(&[0x99; 32], signature, public_key).unwrap();
        assert_eq!(invalid, Value::Bool(false));
        assert_eq!(invalid_gas, gas);
        let (bad_key, bad_key_gas) = p256_call(digest, signature, &[0; 64]).unwrap();
        assert_eq!(bad_key, Value::Bool(false));
        assert_eq!(bad_key_gas, gas);

        // EIP-7951 vector #136 makes the verification point the point at infinity.
        let infinity_case = hex::decode(concat!(
            "bb5a52f42f9c9261ed4361f59422a1e30036e7c32b270c8807a419feca605023",
            "7fffffff800000007fffffffffffffffde737d56d38bcf4279dce5617e3192a8",
            "555555550000000055555555555555553ef7a8e48d07df81a693439654210c70",
            "b533d4695dd5b8c5e07757e55e6e516f7e2c88fa0239e23f60e8ec07dd70f28",
            "71b134ee58cc583278456863f33c3a85d881f7d4a39850143e29d4eaf009afe47"
        ))
        .unwrap();
        assert_eq!(infinity_case.len(), 160);
        assert_eq!(
            p256_call(
                &infinity_case[..32],
                &infinity_case[32..96],
                &infinity_case[96..]
            )
            .unwrap()
            .0,
            Value::Bool(false)
        );

        // EIP-7951 vector #137 exercises the adjacent malleability boundary.
        let edge_case = hex::decode(concat!(
            "bb5a52f42f9c9261ed4361f59422a1e30036e7c32b270c8807a419feca605023",
            "7fffffff800000007fffffffffffffffde737d56d38bcf4279dce5617e3192a9",
            "7fffffff800000007fffffffffffffffde737d56d38bcf4279dce5617e3192a8",
            "f50d371b91bfb1d7d14e1323523bc3aa8cbf2c57f9e284de628c8b4536787b86",
            "f94ad887ac94d527247cd2e7d0c8b1291c553c9730405380b14cbb209f5fa2dd"
        ))
        .unwrap();
        assert_eq!(edge_case.len(), 160);
        assert_eq!(
            p256_call(&edge_case[..32], &edge_case[32..96], &edge_case[96..])
                .unwrap()
                .0,
            Value::Bool(true)
        );
    }

    #[test]
    fn p256_verify_rejects_malformed_lengths_and_invalid_values_as_specified() {
        let zero_digest = [0u8; 32];
        let mut zero_sig = [0u8; 64];
        let mut n_sig = [0u8; 64];
        n_sig[..32].copy_from_slice(
            &hex::decode("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551")
                .unwrap(),
        );
        let mut n_s_sig = [0u8; 64];
        n_s_sig[32..].copy_from_slice(
            &hex::decode("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551")
                .unwrap(),
        );
        let public_key = hex::decode(concat!(
            "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
            "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"
        ))
        .unwrap();
        for signature in [&zero_sig[..], &n_sig[..], &n_s_sig[..]] {
            let (value, _) = p256_call(&zero_digest, signature, &public_key).unwrap();
            assert_eq!(value, Value::Bool(false));
        }
        zero_sig[32..].fill(1);
        let (value, _) = p256_call(&zero_digest, &zero_sig, &public_key).unwrap();
        assert_eq!(value, Value::Bool(false));

        for (digest, signature, key) in [
            (&[0; 31][..], &[0; 64][..], &public_key[..]),
            (&[0; 32][..], &[0; 63][..], &public_key[..]),
            (&[0; 32][..], &[0; 64][..], &public_key[..63]),
        ] {
            assert!(matches!(
                p256_call(digest, signature, key),
                Err(ItrErr(NativeFuncError, _))
            ));
        }
        let mut out_of_field = [0xff; 64];
        out_of_field[0] = 0xff;
        let (value, _) = p256_call(&zero_digest, &zero_sig, &out_of_field).unwrap();
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn p256_verify_reduces_verification_point_x_mod_order() {
        use p256::elliptic_curve::{
            group::Group,
            sec1::{FromEncodedPoint, ToEncodedPoint},
            PrimeField,
        };
        use p256::{AffinePoint, EncodedPoint, FieldBytes, ProjectivePoint, Scalar};

        let n = hex::decode("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551")
            .unwrap();
        let mut recovered_point = None;
        let mut r_bytes = [0u8; 32];
        for delta in 1u8..=100 {
            let mut x = n.clone();
            x[31] += delta;
            let mut r = [0u8; 32];
            r[31] = delta;
            let mut compressed = [0u8; 33];
            compressed[0] = 2;
            compressed[1..].copy_from_slice(&x);
            let encoded = EncodedPoint::from_bytes(compressed).unwrap();
            if let Some(point) =
                Option::<AffinePoint>::from(AffinePoint::from_encoded_point(&encoded))
            {
                recovered_point = Some(ProjectivePoint::from(point));
                r_bytes = r;
                break;
            }
        }
        let recovered_point = recovered_point.expect("an x in n..p has a curve point");
        let r_scalar =
            Option::<Scalar>::from(Scalar::from_repr(FieldBytes::clone_from_slice(&r_bytes)))
                .expect("chosen r is nonzero and below the order");
        let inverse = Option::<Scalar>::from(r_scalar.invert()).unwrap();
        let q = (recovered_point - ProjectivePoint::GENERATOR) * inverse;
        assert!(!bool::from(q.is_identity()));
        let encoded_q = q.to_affine().to_encoded_point(false);
        let mut signature = [0u8; 64];
        signature[..32].copy_from_slice(&r_bytes);
        signature[63] = 1;
        let mut digest = [0u8; 32];
        digest[31] = 1;
        let (value, _) = p256_call(&digest, &signature, &encoded_q.as_bytes()[1..]).unwrap();
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn p256_verify_exhausts_parameter_types_and_exact_lengths() {
        let digest = [0u8; 32];
        let signature = [0u8; 64];
        let public_key = [0u8; 64];
        let base = vec![
            Value::bytes(digest.to_vec()),
            Value::bytes(signature.to_vec()),
            Value::bytes(public_key.to_vec()),
        ];
        for value in non_bytes_values() {
            for index in 0..=2 {
                let mut args = base.clone();
                args[index] = value.clone();
                assert_native_error(call(NativeFunc::p256_verify, args), "bytes");
            }
        }
        for (index, len, label) in [
            (0, 31, "digest"),
            (1, 63, "signature"),
            (2, 63, "public_key"),
        ] {
            let mut args = base.clone();
            args[index] = Value::bytes(vec![0; len]);
            assert_native_error(call(NativeFunc::p256_verify, args), label);
            let mut args = base.clone();
            args[index] = Value::bytes(vec![0; len + 2]);
            assert_native_error(call(NativeFunc::p256_verify, args), label);
        }
        let (value, gas) = call(NativeFunc::p256_verify, base).unwrap();
        assert_eq!(value, Value::Bool(false));
        assert_eq!(gas, 40);
    }
}
