//! VM syscall actions: ACTENV (0x07xx) / ACTVIEW (0x06xx), invoked via
//! `Context::action_call` with kid = `[0x07|0x06, idx]` where idx = KIND % 256.

use field::{
    Address, AddressW1, AssetAmtW1, DiamondName, DiamondNameListMax200, DiamondNumber,
    Fold64, ListW1, Uint1, Uint2,
};

base::action_simple! { EnvHeight, 0x0701, 3, CALL_ONLY, {
}, this, {
    name: "block_height",
    description: "Syscall: Get block height".to_owned()
}}

base::action_simple! { TxMainAddr, 0x0702, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get main address".to_owned()
}}

base::action_simple! { BlockAuthorAddr, 0x0703, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get author address".to_owned()
}}

base::action_simple! { TxMessageNum, 0x0704, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get transaction message count".to_owned()
}}

base::action_simple! { TxBlobNum, 0x0705, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get transaction blob count".to_owned()
}}

base::action_simple! { BalanceCoin, 0x0601, 3, CALL_ONLY, {
    addr: Address
}, this, {
    description: format!("Syscall: Get balance for {}", this.addr.to_readable())
}}

base::action_simple! { BalanceAsset, 0x0602, 3, CALL_ONLY, {
    addr: Address,
    serial: Fold64
}, this, {
    description: format!("Syscall: Get asset {} balance for {}", this.serial.uint(), this.addr.to_readable())
}}

base::action_simple! { BalanceFungible, 0x0603, 3, CALL_ONLY, {
    addr: Address,
    serial: Fold64
}, this, {
    description: format!("Syscall: Get fungible asset {} balance for {}", this.serial.uint(), this.addr.to_readable())
}}

base::action_simple! { AssetMeta, 0x0604, 3, CALL_ONLY, {
    serial: Fold64
}, this, {
    description: format!("Syscall: Get asset {} metadata", this.serial.uint())
}}

// Batch check of the PoolCoreV2 intent slot-1 expected-final-balance table:
// wire `table` = AssetAmtW1 (u8 count + fold64(serial) ++ fold64(expected) pairs),
// the same typed list the balance ledger itself uses — schema-rendered entries,
// decode-time fold bounds, zero expected allowed. Caller builds the prefixed wire
// form (the house convention, cf. the bridge's sigset_at_least AddressW1).
// Returns whether every entry matches its expected fungible balance.
base::action_simple! { BalanceTableCheck, 0x0605, 3, CALL_ONLY, {
    addr: Address,
    table: AssetAmtW1
}, this, {
    description: format!("Syscall: Check {} fungible balance table ({} entries)", this.addr.to_readable(), this.table.length())
}}

// Ordered, bounded balance snapshot. The caller supplies a W1 count followed
// by canonical Fold64 serials; the view returns count + u64 BE values.
base::action_simple! { BalanceFungibleBatch, 0x0606, 3, CALL_ONLY, {
    addr: Address,
    serials: ListW1<Fold64>
}, this, {
    description: format!("Syscall: Get {} fungible balances for {}", this.serials.length(), this.addr.to_readable())
}}

base::action_simple! { CheckSignature, 0x0609, 3, CALL_ONLY, {
    addr: Address
}, this, {
    description: format!("Syscall: Check signature for {}", this.addr.to_readable())
}}

base::action_simple! { SigsetCount, 0x060A, 3, CALL_ONLY, {
    keys: AddressW1
}, this, {
    description: format!("Syscall: Count signatures among {} keys", this.keys.length())
}}

base::action_simple! { SigsetAtLeast, 0x060B, 3, CALL_ONLY, {
    keys: AddressW1,
    threshold: Uint1
}, this, {
    description: format!(
        "Syscall: At-least {} signatures among {} keys",
        this.threshold.uint(),
        this.keys.length()
    )
}}

base::action_simple! { HacdInscNum, 0x0611, 3, CALL_ONLY, {
    diamond: DiamondName
}, this, {
    description: format!("Syscall: Get diamond inscription number for <{}>", this.diamond.to_readable())
}}

base::action_simple! { HacdInscGet, 0x0612, 3, CALL_ONLY, {
    diamond: DiamondName,
    inscidx: Uint1
}, this, {
    description: format!("Syscall: Get diamond inscription data for <{}>", this.diamond.to_readable())
}}

base::action_simple! { HacdNameList, 0x0613, 3, CALL_ONLY, {
    addr: Address,
    page: DiamondNumber,
    limit: DiamondNumber
}, this, {
    description: format!("Syscall: Get HACD name list for {} page {} limit {}", this.addr.to_readable(), this.page.uint(), this.limit.uint())
}}

base::action_simple! { HacdOwnerAddrs, 0x0614, 3, CALL_ONLY, {
    diamonds: DiamondNameListMax200
}, this, {
    description: format!("Syscall: Get HACD owner addresses for {}", this.diamonds.splitstr())
}}
base::action_simple! { TxMessage, 0x0621, 3, CALL_ONLY, {
    idx: Uint1
}, this, {
    description: format!("Syscall: Get transaction message {}", this.idx.uint())
}}

base::action_simple! { TxMessageSingle, 0x0622, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get the only transaction message".to_owned()
}}

base::action_simple! { TxBlob, 0x0623, 3, CALL_ONLY, {
    idx: Uint1,
    start: Uint2,
    end: Uint2
}, this, {
    description: format!("Syscall: Get transaction blob {} [{}..{}]", this.idx.uint(), this.start.uint(), this.end.uint())
}}

base::action_simple! { TxBlobSize, 0x0624, 3, CALL_ONLY, {
    idx: Uint1
}, this, {
    description: format!("Syscall: Get transaction blob {} size", this.idx.uint())
}}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::test_reg::TestRegistry;
    use base::{ActScope, Action, ActionName, BinaryCodecs};
    use field::{AssetAmt, Decode, Encode};

    fn privkey_addr(n: u8) -> Address {
        let mut bytes = [0u8; Address::SIZE];
        bytes[1] = n;
        Address::from(bytes)
    }

    fn keys(addrs: Vec<Address>) -> AddressW1 {
        AddressW1::from(addrs).unwrap()
    }

    #[test]
    fn sigset_names_kinds_and_scope_are_call_only_views() {
        assert_eq!(SigsetCount::KIND, 0x060A);
        assert_eq!(SigsetAtLeast::KIND, 0x060B);
        assert_eq!(SigsetCount::NAME, "sigset_count");
        assert_eq!(SigsetAtLeast::NAME, "sigset_at_least");
        assert_eq!(<SigsetCount as ActionName>::NAME, "sigset_count");
        assert_eq!(SigsetCount::SCOPE, ActScope::CALL_ONLY);
        assert_eq!(SigsetAtLeast::SCOPE, ActScope::CALL_ONLY);
        assert_eq!(
            SigsetCount::new(keys(vec![privkey_addr(1)])).scope(),
            ActScope::CALL_ONLY
        );
    }

    #[test]
    fn sigset_round_trips_and_n50_body_sizes() {
        let one = keys(vec![privkey_addr(1)]);
        let count = SigsetCount::new(one.clone());
        let wire = count.encode();
        let (decoded, used) = SigsetCount::decode(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);
        assert_eq!(count.size(), wire.len());

        let at_least = SigsetAtLeast::new(one, Uint1::from(1));
        let wire = at_least.encode();
        let (decoded, used) = SigsetAtLeast::decode(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);

        let n50 = keys((1u8..=50).map(privkey_addr).collect());
        assert_eq!(n50.encode().len(), 1 + 21 * 50);
        assert_eq!(n50.size(), 1051);
        let count50 = SigsetCount::new(n50.clone());
        assert_eq!(count50.size(), 2 + 1051);
        let (decoded, used) = SigsetCount::decode(&count50.encode()).unwrap();
        assert_eq!(used, 2 + 1051);
        assert_eq!(decoded.keys.length(), 50);

        let at_least50 = SigsetAtLeast::new(n50, Uint1::from(1));
        assert_eq!(at_least50.size(), 2 + 1052);
        let (decoded, used) = SigsetAtLeast::decode(&at_least50.encode()).unwrap();
        assert_eq!(used, 2 + 1052);
        assert_eq!(decoded.threshold.uint(), 1);

        let n51 = keys((1u8..=51).map(privkey_addr).collect());
        assert_eq!(n51.encode().len(), 1 + 21 * 51);
        let count51 = SigsetCount::new(n51.clone());
        let (decoded, used) = SigsetCount::decode(&count51.encode()).unwrap();
        assert_eq!(used, count51.encode().len());
        assert_eq!(decoded.keys.length(), 51);
        let at_least51 = SigsetAtLeast::new(n51, Uint1::from(1));
        assert!(SigsetAtLeast::decode(&at_least51.encode()).is_ok());

        let n200 = keys((1u8..=200).map(privkey_addr).collect());
        assert_eq!(n200.encode().len(), 1 + 21 * 200);
        assert_eq!(n200.encode().len(), 4201);
        let count200 = SigsetCount::new(n200.clone());
        assert_eq!(count200.size(), 2 + 4201);
        let (decoded, used) = SigsetCount::decode(&count200.encode()).unwrap();
        assert_eq!(used, 2 + 4201);
        assert_eq!(decoded.keys.length(), 200);

        let at_least200 = SigsetAtLeast::new(n200, Uint1::from(1));
        assert_eq!(at_least200.size(), 2 + 4202);
        let (decoded, used) = SigsetAtLeast::decode(&at_least200.encode()).unwrap();
        assert_eq!(used, 2 + 4202);
        assert_eq!(decoded.keys.length(), 200);
        assert_eq!(decoded.threshold.uint(), 1);
    }

    #[test]
    fn sigset_malformed_bodies_fail_decode() {
        // Truncated: kind + n=1 with no address bytes.
        let mut truncated = SigsetCount::KIND.to_be_bytes().to_vec();
        truncated.push(1);
        assert!(SigsetCount::decode(&truncated).is_err());

        // n vs length mismatch: n=2 but only one address.
        let mut mismatch = SigsetCount::KIND.to_be_bytes().to_vec();
        mismatch.push(2);
        mismatch.extend_from_slice(privkey_addr(1).as_ref());
        assert!(SigsetCount::decode(&mismatch).is_err());

        // Extra trailing byte: type decode reports leftover; exact action_call fails.
        let mut extra = SigsetCount::new(keys(vec![privkey_addr(1)])).encode();
        let exact = extra.len();
        extra.push(0xff);
        let (_, used) = SigsetCount::decode(&extra).unwrap();
        assert_eq!(used, exact);
        let reg = TestRegistry::protocol().unwrap();
        let (decoded, used) = reg.decode_action(&extra).unwrap();
        assert_eq!(used, exact);
        assert_eq!(decoded.kind(), SigsetCount::KIND);
        assert!(reg.decode_action_exact(&extra).is_err());

        // Unsupported address version fails in Address::decode.
        let mut bad_ver = [0u8; Address::SIZE];
        bad_ver[0] = 2;
        bad_ver[1] = 1;
        let mut bad = SigsetCount::KIND.to_be_bytes().to_vec();
        bad.push(1);
        bad.extend_from_slice(&bad_ver);
        assert!(SigsetCount::decode(&bad).is_err());
        assert!(Address::decode(&bad_ver).is_err());
    }

    #[test]
    fn sigset_empty_n51_and_n201_are_legal_wire() {
        // Execute-layer max is SIGSET_MAX=200; n=51 and n=201 remain legal on the wire.
        let empty = AddressW1::from(vec![]).unwrap();
        assert_eq!(empty.encode(), vec![0]);
        assert!(SigsetCount::decode(&SigsetCount::new(empty).encode()).is_ok());

        let n51 = keys((1u8..=51).map(privkey_addr).collect());
        assert!(SigsetCount::decode(&SigsetCount::new(n51.clone()).encode()).is_ok());
        assert!(SigsetAtLeast::decode(&SigsetAtLeast::new(n51, Uint1::from(1)).encode()).is_ok());

        let n201 = keys((1u8..=201).map(privkey_addr).collect());
        let count201 = SigsetCount::new(n201.clone());
        let (decoded, used) = SigsetCount::decode(&count201.encode()).unwrap();
        assert_eq!(used, count201.encode().len());
        assert_eq!(decoded.keys.length(), 201);
        let at_least201 = SigsetAtLeast::new(n201, Uint1::from(1));
        assert!(SigsetAtLeast::decode(&at_least201.encode()).is_ok());
    }

    fn table(entries: &[(u64, u64)]) -> AssetAmtW1 {
        AssetAmtW1::from(
            entries
                .iter()
                .map(|(serial, expected)| AssetAmt {
                    serial: Fold64::from(*serial).unwrap(),
                    amount: Fold64::from(*expected).unwrap(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn balance_table_check_names_kind_scope_and_wire() {
        assert_eq!(BalanceTableCheck::KIND, 0x0605);
        assert_eq!(BalanceTableCheck::NAME, "balance_table_check");
        assert_eq!(BalanceTableCheck::SCOPE, ActScope::CALL_ONLY);

        let action =
            BalanceTableCheck::new(privkey_addr(1), table(&[(1, 100), (11, 7)]));
        let wire = action.encode();
        // kind(u16) + addr(21) + count(u8) + per-entry fold64(serial)+fold64(amount).
        let entries_bytes: usize = [(1u64, 100u64), (11, 7)]
            .iter()
            .map(|(s, v)| Fold64::from(*s).unwrap().size() + Fold64::from(*v).unwrap().size())
            .sum();
        assert_eq!(wire.len(), 2 + 21 + 1 + entries_bytes);
        assert_eq!(action.size(), wire.len());
        let (decoded, used) = BalanceTableCheck::decode(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);
        assert_eq!(decoded.table.length(), 2);
        assert_eq!(decoded.table.as_list()[0].amount.uint(), 100);

        let empty = BalanceTableCheck::new(privkey_addr(1), table(&[]));
        assert_eq!(empty.size(), 2 + 21 + 1);

        // Same table -> identical wire (deterministic gas); a bigger table is
        // charged strictly more (linear in entries).
        let again = BalanceTableCheck::new(privkey_addr(1), table(&[(1, 100), (11, 7)]));
        assert_eq!(again.encode(), wire);
        let bigger = BalanceTableCheck::new(
            privkey_addr(1),
            table(&[(1, 100), (11, 7), (12, 9)]),
        );
        assert!(bigger.size() > action.size());

        // The u8 count prefix bounds the table; serial 0 fails at wire decode
        // (AssetAmt::checked), not at typed construction.
        let zero_serial = BalanceTableCheck::new(
            privkey_addr(1),
            AssetAmtW1::from(vec![AssetAmt {
                serial: Fold64::from(0).unwrap(),
                amount: Fold64::from(1).unwrap(),
            }])
            .unwrap(),
        );
        assert!(BalanceTableCheck::decode(&zero_serial.encode()).is_err());
    }
}
