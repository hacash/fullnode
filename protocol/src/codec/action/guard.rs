//! ChainAllow / HeightScope / BalanceFloor / RequiredSigners guard actions.

#[cfg(test)]
use std::sync::Arc;

use base::{AddrOrPtr, Transaction};
use field::{Amount, AssetAmtW1, BlockHeight, ChainIDList, DiamondNumber, ListW2, Satoshi, Uint2};
use sys::{Rerr, Ret, errf};

use super::transfer::addr_or_ptr_readable;

base::action_simple! { ChainAllow, 0x0411, 2, GUARD, {
    chains: ChainIDList
}, this, {
    ctor: manual,
    description: format!("Valid chain ID list {}", this.chains.as_list().iter().map(|id| id.to_string()).collect::<Vec<_>>().join(","))
}}

base::action_simple! { HeightScope, 0x0412, 2, GUARD, {
    start: BlockHeight,
    end: BlockHeight
}, this, {
    ctor: manual,
    description: format!("Limit height range ({}, {})", this.start.uint(), if this.end.uint() == 0 { "Unlimited".to_owned() } else { this.end.uint().to_string() })
}}

base::action_simple! { BalanceFloor, 0x0413, 2, GUARD, {
    addr: AddrOrPtr,
    hacash: Amount,
    satoshi: Satoshi,
    diamond: DiamondNumber,
    assets: AssetAmtW1
}, this, {
    ctor: manual,
    description: format!("Balance floor for {} (hac={}, sat={}, dia={}, assets={})", addr_or_ptr_readable(&this.addr), this.hacash, this.satoshi.uint(), this.diamond.uint(), this.assets.length())
}}

impl ChainAllow {
    pub fn new(chains: ChainIDList) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            chains,
        }
    }
}

impl HeightScope {
    pub fn new(start: BlockHeight, end: BlockHeight) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            start,
            end,
        }
    }
}

impl BalanceFloor {
    pub fn new(addr: AddrOrPtr, hacash: Amount, satoshi: Satoshi, diamond: DiamondNumber) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            addr,
            hacash,
            satoshi,
            diamond,
            assets: AssetAmtW1::default(),
        }
    }
}

// Explicit extra required signers beyond intrinsic action req_sign.
// Type3 uses this as E in D = R0 ∪ E (exact SignW2 match).
base::action_simple! { RequiredSigners, 0x0414, 2, TOP_GUARD_UNIQUE, {
    signers: ListW2<AddrOrPtr>
}, this, {
    validate: "Self::validate_codec",
    req_sign: this.signers.0.clone(),
    ctor: none,
    description: format!("Require extra signers ({})", this.signers.length())
}}

impl RequiredSigners {
    pub fn create_by(signers: Vec<AddrOrPtr>) -> Ret<Self> {
        let value = Self {
            kind: Uint2::from(Self::KIND),
            signers: ListW2::from(signers)?,
        };
        value.validate_codec()?;
        Ok(value)
    }

    pub fn create_by_addrs(addrs: Vec<field::Address>) -> Ret<Self> {
        let ptrs = addrs.into_iter().map(AddrOrPtr::Addr).collect();
        Self::create_by(ptrs)
    }

    /// Resolve and validate E: non-empty, unique, PRIVAKEY, not unknown system.
    /// Signer lists are short; a `Vec` with a linear duplicate scan keeps the
    /// hash-table machinery out of the wasm graph.
    pub fn validate_against(&self, addrs: &[field::Address]) -> Ret<Vec<field::Address>> {
        if self.signers.0.is_empty() {
            return errf!("RequiredSigners cannot be empty");
        }
        let mut e: Vec<field::Address> = Vec::new();
        for ptr in self.signers.as_list() {
            let adr = ptr.real(addrs)?;
            if !adr.is_privkey() {
                return errf!(
                    "RequiredSigners address {} must be PRIVAKEY type",
                    adr.to_readable()
                );
            }
            if adr.is_privkey_unknown() {
                return errf!(
                    "RequiredSigners address {} is a system address with unknown private key",
                    adr.to_readable()
                );
            }
            if e.contains(&adr) {
                return errf!(
                    "RequiredSigners address {} is duplicated",
                    adr.to_readable()
                );
            }
            e.push(adr);
        }
        Ok(e)
    }

    fn validate_codec(&self) -> Ret<()> {
        if self.signers.0.is_empty() {
            return errf!("RequiredSigners cannot be empty");
        }
        Ok(())
    }
}

fn check_balance_floor_assets(assets: &AssetAmtW1) -> Rerr {
    const BALANCE_ASSET_MAX: usize = 20;
    if assets.length() > BALANCE_ASSET_MAX {
        return errf!(
            "balance floor assets item quantity cannot exceed {}",
            BALANCE_ASSET_MAX
        );
    }
    let mut seen: Vec<u64> = Vec::new();
    for ast in assets.as_list() {
        let serial = ast.serial.uint();
        let amount = ast.amount.uint();
        if serial == 0 {
            return errf!("balance floor asset serial cannot be zero");
        }
        if amount == 0 {
            return errf!("balance floor asset {} amount cannot be zero", serial);
        }
        if seen.contains(&serial) {
            return errf!("balance floor asset serial {} is duplicated", serial);
        }
        seen.push(serial);
    }
    Ok(())
}

/// Structural (chain-state-free) validation of a `BalanceFloor` (negative amount,
/// malformed asset list, empty floor); shared by `execute`/`guard_facts`, returns the per-asset check flags.
pub(crate) fn validate_balance_floor_struct(floor: &BalanceFloor) -> Ret<(bool, bool, bool, bool)> {
    if floor.hacash.is_negative() {
        return errf!("balance floor hacash {} cannot be negative", floor.hacash);
    }
    check_balance_floor_assets(&floor.assets)?;
    let check_hac = !floor.hacash.is_zero();
    let check_sat = floor.satoshi.uint() > 0;
    let check_dia = floor.diamond.uint() > 0;
    let check_assets = floor.assets.length() > 0;
    if !(check_hac || check_sat || check_dia || check_assets) {
        return errf!("balance floor is empty");
    }
    Ok((check_hac, check_sat, check_dia, check_assets))
}

// ================================ review facts ================================

/// Effective guard-action facts for one transaction: chains/height-range are the
/// intersection of all `ChainAllow`/`HeightScope` actions. Single analysis shared by node and SDK review; unknown guard kinds surface as notes, never silently dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardFacts {
    /// Effective allowed chain set (intersection of all `ChainAllow`
    /// actions); `None` when no `ChainAllow` action is present.
    pub chains: Option<Vec<u32>>,
    /// Effective height range `(start, end)` — intersection of all `HeightScope`
    /// actions; `None` when none present, `end == 0` means unlimited.
    pub height_range: Option<(u64, u64)>,
    /// (action index, note) pairs attached to the matching action descriptor.
    pub action_notes: Vec<(usize, String)>,
    /// Protocol-level violations: signing must be rejected, decode stays ok.
    pub protocol_violations: Vec<String>,
}

impl GuardFacts {
    /// Evaluate facts against a height and chain id → `(expired_height, wrong_chain)`,
    /// using the same predicates as the execute bodies; a missing fact = check inactive.
    pub fn against_context(&self, current_height: u64, expected_chain_id: u32) -> (bool, bool) {
        let expired = self.height_range.map_or(false, |(start, end)| {
            !height_in_range(start, end, current_height)
        });
        let wrong = self
            .chains
            .as_ref()
            .map_or(false, |chains| !chains.contains(&expected_chain_id));
        (expired, wrong)
    }
}

fn push_guard_note(facts: &mut GuardFacts, index: usize, text: String) {
    facts.protocol_violations.push(text.clone());
    facts.action_notes.push((index, text));
}

/// Height-range predicate of `HeightScope`: `end == 0` means unlimited, `start > end`
/// (non-zero `end`) is never in range. Single implementation shared by SDK review and chain checks.
pub fn height_in_range(start: u64, end: u64, height: u64) -> bool {
    if start > end && end != 0 {
        return false;
    }
    height >= start && (end == 0 || height <= end)
}

/// Single guard-facts analysis for a transaction (see `GuardFacts`).
pub fn guard_facts(tx: &dyn Transaction) -> GuardFacts {
    let mut facts = GuardFacts {
        chains: None,
        height_range: None,
        action_notes: Vec::new(),
        protocol_violations: Vec::new(),
    };
    for (index, action) in tx.actions().iter().enumerate() {
        match action.kind() {
            ChainAllow::KIND => {
                let Some(chain_allow) = action.as_any().downcast_ref::<ChainAllow>() else {
                    continue;
                };
                let allowed: Vec<u32> = chain_allow
                    .chains
                    .as_list()
                    .iter()
                    .map(|id| id.uint())
                    .collect();
                if allowed.is_empty() {
                    push_guard_note(
                        &mut facts,
                        index,
                        "chain_allow with empty chain list is protocol-invalid".to_owned(),
                    );
                    continue;
                }
                facts.chains = Some(match facts.chains.take() {
                    None => allowed,
                    Some(prev) => prev.into_iter().filter(|id| allowed.contains(id)).collect(),
                });
                if facts.chains.as_ref().is_some_and(|set| set.is_empty()) {
                    push_guard_note(
                        &mut facts,
                        index,
                        "chain_allow constraints conflict: no chain satisfies all of them"
                            .to_owned(),
                    );
                }
            }
            HeightScope::KIND => {
                let Some(scope) = action.as_any().downcast_ref::<HeightScope>() else {
                    continue;
                };
                let start = scope.start.uint();
                let end = scope.end.uint();
                if start > end && end != 0 {
                    push_guard_note(
                        &mut facts,
                        index,
                        format!("height_scope left {start} exceeds right {end}"),
                    );
                    continue;
                }
                let end = if end == 0 { u64::MAX } else { end };
                facts.height_range = Some(match facts.height_range {
                    None => (start, end),
                    Some(prev) => (prev.0.max(start), prev.1.min(end)),
                });
                if facts
                    .height_range
                    .as_ref()
                    .is_some_and(|range| range.0 > range.1)
                {
                    push_guard_note(
                        &mut facts,
                        index,
                        "height_scope constraints conflict: no height satisfies all of them"
                            .to_owned(),
                    );
                }
            }
            RequiredSigners::KIND => {
                if let Some(list) = action.as_any().downcast_ref::<RequiredSigners>() {
                    if let Err(error) = list.validate_against(&tx.addrs()) {
                        push_guard_note(&mut facts, index, error.to_string());
                    }
                }
            }
            BalanceFloor::KIND => {
                if let Some(floor) = action.as_any().downcast_ref::<BalanceFloor>() {
                    if let Err(error) = validate_balance_floor_struct(floor) {
                        push_guard_note(&mut facts, index, error.to_string());
                    }
                }
            }
            kind if action.scope().is_guard() => {
                push_guard_note(
                    &mut facts,
                    index,
                    format!("guard action kind {kind} has no review facts"),
                );
            }
            _ => {}
        }
    }
    // Convert the internal unlimited sentinel back to the wire's 0 form.
    if let Some(range) = &mut facts.height_range {
        if range.1 == u64::MAX {
            range.1 = 0;
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tx::StdTransaction;
    use field::{Amount, BlockHeight, ChainIDList, DiamondNumber, Satoshi, Uint4};

    fn main_address() -> field::Address {
        let account = sys::Account::create_by("123456").unwrap();
        field::Address::from(*account.address())
    }

    fn sample_tx() -> StdTransaction {
        let main = main_address();
        StdTransaction::new_by(
            hacash_params::TX_TYPE_2,
            main,
            Amount::from("1:244").unwrap(),
            0,
        )
    }

    /// Every standard guard kind must produce a specific fact (chains, height range,
    /// signer validation, balance-floor) — never a "no review facts" note.
    #[test]
    fn guard_facts_cover_every_standard_guard_kind() {
        let main = main_address();
        let mut tx = sample_tx();
        tx.push_action_in(Arc::new(ChainAllow::new(
            ChainIDList::from(vec![Uint4::from(1)]).unwrap(),
        )));
        tx.push_action_in(Arc::new(HeightScope::new(
            BlockHeight::from(100),
            BlockHeight::from(200),
        )));
        tx.push_action_in(Arc::new(
            RequiredSigners::create_by_addrs(vec![main]).unwrap(),
        ));
        tx.push_action_in(Arc::new(BalanceFloor::new(
            AddrOrPtr::Addr(main),
            Amount::from("1:244").unwrap(),
            Satoshi::from(100),
            DiamondNumber::from(1),
        )));

        let facts = guard_facts(&tx);
        assert!(
            facts.protocol_violations.is_empty(),
            "{:?}",
            facts.protocol_violations
        );
        assert_eq!(facts.chains, Some(vec![1]));
        assert_eq!(facts.height_range, Some((100, 200)));
        assert!(facts.action_notes.is_empty(), "{:?}", facts.action_notes);
    }

    /// ChainAllow intersection and conflicting pairs are the same analysis the
    /// strict inspect consumes.
    #[test]
    fn guard_facts_intersect_chain_allow_and_height_scope() {
        let mut tx = sample_tx();
        tx.push_action_in(Arc::new(ChainAllow::new(
            ChainIDList::from(vec![Uint4::from(0), Uint4::from(1)]).unwrap(),
        )));
        tx.push_action_in(Arc::new(ChainAllow::new(
            ChainIDList::from(vec![Uint4::from(1), Uint4::from(2)]).unwrap(),
        )));
        tx.push_action_in(Arc::new(HeightScope::new(
            BlockHeight::from(100),
            BlockHeight::from(0), // unlimited
        )));
        tx.push_action_in(Arc::new(HeightScope::new(
            BlockHeight::from(150),
            BlockHeight::from(300),
        )));

        let facts = guard_facts(&tx);
        assert!(
            facts.protocol_violations.is_empty(),
            "{:?}",
            facts.protocol_violations
        );
        assert_eq!(facts.chains, Some(vec![1]));
        assert_eq!(facts.height_range, Some((150, 300)));
        assert_eq!(facts.against_context(150, 1), (false, false));
        assert_eq!(facts.against_context(149, 1), (true, false));
        assert_eq!(facts.against_context(301, 1), (true, false));
        assert_eq!(facts.against_context(200, 2), (false, true));

        // A conflicting chain pair is a protocol fact (empty effective set).
        let mut tx = sample_tx();
        tx.push_action_in(Arc::new(ChainAllow::new(
            ChainIDList::from(vec![Uint4::from(0)]).unwrap(),
        )));
        tx.push_action_in(Arc::new(ChainAllow::new(
            ChainIDList::from(vec![Uint4::from(1)]).unwrap(),
        )));
        let facts = guard_facts(&tx);
        assert_eq!(facts.chains, Some(vec![]));
        assert_eq!(facts.protocol_violations.len(), 1);
    }

    /// A structurally invalid BalanceFloor is a review violation, not a
    /// decode failure (the execute body applies the same structural rules).
    #[test]
    fn guard_facts_report_invalid_balance_floor_struct() {
        let main = main_address();
        let mut tx = sample_tx();
        tx.push_action_in(Arc::new(BalanceFloor::new(
            AddrOrPtr::Addr(main),
            Amount::from("-1:244").unwrap(),
            Satoshi::from(100),
            DiamondNumber::from(1),
        )));
        let facts = guard_facts(&tx);
        assert_eq!(facts.protocol_violations.len(), 1);
        assert!(facts.protocol_violations[0].contains("cannot be negative"));
        assert_eq!(facts.action_notes.len(), 1);
        assert_eq!(facts.action_notes[0].0, 0);
    }

    #[test]
    fn guard_facts_against_context_treats_unlimited_end_as_open() {
        let mut tx = sample_tx();
        tx.push_action_in(Arc::new(HeightScope::new(
            BlockHeight::from(100),
            BlockHeight::from(0),
        )));
        let facts = guard_facts(&tx);
        assert_eq!(facts.height_range, Some((100, 0)));
        assert_eq!(facts.against_context(100, 0), (false, false));
        assert_eq!(facts.against_context(99, 0), (true, false));
        assert_eq!(facts.against_context(u64::MAX, 0), (false, false));
    }

    #[test]
    fn req_sign_list_rejects_count_that_would_truncate() {
        let signer = AddrOrPtr::Addr(main_address());
        let error = RequiredSigners::create_by(vec![signer; u16::MAX as usize + 1]).unwrap_err();
        assert!(error.to_string().contains("65535"), "{error}");
    }
}
