use super::*;
fn packed(args: Vec<Value>) -> Value {
    Value::Tuple(TupleItem::new(args).unwrap())
}

fn call_native(idx: NativeFunc, args: Vec<Value>) -> VmrtRes<(Value, i64)> {
    let cap = SpaceCap::new(0);
    NativeFunc::call_packed(NativeFnEnv::new(&cap), idx as u8, packed(args))
}

fn hash_pair(kind: u8, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(left);
    input[32..].copy_from_slice(right);
    if kind == 0 {
        Sha256::digest(input).into()
    } else {
        let mut hash = Keccak::v256();
        let mut out = [0; 32];
        hash.update(&input);
        hash.finalize(&mut out);
        out
    }
}

fn merkle_args(kind: u8, mode: u8, leaf: &[u8], siblings: &[u8], path: u64) -> Vec<Value> {
    vec![
        Value::U8(kind),
        Value::U8(mode),
        Value::bytes(leaf.to_vec()),
        Value::bytes(siblings.to_vec()),
        Value::U64(path),
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

fn non_uint_values() -> Vec<Value> {
    vec![
        Value::Nil,
        Value::Bool(false),
        Value::bytes(vec![]),
        Value::Address(field::Address::default()),
        Value::Compo(CompoItem::list(std::collections::VecDeque::new()).unwrap()),
        Value::handle(()),
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

#[test]
fn merkle_root_hashes_both_modes_and_charges_per_level() {
    assert_eq!(NativeFunc::merkle_root as u8, 43);
    assert_eq!(NativeFunc::argv_len(43), Some(5));
    assert_eq!(NativeFunc::merkle_root.rty_of(), ValueTy::Bytes);
    assert_eq!(NativeFunc::merkle_root.gas_of(), 16);
    assert_eq!(NativeFunc::secp256k1_recover.rty_of(), ValueTy::Bytes);
    assert_eq!(NativeFunc::secp256k1_recover.gas_of(), 40);
    assert!(matches!(
        finish_ntfunc(NativeFunc::merkle_root, Value::U8(0)),
        Err(ItrErr(NativeFuncError, _))
    ));
    for kind in 0..=1 {
        for mode in 0..=1 {
            for depth in 0usize..=32 {
                let leaf: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(3));
                let siblings: Vec<[u8; 32]> = (0..depth)
                    .map(|level| std::array::from_fn(|i| (i + level * 7 + 1) as u8))
                    .collect();
                let mut node = leaf;
                let path = if mode == 0 && depth != 0 { 1 } else { 0 };
                for (i, sibling) in siblings.iter().enumerate() {
                    node = if mode == 1 {
                        if node <= *sibling {
                            hash_pair(kind, &node, sibling)
                        } else {
                            hash_pair(kind, sibling, &node)
                        }
                    } else if path & (1 << i) == 0 {
                        hash_pair(kind, &node, sibling)
                    } else {
                        hash_pair(kind, sibling, &node)
                    };
                }
                let flat: Vec<u8> = siblings.iter().flatten().copied().collect();
                let (got, gas) = call_native(
                    NativeFunc::merkle_root,
                    merkle_args(kind, mode, &leaf, &flat, path),
                )
                .unwrap();
                assert_eq!(got, Value::bytes(node.to_vec()));
                let per_level = if kind == 0 { 16 } else { 20 };
                assert_eq!(gas, 16 + depth as i64 * per_level);
            }
        }
    }
}

#[test]
fn merkle_root_rejects_invalid_modes_lengths_and_paths() {
    let leaf = [1u8; 32];
    let sibling = [2u8; 32];
    let cases = [
        merkle_args(2, 0, &leaf, &[], 0),
        merkle_args(0, 2, &leaf, &[], 0),
        merkle_args(0, 0, &[0; 31], &[], 0),
        merkle_args(0, 0, &leaf, &[0; 31], 0),
        merkle_args(0, 0, &leaf, &[0; 1056], 0),
        merkle_args(0, 0, &leaf, &sibling, 2),
        merkle_args(0, 1, &leaf, &[], 1),
    ];
    for args in cases {
        assert!(matches!(
            call_native(NativeFunc::merkle_root, args),
            Err(ItrErr(NativeFuncError, _))
        ));
    }
}

#[test]
fn merkle_root_exhausts_parameter_types_and_boundary_paths() {
    let leaf = [1u8; 32];
    let base = merkle_args(0, 0, &leaf, &[], 0);
    for index in [0usize, 1] {
        for value in uint_values(0) {
            let mut args = base.clone();
            args[index] = value;
            assert_eq!(call_native(NativeFunc::merkle_root, args).unwrap().1, 16);
        }
        for value in non_uint_values() {
            let mut args = base.clone();
            args[index] = value;
            assert_native_error(call_native(NativeFunc::merkle_root, args), "uint");
        }
    }
    for index in [2usize, 3] {
        for value in non_bytes_values() {
            let mut args = base.clone();
            args[index] = value;
            assert_native_error(call_native(NativeFunc::merkle_root, args), "bytes");
        }
    }
    for value in uint_values(0) {
        let mut args = base.clone();
        args[4] = value;
        assert_eq!(call_native(NativeFunc::merkle_root, args).unwrap().1, 16);
    }
    for value in non_uint_values() {
        let mut args = base.clone();
        args[4] = value;
        assert_native_error(call_native(NativeFunc::merkle_root, args), "uint");
    }

    let siblings: Vec<u8> = (0..32).flat_map(|n| [n as u8; 32]).collect();
    let (value, gas) = call_native(
        NativeFunc::merkle_root,
        merkle_args(0, 0, &leaf, &siblings, u32::MAX as u64),
    )
    .unwrap();
    assert_eq!(value.ty(), ValueTy::Bytes);
    assert_eq!(gas, 16 + 32 * 16);
    assert_native_error(
        call_native(
            NativeFunc::merkle_root,
            merkle_args(0, 0, &leaf, &siblings, 1u64 << 32),
        ),
        "outside",
    );
    assert_native_error(
        call_native(
            NativeFunc::merkle_root,
            merkle_args(0, 0, &leaf, &[], u64::MAX),
        ),
        "outside",
    );
}

#[test]
fn secp256k1_recover_returns_uncompressed_key_and_rejects_bad_inputs() {
    let account = sys::Account::create_by_secret_key_value([7; 32]).unwrap();
    let digest = [0x42; 32];
    let signature = account.do_sign(&digest);
    let expected_public_key = account.public_key().serialize();
    let recovery_id = (0..=3)
        .find(|id| {
            sys::Account::recover_public_key(&digest, &signature, *id) == Some(expected_public_key)
        })
        .expect("signature must be recoverable");
    let (value, gas) = call_native(
        NativeFunc::secp256k1_recover,
        vec![
            Value::bytes(digest.to_vec()),
            Value::bytes(signature.to_vec()),
            Value::U8(recovery_id),
        ],
    )
    .unwrap();
    assert_eq!(gas, 40);
    let Value::Bytes(public_key) = value else {
        panic!("expected bytes")
    };
    assert_eq!(public_key.len(), 65);
    assert_eq!(public_key[0], 4);
    assert_eq!(public_key, expected_public_key);

    let mut zero_r = signature;
    zero_r[..32].fill(0);
    let mut zero_s = signature;
    zero_s[32..].fill(0);
    let mut overflow_r = signature;
    overflow_r[..32].fill(0xff);
    let mut high_s = signature;
    high_s[32..].fill(0xff);
    for (digest, sig, recid) in [
        (vec![0; 31], signature.to_vec(), recovery_id),
        (digest.to_vec(), vec![0; 63], recovery_id),
        (digest.to_vec(), zero_r.to_vec(), recovery_id),
        (digest.to_vec(), zero_s.to_vec(), recovery_id),
        (digest.to_vec(), overflow_r.to_vec(), recovery_id),
        (digest.to_vec(), high_s.to_vec(), recovery_id),
        (digest.to_vec(), signature.to_vec(), 4),
    ] {
        assert!(matches!(
            call_native(
                NativeFunc::secp256k1_recover,
                vec![Value::bytes(digest), Value::bytes(sig), Value::U8(recid)]
            ),
            Err(ItrErr(NativeFuncError, _))
        ));
    }

    let mut evm_hash = Keccak::v256();
    let mut address_hash = [0; 32];
    evm_hash.update(&public_key[1..]);
    evm_hash.finalize(&mut address_hash);
    assert_eq!(
        hex::encode(&address_hash[12..]),
        "4a62316623ad457f02cdc5d997ded67a383ec569"
    );

    for id in 0..=3 {
        let recovered = sys::Account::recover_public_key(&digest, &signature, id);
        if id == recovery_id {
            assert_eq!(recovered, Some(expected_public_key));
        }
        let result = call_native(
            NativeFunc::secp256k1_recover,
            vec![
                Value::bytes(digest.to_vec()),
                Value::bytes(signature.to_vec()),
                Value::U8(id),
            ],
        );
        match recovered {
            Some(expected) => assert_eq!(result.unwrap().0, Value::bytes(expected.to_vec())),
            None => assert_native_error(result, "invalid signature"),
        }
    }
}

#[test]
fn secp256k1_recover_exhausts_parameter_types_and_exact_lengths() {
    let account = sys::Account::create_by_secret_key_value([9; 32]).unwrap();
    let digest = [0x23; 32];
    let signature = account.do_sign(&digest);
    let recovery_id = (0..=3)
        .find(|id| sys::Account::recover_public_key(&digest, &signature, *id).is_some())
        .unwrap();
    let base = vec![
        Value::bytes(digest.to_vec()),
        Value::bytes(signature.to_vec()),
        Value::U8(recovery_id),
    ];
    for index in [0usize, 1] {
        for value in non_bytes_values() {
            let mut args = base.clone();
            args[index] = value;
            assert_native_error(call_native(NativeFunc::secp256k1_recover, args), "bytes");
        }
    }
    for value in uint_values(recovery_id as u64) {
        let mut args = base.clone();
        args[2] = value;
        assert_eq!(
            call_native(NativeFunc::secp256k1_recover, args).unwrap().1,
            40
        );
    }
    for value in non_uint_values() {
        let mut args = base.clone();
        args[2] = value;
        assert_native_error(call_native(NativeFunc::secp256k1_recover, args), "uint");
    }
    for (index, len, label) in [(0, 31, "digest"), (1, 63, "signature")] {
        let mut args = base.clone();
        args[index] = Value::bytes(vec![0; len]);
        assert_native_error(call_native(NativeFunc::secp256k1_recover, args), label);
        let mut args = base.clone();
        args[index] = Value::bytes(vec![0; len + 2]);
        assert_native_error(call_native(NativeFunc::secp256k1_recover, args), label);
    }
}
