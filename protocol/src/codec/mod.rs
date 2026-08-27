//! Standard action / tx / block codecs registered into Registry.

pub mod action;
pub mod block;
pub mod tx;

#[cfg(test)]
pub(crate) mod test_reg {
    use base::{
        ActionRef, BinaryCodecs, BlockHasherFn, BlockRef, HASH_SIZE, JsonCodecs, TxRef,
        WireCodecTable,
    };
    use sys::Ret;

    pub fn hasher() -> BlockHasherFn {
        |_, stuff| sys::calculate_hash(stuff)
    }

    pub struct TestRegistry {
        table: WireCodecTable,
    }

    impl TestRegistry {
        pub fn protocol() -> Ret<Self> {
            let mut table = WireCodecTable::new();
            for binding in crate::wire::ACTION_CODECS {
                table.add_action(*binding)?;
            }
            for binding in crate::wire::TX_CODECS {
                table.add_tx(*binding)?;
            }
            Ok(Self { table })
        }
    }

    impl BinaryCodecs for TestRegistry {
        fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)> {
            self.table.decode_action(self, buf)
        }

        fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)> {
            self.table.decode_transaction(self, buf)
        }

        fn decode_block(&self, buf: &[u8]) -> Ret<(BlockRef, usize)> {
            crate::codec::block::create_std_block(self, buf)
        }

        fn peek_block_size(&self, buf: &[u8]) -> Ret<usize> {
            self.decode_block(buf).map(|(_, used)| used)
        }

        fn block_hash(&self, _height: u64, stuff: &[u8]) -> [u8; HASH_SIZE] {
            sys::calculate_hash(stuff)
        }

        fn block_hasher_fn(&self) -> BlockHasherFn {
            hasher()
        }
    }

    impl JsonCodecs for TestRegistry {
        fn decode_action_json(&self, json: &str) -> Ret<ActionRef> {
            self.table.decode_action_json(self, json)
        }
    }
}

#[cfg(test)]
mod binary_codec_tests {
    use super::action::{
        ActionListW1, AstIf, AstSelect, TexCellExecute, TransferHacTo, TransferSatTo,
        create_ast_if, create_ast_select,
    };
    use super::block::{StdBlock, create_std_block};
    use super::test_reg::{TestRegistry, hasher};
    use super::tx::{
        StdTransaction, create_transaction_type1, create_transaction_type2,
        create_transaction_type3,
    };
    use crate::codec::action::tex::TexCell;
    use base::{BinaryCodecs, BlockBuild, JsonCodecs, TransactionBuild};
    use field::{Address, Amount, Decode, Encode, Fold64, ListW1, Satoshi, Sign, ToJSON, Uint2};
    use std::sync::Arc;

    fn assert_size_matches_encode(value: &impl Encode) {
        assert_eq!(value.size(), value.encode().len());
    }

    fn sample_sign() -> Sign {
        Sign {
            publickey: [0x02; Sign::PUBLICKEY_SIZE].into(),
            signature: [0xab; Sign::SIGNATURE_SIZE].into(),
        }
    }

    fn child_action() -> TransferHacTo {
        TransferHacTo::new(Address::default(), Amount::mei(1))
    }

    #[test]
    fn derived_and_handwritten_actions_size_matches_encode() {
        assert_size_matches_encode(&child_action());

        let empty = AstSelect::create_by(0, 0, vec![]).unwrap();
        assert_size_matches_encode(&empty);

        let filled = AstSelect::create_by(1, 1, vec![Arc::new(child_action())]).unwrap();
        assert_size_matches_encode(&filled);

        let ast_if = AstIf::create_by(
            filled.clone(),
            empty.clone(),
            AstSelect::create_by(1, 1, vec![]).unwrap(),
        );
        assert_size_matches_encode(&ast_if);

        let tex = TexCellExecute {
            kind: Uint2::from(TexCellExecute::KIND),
            addr: Address::default(),
            cells: ListW1::from(vec![TexCell::ZhuPay {
                haczhu: Fold64::from(1).unwrap(),
            }])
            .unwrap(),
            sign: sample_sign(),
        };
        assert_size_matches_encode(&tex);

        let max_w1 = ActionListW1::from_vec(
            (0..u8::MAX)
                .map(|_| Arc::new(child_action()) as _)
                .collect(),
        )
        .unwrap();
        assert_size_matches_encode(&max_w1);
    }

    #[test]
    fn standard_transactions_and_block_size_matches_encode() {
        let fee = Amount::mei(1);
        let main = Address::default();
        assert_size_matches_encode(&StdTransaction::new(
            hacash_params::TX_TYPE_1,
            main,
            fee.clone(),
        ));
        assert_size_matches_encode(&StdTransaction::new(
            hacash_params::TX_TYPE_2,
            main,
            fee.clone(),
        ));
        assert_size_matches_encode(&StdTransaction::new(
            hacash_params::TX_TYPE_3,
            main,
            fee.clone(),
        ));

        let mut t1 = StdTransaction::new(hacash_params::TX_TYPE_1, main, fee.clone());
        t1.push_action(Arc::new(child_action())).unwrap();
        assert_size_matches_encode(&t1);
        let mut t2 = StdTransaction::new(hacash_params::TX_TYPE_2, main, fee.clone());
        t2.push_action(Arc::new(child_action())).unwrap();
        assert_size_matches_encode(&t2);
        let mut t3 = StdTransaction::new(hacash_params::TX_TYPE_3, main, fee);
        t3.push_action(Arc::new(child_action())).unwrap();
        assert_size_matches_encode(&t3);

        let empty_block = StdBlock::new(hasher());
        assert_size_matches_encode(&empty_block);
        assert_eq!(empty_block.encode().len(), StdBlock::INTRO_SIZE);

        let mut block = StdBlock::new(hasher());
        block.push_transaction(Arc::new(t2)).unwrap();
        assert_size_matches_encode(&block);
    }

    #[test]
    fn wrong_kind_type_version_are_rejected_consistently() {
        let reg = TestRegistry::protocol().unwrap();
        let hac = child_action().encode();
        let sat = TransferSatTo::new(Address::default(), Satoshi::from(7)).encode();

        assert!(TransferHacTo::decode(&sat).is_err());
        assert!(TransferSatTo::decode(&hac).is_err());
        assert!(create_ast_select(&reg, &hac).is_err());
        assert!(create_ast_if(&reg, &hac).is_err());

        let unregistered = {
            let mut buf = hac.clone();
            buf[0] = 0xff;
            buf[1] = 0xff;
            buf
        };
        assert!(TransferHacTo::decode(&unregistered).is_err());
        assert!(reg.decode_action(&unregistered).is_err());
        assert!(create_ast_select(&reg, &unregistered).is_err());

        let nested = AstIf::create_by(
            AstSelect::create_by(1, 1, vec![Arc::new(child_action())]).unwrap(),
            AstSelect::create_by(0, 0, vec![]).unwrap(),
            AstSelect::create_by(0, 0, vec![]).unwrap(),
        );
        let mut nested_wire = nested.encode();
        // AstIf kind (2 bytes) then cond.kind (2 bytes).
        nested_wire[2] = 0;
        nested_wire[3] = 1;
        assert!(create_ast_if(&reg, &nested_wire).is_err());
        assert!(reg.decode_action(&nested_wire).is_err());

        let t2 = StdTransaction::new(hacash_params::TX_TYPE_2, Address::default(), Amount::mei(1))
            .encode();
        assert!(create_transaction_type1(&reg, &t2).is_err());
        assert!(create_transaction_type3(&reg, &t2).is_err());
        let (decoded, used) = create_transaction_type2(&reg, &t2).unwrap();
        assert_eq!(used, t2.len());
        assert_eq!(decoded.ty(), hacash_params::TX_TYPE_2);

        let mut t1_as_t2 = t2.clone();
        t1_as_t2[0] = hacash_params::TX_TYPE_1;
        assert!(create_transaction_type2(&reg, &t1_as_t2).is_err());

        let mut intro = StdBlock::new(hasher()).encode_intro();
        intro[0] = 2;
        assert!(StdBlock::decode_intro(hasher(), &intro).is_err());
        assert!(create_std_block(&reg, &intro).is_err());
    }

    #[test]
    fn decode_action_json_accepts_registered_name_kind() {
        let reg = TestRegistry::protocol().unwrap();
        let hac = child_action();
        let numeric = hac.to_json();

        // Numeric kind still round-trips.
        let via_numeric = reg.decode_action_json(&numeric).unwrap();
        assert_eq!(via_numeric.encode(), hac.encode());

        // Quoted registered action name as `kind`.
        let to = Address::default().to_readable();
        let amount = Amount::mei(1).to_fin_string();
        let quoted = format!(
            "{{\"kind\":\"transfer_hac_to\",\"to\":\"{}\",\"hacash\":\"{}\"}}",
            to, amount
        );
        let via_name = reg.decode_action_json(&quoted).unwrap();
        assert_eq!(via_name.encode(), hac.encode());

        // Unknown name is rejected.
        let bad = quoted.replace("transfer_hac_to", "no_such_action");
        assert!(reg.decode_action_json(&bad).is_err());

        // A bare name token is not valid JSON, so it fails at the parser layer.
        let bare = format!(
            "{{\"kind\":transfer_hac_to,\"to\":\"{}\",\"hacash\":\"{}\"}}",
            to, amount
        );
        assert!(reg.decode_action_json(&bare).is_err());
    }
}
