//! ChainAllow / HeightScope / BalanceFloor / ReqSignList guard actions.

use std::collections::HashSet;
use std::sync::Arc;

use base::{ActScope, Action, ActionExecute, ActionRef, AddrOrPtr, CoreState, Transaction};
use field::{
    Amount, AssetAmt, AssetAmtW1, BlockHeight, ChainIDList, Decode, DiamondNumber, Encode, Reader,
    Satoshi, ToJSON, Uint2, json_decode_value, json_split_array, json_split_object,
};
use sys::{Rerr, Ret, errf};

use super::common::{
    addr_or_ptr_readable, addr_or_ptr_size, check_action_kind, decode_addr_or_ptr,
    encode_addr_or_ptr,
};

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ChainAllow {
    pub kind: Uint2,
    pub chains: ChainIDList,
}

impl field::ToJSON for ReqSignList {
    fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
        let signers = self
            .signers
            .iter()
            .map(|signer| field::ToJSON::to_json_fmt(signer, fmt))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"kind\":{},\"signers\":[{}]}}",
            field::ToJSON::to_json_fmt(&self.kind, fmt),
            signers
        )
    }
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HeightScope {
    pub kind: Uint2,
    pub start: BlockHeight,
    pub end: BlockHeight,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct BalanceFloor {
    pub kind: Uint2,
    pub addr: AddrOrPtr,
    pub hacash: Amount,
    pub satoshi: Satoshi,
    pub diamond: DiamondNumber,
    pub assets: AssetAmtW1,
}

impl ChainAllow {
    pub const KIND: u16 = 0x0411;

    pub fn new(chains: ChainIDList) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            chains,
        }
    }
}

impl HeightScope {
    pub const KIND: u16 = 0x0412;

    pub fn new(start: BlockHeight, end: BlockHeight) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            start,
            end,
        }
    }
}

impl BalanceFloor {
    pub const KIND: u16 = 0x0413;

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

/// Explicit extra required signers beyond intrinsic action req_sign.
/// Type3 uses this as E in D = R0 ∪ E (exact SignW2 match).
#[derive(Debug, Clone)]
pub struct ReqSignList {
    pub kind: Uint2,
    pub signers: Vec<AddrOrPtr>,
}

impl ReqSignList {
    pub const KIND: u16 = 0x0414;

    pub fn create_by(signers: Vec<AddrOrPtr>) -> Ret<Self> {
        if signers.is_empty() {
            return errf!("ReqSignList cannot be empty");
        }
        // The native wire count is Uint2; validate before storing the list so
        // every constructor rejects values that would otherwise truncate in
        // `Encode::encode_to`.
        Uint2::from_usize(signers.len())?;
        Ok(Self {
            kind: Uint2::from(Self::KIND),
            signers,
        })
    }

    pub fn create_by_addrs(addrs: Vec<field::Address>) -> Ret<Self> {
        let ptrs = addrs.into_iter().map(AddrOrPtr::Addr).collect();
        Self::create_by(ptrs)
    }

    /// Resolve and validate E: non-empty, unique, PRIVAKEY, not unknown system.
    pub fn validate_against(&self, addrs: &[field::Address]) -> Ret<HashSet<field::Address>> {
        if self.signers.is_empty() {
            return errf!("ReqSignList cannot be empty");
        }
        let mut e = HashSet::new();
        for ptr in &self.signers {
            let adr = ptr.real(addrs)?;
            if !adr.is_privkey() {
                return errf!(
                    "ReqSignList address {} must be PRIVAKEY type",
                    adr.to_readable()
                );
            }
            if adr.is_privkey_unknown() {
                return errf!(
                    "ReqSignList address {} is a system address with unknown private key",
                    adr.to_readable()
                );
            }
            if !e.insert(adr) {
                return errf!("ReqSignList address {} is duplicated", adr.to_readable());
            }
        }
        Ok(e)
    }
}

fn parse_req_sign_list_json(json: &str) -> Ret<ReqSignList> {
    let mut declared = Uint2::from(ReqSignList::KIND);
    let mut signers_json = None;
    let mut seen = HashSet::new();
    for (key, value) in json_split_object(json)? {
        if !seen.insert(key) {
            return sys::normalf!("ReqSignList JSON field {} is duplicated", key);
        }
        match key {
            "kind" => declared = json_decode_value(value)?,
            "signers" => signers_json = Some(value),
            _ => {}
        }
    }
    if declared.uint() != ReqSignList::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            ReqSignList::KIND,
            declared.uint()
        );
    }
    let raw = signers_json.ok_or_else(|| sys::Error::normal("ReqSignList JSON missing signers"))?;
    let mut signers = Vec::new();
    for value in json_split_array(raw)? {
        signers.push(json_decode_value(value)?);
    }
    Ok(ReqSignList::create_by(signers)?)
}

impl field::FromJSON for ReqSignList {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = parse_req_sign_list_json(json)?;
        Ok(())
    }
}

impl base::ActionJsonCodec for ReqSignList {
    fn decode_json(json: &str) -> Ret<Self> {
        parse_req_sign_list_json(json)
    }
}

/// Registry JSON decoder for the dynamic signer-list action.
pub fn decode_req_sign_list_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    if kind != ReqSignList::KIND {
        return sys::normalf!("ReqSignList JSON codec got kind {}", kind);
    }
    Ok(Arc::new(parse_req_sign_list_json(json)?))
}

impl Encode for ReqSignList {
    fn size(&self) -> usize {
        self.kind.size() + Uint2::SIZE + self.signers.iter().map(addr_or_ptr_size).sum::<usize>()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        // `create_by` validates the Uint2 bound. Keep this path defensive for
        // values assembled through struct literals in consensus code.
        Uint2::from_usize(self.signers.len())
            .expect("ReqSignList signer count exceeds Uint2")
            .encode_to(out);
        for ptr in &self.signers {
            encode_addr_or_ptr(ptr, out);
        }
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
    let mut seen = std::collections::HashSet::new();
    for ast in assets.as_list() {
        let serial = ast.serial.uint();
        let amount = ast.amount.uint();
        if serial == 0 {
            return errf!("balance floor asset serial cannot be zero");
        }
        if amount == 0 {
            return errf!("balance floor asset {} amount cannot be zero", serial);
        }
        if !seen.insert(serial) {
            return errf!("balance floor asset serial {} is duplicated", serial);
        }
    }
    Ok(())
}

/// Structural validation of a `BalanceFloor` (no chain state): negative
/// amount, malformed asset list, empty floor. Shared by `execute` (revert
/// semantics) and `guard_facts` (review facts); returns the per-asset check
/// flags the execute body needs.
fn validate_balance_floor_struct(floor: &BalanceFloor) -> Ret<(bool, bool, bool, bool)> {
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

base::impl_action! {
    ChainAllow {
        name: "chain_allow",
        scope: ActScope::GUARD,
        min_tx_type: 2,
        description: |this: &ChainAllow| format!("Valid chain ID list {}", this.chains.as_list().iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")),
        execute: (self, ctx) {

        let cid = ctx.env().chain.id;
        if !self
            .chains
            .as_list()
            .iter()
            .any(|id| id.uint() == cid.get())
        {
            let cids = self
                .chains
                .as_list()
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            return sys::revertf!(
                "transaction must belong to chains {} but on chain {}",
                cids,
                cid
            );
        }
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    HeightScope {
        name: "height_scope",
        scope: ActScope::GUARD,
        min_tx_type: 2,
        description: |this: &HeightScope| format!("Limit height range ({}, {})", this.start.uint(), if this.end.uint() == 0 { "Unlimited".to_owned() } else { this.end.uint().to_string() }),
        execute: (self, ctx) {

        let left = self.start.uint();
        let right = match self.end.uint() {
            0 => u64::MAX,
            h => h,
        };
        if left > right {
            return errf!("left height {} cannot exceed right height {}", left, right);
        }
        let height = ctx.env().block.height;
        if height < left || height > right {
            return sys::revertf!(
                "transaction must be submitted in height between {} and {}",
                left,
                right
            );
        }
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    BalanceFloor {
        name: "balance_floor",
        scope: ActScope::GUARD,
        min_tx_type: 2,
        description: |this: &BalanceFloor| format!("Balance floor for {} (hac={}, sat={}, dia={}, assets={})", addr_or_ptr_readable(&this.addr), this.hacash, this.satoshi.uint(), this.diamond.uint(), this.assets.length()),
        execute: (self, ctx) {

        let (check_hac, check_sat, check_dia, _check_assets) = validate_balance_floor_struct(self)?;
        let addr = ctx.addr(&self.addr)?;
        let balance = CoreState::wrap(ctx.layer())
            .balance(&addr)?
            .unwrap_or_default();
        if check_hac && balance.hacash < self.hacash {
            return sys::revertf!(
                "address {} hacash {} is lower than floor {}",
                addr.to_json(),
                balance.hacash,
                self.hacash
            );
        }
        if check_sat {
            let sat = balance.satoshi.to_satoshi();
            if sat < self.satoshi {
                return sys::revertf!(
                    "address {} satoshi {} is lower than floor {}",
                    addr.to_json(),
                    sat,
                    self.satoshi
                );
            }
        }
        if check_dia {
            let dia = balance.diamond.to_diamond()?;
            if dia < self.diamond {
                return sys::revertf!(
                    "address {} diamond {} is lower than floor {}",
                    addr.to_json(),
                    dia,
                    self.diamond
                );
            }
        }
        for floor in self.assets.as_list() {
            let cur = balance
                .asset(floor.serial)
                .unwrap_or(AssetAmt::from_serial(floor.serial)?);
            if cur.amount < floor.amount {
                return sys::revertf!(
                    "address {} asset {}:{} is lower than floor {}:{}",
                    addr.to_json(),
                    cur.serial,
                    cur.amount,
                    floor.serial,
                    floor.amount
                );
            }
        }
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    ReqSignList {
        name: "req_sign_list",
        scope: ActScope::TOP_GUARD_UNIQUE,
        min_tx_type: 2,
        extra9: |_: &ReqSignList| false,
        req_sign: |this: &ReqSignList| this.signers.clone(),
        as_transfer_like: none,
        description: |this: &ReqSignList| format!("Require extra signers ({})", this.signers.len()),
        execute: (self, ctx) {

        self.validate_against(&ctx.env().tx.addrs)?;
        Ok(vec![])
        
        }
    }
}

pub fn create_chain_guard_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    let mut r = Reader::new(buf);
    let kind_field: Uint2 = r.read()?;
    if kind_field.uint() != kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            kind,
            kind_field.uint()
        );
    }
    match kind {
        ChainAllow::KIND => decode_regular_guard_action::<ChainAllow>(buf),
        HeightScope::KIND => decode_regular_guard_action::<HeightScope>(buf),
        BalanceFloor::KIND => decode_regular_guard_action::<BalanceFloor>(buf),
        ReqSignList::KIND => {
            let count: Uint2 = r.read()?;
            let mut signers = Vec::with_capacity(count.uint() as usize);
            for _ in 0..count.uint() {
                let (ptr, used) = decode_addr_or_ptr(&buf[r.used()..])?;
                let _ = r.read_bytes(used)?;
                signers.push(ptr);
            }
            Ok((
                Arc::new(ReqSignList {
                    kind: kind_field,
                    signers,
                }),
                r.used(),
            ))
        }
        _ => sys::normalf!("chain guard action kind {} not registered", kind),
    }
}

#[cfg(feature = "execute")]
fn decode_regular_guard_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + ActionExecute + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}

#[cfg(not(feature = "execute"))]
fn decode_regular_guard_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}

// ================================ wire schema ================================

impl base::ActionSchemaProvider for ReqSignList {
    const ACTION_SCHEMA: base::ActionSchema = base::ActionSchema {
        kind: Self::KIND,
        name: "req_sign_list",
        audit_class: "full",
        blob: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("signers", base::FieldWire::ListW2("AddrOrPtr")),
        ],
    };
}

// ================================ review facts ================================

/// Facts of the guard actions in one transaction: the *effective* allowed
/// chains and height range (the protocol executes every guard action
/// independently, so the effective chain set is the intersection of all
/// `ChainAllow` lists and the effective height range is the intersection of
/// all `HeightScope` ranges), plus per-action notes and structural protocol
/// violations. This is the single analysis shared by the full node and the
/// SDK review; state-dependent rules stay in the `execute` bodies. A guard
/// kind without a facts arm is reported as a note instead of being silently
/// dropped, so a new guard action never disappears from the review.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardFacts {
    /// Effective allowed chain set (intersection of all `ChainAllow`
    /// actions); `None` when no `ChainAllow` action is present.
    pub chains: Option<Vec<u32>>,
    /// Effective height range `(start, end)` (intersection of all
    /// `HeightScope` actions); `None` when no `HeightScope` action is
    /// present. `end == 0` means unlimited (the collector's internal
    /// `u64::MAX` sentinel is rewritten to `0` before facts are returned).
    pub height_range: Option<(u64, u64)>,
    /// (action index, note) pairs attached to the matching action descriptor.
    pub action_notes: Vec<(usize, String)>,
    /// Protocol-level violations: signing must be rejected, decode stays ok.
    pub protocol_violations: Vec<String>,
}

impl GuardFacts {
    /// Evaluate collected facts against a caller-provided height and chain id.
    /// Returns `(expired_height, wrong_chain)`. Uses the same predicates the
    /// chain's HeightScope / ChainAllow execute bodies use (`height_in_range`
    /// and set membership). A missing fact means that check is inactive
    /// (not expired / not on the wrong chain).
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

/// Height-range membership predicate of the `HeightScope` guard action:
/// `end == 0` means unlimited and `start > end` with a non-zero `end` is an
/// unsatisfiable range (never in range). This is the single implementation —
/// the SDK review facts and any chain-side guard check both call it, so the
/// semantics can never be re-derived differently.
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
            ReqSignList::KIND => {
                if let Some(list) = action.as_any().downcast_ref::<ReqSignList>() {
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
            kind if action.scope() == ActScope::GUARD
                    || action.scope() == ActScope::TOP_GUARD_UNIQUE =>
            {
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
    use crate::codec::tx::TransactionType2;
    use field::{Amount, BlockHeight, ChainIDList, DiamondNumber, Satoshi, Uint4};

    fn main_address() -> field::Address {
        let account = sys::Account::create_by("123456").unwrap();
        field::Address::from(*account.address())
    }

    fn sample_tx() -> TransactionType2 {
        let main = main_address();
        TransactionType2::new_by(main, Amount::from("1:244").unwrap(), 0)
    }

    /// Every standard guard kind must produce a specific fact (no "no review
    /// facts" note): chains intersection, height range, signer validation and
    /// balance-floor structural validation.
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
        tx.push_action_in(Arc::new(ReqSignList::create_by_addrs(vec![main]).unwrap()));
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
        let error = ReqSignList::create_by(vec![signer; u16::MAX as usize + 1]).unwrap_err();
        assert!(error.to_string().contains("65535"), "{error}");
    }
}
