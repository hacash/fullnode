//! Execute-adjacent protocol facts that do not need ledger state.
//!
//! The execute path gates on the first finding; the SDK reports the full list
//! and never refuses to inspect or construct because of them. Message strings
//! are owned here so inspect findings and execute errors cannot drift.

use base::Transaction;
use field::{Address, Encode};

use crate::codec::tx::{TransactionType1, TransactionType2, TransactionType3};

/// Envelope / height / flag findings for one transaction body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduleFacts {
    pub findings: Vec<String>,
}

fn wire_gas_max(tx: &dyn Transaction) -> Option<u8> {
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType1>() {
        return Some(t.gas_max.uint());
    }
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType2>() {
        return Some(t.gas_max.uint());
    }
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType3>() {
        return Some(t.gas_max.uint());
    }
    None
}

fn wire_ano_mark(tx: &dyn Transaction) -> Option<u8> {
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType1>() {
        return Some(t.ano_mark[0]);
    }
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType2>() {
        return Some(t.ano_mark[0]);
    }
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType3>() {
        return Some(t.ano_mark[0]);
    }
    None
}

/// Type 1/2 carry a `gas_max` byte on the wire but execute requires it to be 0.
pub fn gas_max_finding_with_params(
    params: &hacash_params::ProtocolParams,
    ty: u8,
    gas_max: u8,
) -> Option<String> {
    if ty != params.tx_type_3 && gas_max != 0 {
        Some(format!("tx type {ty} gas_max must be zero"))
    } else {
        None
    }
}

pub fn ano_mark_finding(ty: u8, ano_mark: u8) -> Option<String> {
    if ano_mark != 0 {
        Some(format!("tx type {ty} ano_mark must be zero"))
    } else {
        None
    }
}

pub fn type1_deprecated_finding_with_params(
    params: &hacash_params::ProtocolParams,
    ty: u8,
    height: u64,
) -> Option<String> {
    if height > params.type1_deprecated_after_height && ty <= params.tx_type_1 {
        Some("Type 1 transactions have been deprecated after height 33,033".to_owned())
    } else {
        None
    }
}

pub fn fee_size_finding_with_params(
    params: &hacash_params::ProtocolParams,
    fee_size: usize,
    height: u64,
) -> Option<String> {
    if height > params.fee_size_limit_after_height
        && fee_size > params.max_fee_size_after_limit_height
    {
        Some("tx fee size cannot exceed 6 bytes when block height above 200,000".to_owned())
    } else {
        None
    }
}

pub fn main_address_findings(main: Address) -> Vec<String> {
    let mut findings = Vec::new();
    if !main.is_privkey() {
        findings.push("tx fee address version must be PRIVAKEY type".to_owned());
    }
    if main.is_privkey_unknown() {
        findings.push(format!(
            "tx main address {} is a system address with unknown private key",
            main.to_readable()
        ));
    }
    findings
}

pub fn addr_version_findings(addrs: &[Address]) -> Vec<String> {
    addrs
        .iter()
        .filter(|addr| !addr.is_supported())
        .map(|addr| format!("address version {} not supported", addr.version()))
        .collect()
}

pub fn activation_finding(ty: u8, need: u64, flags: u64) -> Option<String> {
    if need & !flags != 0 {
        Some(format!("tx type {ty} not activated (flags need {need:#x})"))
    } else {
        None
    }
}

/// Analyse execute-adjacent envelope rules without gating.
///
/// - `height = None` skips height-scheduled rules (type-1 deprecation, fee size).
/// - `flags = None` skips activation; the item is not treated as activated or
///   deactivated — it is simply not judged.
pub fn schedule_facts(
    tx: &dyn Transaction,
    height: Option<u64>,
    flags: Option<u64>,
) -> ScheduleFacts {
    schedule_facts_with_params(&hacash_params::MAINNET_PARAMS.protocol, tx, height, flags)
}

pub fn schedule_facts_with_params(
    params: &hacash_params::ProtocolParams,
    tx: &dyn Transaction,
    height: Option<u64>,
    flags: Option<u64>,
) -> ScheduleFacts {
    let mut findings = Vec::new();
    findings.extend(main_address_findings(tx.main()));
    findings.extend(addr_version_findings(&tx.addrs()));
    if let Some(gas_max) = wire_gas_max(tx) {
        if let Some(note) = gas_max_finding_with_params(params, tx.ty(), gas_max) {
            findings.push(note);
        }
    }
    if let Some(ano_mark) = wire_ano_mark(tx) {
        if let Some(note) = ano_mark_finding(tx.ty(), ano_mark) {
            findings.push(note);
        }
    }
    if let Some(height) = height {
        if let Some(note) = type1_deprecated_finding_with_params(params, tx.ty(), height) {
            findings.push(note);
        }
        if let Some(note) = fee_size_finding_with_params(params, Encode::size(tx.fee()), height) {
            findings.push(note);
        }
    }
    if let Some(flags) = flags {
        if let Some(note) = activation_finding(tx.ty(), tx.required_flags(), flags) {
            findings.push(note);
        }
    }
    ScheduleFacts { findings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::action::HacToTrs;
    use crate::codec::tx::{TransactionType1, TransactionType2};
    use base::{ActionRef, TransactionBuild};
    use field::{Amount, Timestamp, Uint1};
    use std::sync::Arc;

    fn transfer() -> ActionRef {
        Arc::new(HacToTrs::new(
            Address::from(*sys::Account::create_by("123456").unwrap().address()),
            Amount::from("1:244").unwrap(),
        ))
    }

    fn main_addr() -> Address {
        Address::from(*sys::Account::create_by("123456").unwrap().address())
    }

    #[test]
    fn type2_nonzero_gas_max_is_a_finding_not_a_constructor_error() {
        let mut body = TransactionType2 {
            ty: Uint1::from(hacash_params::TX_TYPE_2),
            timestamp: Timestamp::from(1),
            addrlist: field::AddrOrList::from_addr(main_addr()),
            fee: Amount::from("1:244").unwrap(),
            actions: Vec::new(),
            signs: field::SignW2::default(),
            gas_max: Uint1::from(10),
            ano_mark: field::Fixed1::default(),
        };
        body.push_action(transfer()).unwrap();
        let facts = schedule_facts(&body, None, None);
        assert!(
            facts
                .findings
                .iter()
                .any(|f| f.contains("gas_max must be zero")),
            "{:?}",
            facts.findings
        );
    }

    #[test]
    fn type1_deprecated_only_with_height() {
        let mut body = TransactionType1 {
            ty: Uint1::from(hacash_params::TX_TYPE_1),
            timestamp: Timestamp::from(1),
            addrlist: field::AddrOrList::from_addr(main_addr()),
            fee: Amount::from("1:244").unwrap(),
            actions: Vec::new(),
            signs: field::SignW2::default(),
            gas_max: Uint1::from(0),
            ano_mark: field::Fixed1::default(),
        };
        body.push_action(transfer()).unwrap();
        assert!(
            schedule_facts(&body, None, None)
                .findings
                .iter()
                .all(|f| !f.contains("deprecated")),
        );
        let facts = schedule_facts(&body, Some(40_000), None);
        assert!(
            facts.findings.iter().any(|f| f.contains("deprecated")),
            "{:?}",
            facts.findings
        );
    }
}
