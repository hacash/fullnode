//! Registry

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "execute")]
use base::TransactionExecute;
use base::{
    ActionRef, BinaryCodecs, Transaction, TransactionBuild, TransactionSign, TxCreateRequest,
};
use field::{
    AddrOrList, Address, Amount, Encode, Fixed1, Fixed16, Hash, Reader, Sign, SignW2, Timestamp,
    Uint1, Uint2,
};
use sys::{Account, Rerr, Ret, errf};

use crate::codec::action::RequiredSigners;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultPreludeTx {
    pub ty: Uint1,
    pub address: Address,
    pub reward: Amount,
    pub message: Fixed16,
    pub miner_nonce: Hash,
}

impl Default for DefaultPreludeTx {
    fn default() -> Self {
        Self {
            ty: Uint1::from(Self::TYPE),
            address: Address::default(),
            reward: Amount::mei(1),
            message: Fixed16::default(),
            miner_nonce: Hash::default(),
        }
    }
}

impl DefaultPreludeTx {
    pub const TYPE: u8 = 0;

    pub fn new(address: Address, reward: Amount, message: Fixed16, miner_nonce: Hash) -> Self {
        Self {
            ty: Uint1::from(Self::TYPE),
            address,
            reward,
            message,
            miner_nonce,
        }
    }
}

impl Encode for DefaultPreludeTx {
    fn size(&self) -> usize {
        self.ty.size()
            + self.address.size()
            + self.reward.size()
            + self.message.size()
            + self.miner_nonce.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.ty.encode_to(out);
        self.address.encode_to(out);
        self.reward.encode_to(out);
        self.message.encode_to(out);
        self.miner_nonce.encode_to(out);
    }
}

impl Transaction for DefaultPreludeTx {
    fn ty(&self) -> u8 {
        Self::TYPE
    }

    fn main(&self) -> Address {
        self.address
    }

    fn fee(&self) -> &Amount {
        Amount::zero_ref()
    }

    fn fee_pay(&self) -> Amount {
        Amount::zero()
    }

    fn fee_got(&self) -> Amount {
        Amount::zero()
    }

    fn author(&self) -> Option<Address> {
        Some(self.address)
    }

    fn block_reward(&self) -> Option<&Amount> {
        Some(&self.reward)
    }

    fn block_message(&self) -> Option<&Fixed16> {
        Some(&self.message)
    }

    fn fee_receiver(&self) -> Option<Address> {
        Some(self.address)
    }

    fn mempool_policy(&self) -> base::MempoolPolicy {
        base::MempoolPolicy::Forbidden
    }

    fn is_block_prelude(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl TransactionSign for DefaultPreludeTx {
    fn hash(&self) -> Hash {
        Hash::from(sys::calculate_hash(self.encode()))
    }

    fn verify_signature(&self) -> Rerr {
        errf!("cannot verify signature on prelude tx")
    }

    #[cfg(feature = "execute")]
    fn as_execute(&self) -> Option<&dyn TransactionExecute> {
        Some(self)
    }
}

impl TransactionBuild for DefaultPreludeTx {
    fn set_mining_nonce(&mut self, nonce: Hash) {
        self.miner_nonce = nonce;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TxHashMode {
    Legacy,
    Type3,
}

/// Standard user transaction (wire types 1/2/3). `ty` is a data field set at
/// construct/decode and written on encode; behavior branches on `ty.uint()`.
#[derive(Debug, Clone)]
pub struct StdTransaction {
    pub ty: Uint1,
    pub timestamp: Timestamp,
    pub addrlist: AddrOrList,
    pub fee: Amount,
    pub actions: Vec<ActionRef>,
    pub signs: SignW2,
    pub gas_max: Uint1,
    pub ano_mark: Fixed1,
}

fn std_tx_from_request(request: TxCreateRequest) -> Ret<StdTransaction> {
    match request.ty {
        hacash_params::TX_TYPE_1 | hacash_params::TX_TYPE_2 | hacash_params::TX_TYPE_3 => {
            Ok(StdTransaction {
                ty: Uint1::from(request.ty),
                timestamp: Timestamp::from(request.timestamp),
                addrlist: request.addrlist,
                fee: request.fee,
                actions: Vec::new(),
                signs: SignW2::default(),
                gas_max: Uint1::from(request.gas_max),
                ano_mark: Fixed1::default(),
            })
        }
        ty => errf!("unsupported standard user transaction type {}", ty),
    }
}

/// Create an empty standard user transaction by wire type; owns the concrete
/// type behind [`base::TransactionCreator`]. Type 1/2 `gas_max != 0` is wire-legal (execute rejects it); callers use `crate::facts::gas_max_finding`.
pub fn create_standard_transaction(request: TxCreateRequest) -> Ret<base::TxRef> {
    Ok(Arc::new(std_tx_from_request(request)?))
}

/// Push actions (up to the u16 wire count) and mechanically insert signatures;
/// digest/D-set acceptance is separate (`verify_signature` / `signature_report`).
fn fill_standard_tx(tx: &mut StdTransaction, actions: &[ActionRef], signs: &[Sign]) -> Rerr {
    for action in actions {
        tx.push_action(action.clone())?;
    }
    for sign in signs {
        tx.insert_sign(sign.clone())?;
    }
    Ok(())
}

/// Encode a standard transaction body (types 1/2/3). Consensus envelope rules are not constructor gates.
pub fn encode_standard_tx(
    request: TxCreateRequest,
    actions: &[ActionRef],
    signs: &[Sign],
) -> Ret<Vec<u8>> {
    let mut tx = std_tx_from_request(request)?;
    fill_standard_tx(&mut tx, actions, signs)?;
    Ok(tx.encode())
}

fn action_list_size(actions: &[ActionRef]) -> usize {
    Uint2::SIZE + actions.iter().map(|a| a.size()).sum::<usize>()
}

fn encode_action_list(actions: &[ActionRef], out: &mut Vec<u8>) {
    Uint2::from_usize(actions.len())
        .expect("action list length overflow")
        .encode_to(out);
    for act in actions {
        act.encode_to(out);
    }
}

fn decode_action_list(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(Vec<ActionRef>, usize)> {
    let mut r = Reader::new(buf);
    let count: Uint2 = r.read()?;
    let mut actions = Vec::with_capacity(count.uint() as usize);
    for _ in 0..count.uint() {
        let act = r.read_with(|rest| reg.decode_action(rest))?;
        actions.push(act);
    }
    Ok((actions, r.used()))
}

fn tx_hash(
    mode: TxHashMode,
    ty: &Uint1,
    timestamp: &Timestamp,
    addrlist: &AddrOrList,
    fee_bytes: &[u8],
    actions: &[ActionRef],
    gas_max: &Uint1,
    ano_mark: &Fixed1,
) -> Hash {
    let mut stuff = Vec::new();
    ty.encode_to(&mut stuff);
    timestamp.encode_to(&mut stuff);
    addrlist.encode_to(&mut stuff);
    stuff.extend_from_slice(fee_bytes);
    encode_action_list(actions, &mut stuff);
    if mode == TxHashMode::Type3 {
        gas_max.encode_to(&mut stuff);
        ano_mark.encode_to(&mut stuff);
    }
    Hash::from(sys::calculate_hash(stuff))
}

fn req_sign_for(main: Address, addrlist: &AddrOrList, actions: &[ActionRef]) -> Ret<Vec<Address>> {
    let addrs = addrlist.to_list();
    let mut required = vec![main];
    for act in actions {
        for ptr in act.req_sign() {
            let addr = ptr.real(&addrs)?;
            // Legacy: non-PRIVAKEY req_sign targets are not required to sign (a SCRIPTMH
            // `from` cannot sign); keep dev's rule or historical blocks fail verification on replay.
            if addr.is_privkey() && !required.contains(&addr) {
                required.push(addr);
            }
        }
    }
    Ok(required)
}

impl StdTransaction {
    pub const SIGN_ITEM_SIZE: usize = 97;

    pub fn new(ty: u8, main: Address, fee: Amount) -> Self {
        Self::new_by(ty, main, fee, 0)
    }

    pub fn new_by(ty: u8, main: Address, fee: Amount, ts: u64) -> Self {
        Self {
            ty: Uint1::from(ty),
            timestamp: Timestamp::from(ts),
            addrlist: AddrOrList::from_addr(main),
            fee,
            actions: Vec::new(),
            signs: SignW2::default(),
            gas_max: Uint1::default(),
            ano_mark: Fixed1::default(),
        }
    }

    pub fn push_action_in(&mut self, act: ActionRef) {
        self.try_push_action(act)
            .expect("tx action count exceeds u16 wire maximum");
    }

    fn try_push_action(&mut self, act: ActionRef) -> Rerr {
        Uint2::from_usize(self.actions.len() + 1)?;
        self.actions.push(act);
        Ok(())
    }

    pub fn fill_sign_account(&mut self, acc: &Account) -> Ret<Sign> {
        let fhx = if acc.address() == self.main().as_bytes()
            && self.ty.uint() != hacash_params::TX_TYPE_1
        {
            self.hash_with_fee()
        } else {
            self.hash()
        };
        let signobj = Sign::create_by(acc, &fhx);
        self.push_sign(signobj.clone())?;
        Ok(signobj)
    }

    fn hash_ex(&self, fee_bytes: Vec<u8>) -> Hash {
        let mode = if self.ty.uint() == hacash_params::TX_TYPE_3 {
            TxHashMode::Type3
        } else {
            TxHashMode::Legacy
        };
        tx_hash(
            mode,
            &self.ty,
            &self.timestamp,
            &self.addrlist,
            &fee_bytes,
            &self.actions,
            &self.gas_max,
            &self.ano_mark,
        )
    }

    /// Intrinsic R0: main ∪ static action req_sign, excluding RequiredSigners.
    /// Signer sets are small; `Vec` with a linear duplicate scan keeps the
    /// hash-table machinery out of the wasm graph.
    pub fn intrinsic_req_sign(&self) -> Ret<Vec<Address>> {
        let addrs = self.addrs();
        let mut adrsets = vec![self.main()];
        for act in &self.actions {
            if act.kind() == RequiredSigners::KIND {
                continue;
            }
            for ptr in act.req_sign() {
                let adr = ptr.real(&addrs)?;
                if adr.is_privkey() && !adrsets.contains(&adr) {
                    adrsets.push(adr);
                }
            }
        }
        Ok(adrsets)
    }

    /// Extra signers E from the unique top-level RequiredSigners (if any).
    pub fn declared_extra_signers(&self) -> Ret<Vec<Address>> {
        let addrs = self.addrs();
        let mut found: Option<&RequiredSigners> = None;
        for act in &self.actions {
            if let Some(list) = act.as_any().downcast_ref::<RequiredSigners>() {
                if found.is_some() {
                    return errf!("RequiredSigners must be TOP_GUARD_UNIQUE (duplicate found)");
                }
                found = Some(list);
            }
        }
        match found {
            None => Ok(Vec::new()),
            Some(list) => list.validate_against(&addrs),
        }
    }

    /// D = R0 union E with overlap checks.
    pub fn deterministic_signers(&self) -> Ret<Vec<Address>> {
        let mut d = self.intrinsic_req_sign()?;
        let e = self.declared_extra_signers()?;
        for adr in &e {
            if d.contains(adr) {
                return errf!(
                    "RequiredSigners address {} overlaps intrinsic req_sign",
                    adr.to_readable()
                );
            }
        }
        d.extend(e);
        Ok(d)
    }

    pub(crate) fn validate_signer_limit(&self, max: usize) -> Rerr {
        let count = self.deterministic_signers()?.len();
        if count > max {
            return errf!("Type3 signer count {} exceeds maximum {}", count, max);
        }
        Ok(())
    }

    pub fn deterministic_signers_vec(&self) -> Ret<Vec<Address>> {
        let mut v = self.deterministic_signers()?;
        sort_addresses(&mut v);
        Ok(v)
    }

    fn canonical_billing_size(&self) -> Ret<usize> {
        let d = self.deterministic_signers()?;
        let base_size = self
            .size()
            .checked_sub(self.signs.size())
            .ok_or_else(|| sys::Error::fault("Type3 billing size underflow"))?;
        let sign_item_size = Sign::default().size();
        if sign_item_size != Self::SIGN_ITEM_SIZE {
            return errf!(
                "Type3 Sign encoding size must be {}, got {}",
                Self::SIGN_ITEM_SIZE,
                sign_item_size
            );
        }
        let prefix_size = SignW2::default().size();
        let canonical_signs_size = prefix_size
            .checked_add(
                d.len()
                    .checked_mul(sign_item_size)
                    .ok_or_else(|| sys::Error::fault("Type3 canonical signs size overflow"))?,
            )
            .ok_or_else(|| sys::Error::fault("Type3 canonical signs size overflow"))?;
        base_size
            .checked_add(canonical_signs_size)
            .ok_or_else(|| sys::Error::fault("Type3 billing size overflow"))
    }

    /// Type3 fee purity in the chain pricing unit (`base::FEE_PRICING_UNIT` = u232):
    /// `fee / canonical SignW2 billing size`, saturated to `u64::MAX`. Mempool view.
    pub fn type3_fee_purity(&self) -> u64 {
        match self.type3_fee_purity_u128() {
            Ok(p) => p.min(u64::MAX as u128) as u64,
            Err(_) => 0,
        }
    }

    fn type3_fee_purity_u128(&self) -> Ret<u128> {
        let txsz = self.canonical_billing_size()?;
        if txsz == 0 {
            return Ok(0);
        }
        let fee = self.fee.to_unit_u128(base::FEE_PRICING_UNIT)?;
        Ok(fee / txsz as u128)
    }
}

/// Exact Type3 signature verification: SignW2 must match D exactly.
pub fn verify_type3_signatures_exact(tx: &StdTransaction) -> Rerr {
    let d = tx.deterministic_signers_vec()?;
    if tx.signs.length() != d.len() {
        return errf!(
            "Type3 SignW2 length {} != deterministic signer count {}",
            tx.signs.length(),
            d.len()
        );
    }
    let mut present_keys: Vec<[u8; Sign::PUBLICKEY_SIZE]> = Vec::new();
    for sig in tx.signs.as_list() {
        if sig.size() != StdTransaction::SIGN_ITEM_SIZE {
            return errf!(
                "Type3 Sign encoding size must be {}, got {}",
                StdTransaction::SIGN_ITEM_SIZE,
                sig.size()
            );
        }
        if present_keys.contains(sig.publickey.as_array()) {
            return errf!("Type3 SignW2 contains duplicate public key");
        }
        present_keys.push(sig.publickey.into_array());
    }
    let mut present_addrs: Vec<Address> = Vec::new();
    for sig in tx.signs.as_list() {
        let adr = sign_address(sig);
        if present_addrs.contains(&adr) {
            return errf!(
                "Type3 SignW2 contains duplicate signer address {}",
                adr.to_readable()
            );
        }
        present_addrs.push(adr);
        if !d.contains(&adr) {
            return errf!("undeclared Type3 signer {}", adr.to_readable());
        }
        let hx = sign_hash_for(tx, &adr);
        if !Account::verify_signature(&hx.0, &sig.publickey, &sig.signature) {
            return errf!("{:?} signature verification failed", adr);
        }
    }
    sort_addresses(&mut present_addrs);
    if present_addrs != d {
        return errf!("Type3 signer address set does not equal deterministic set D");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxSignatureReport {
    pub required: Vec<Address>,
    pub present: Vec<Address>,
    pub valid: Vec<Address>,
    pub missing: Vec<Address>,
    pub invalid: Vec<Address>,
}

fn sort_addresses(addrs: &mut Vec<Address>) {
    addrs.sort();
    addrs.dedup();
}

fn sign_address(sign: &Sign) -> Address {
    Address::from(Account::get_address_by_public_key(
        sign.publickey.into_array(),
    ))
}

fn signature_present_for(addr: &Address, signs: &[Sign]) -> bool {
    signs.iter().any(|sig| sign_address(sig) == *addr)
}

/// Canonical per-signer sign hash: the main signer of Type-2/3 signs
/// `hash_with_fee`, everyone else (and all Type-1 signers) signs `hash`.
pub fn sign_hash_for(tx: &dyn TransactionSign, adr: &Address) -> Hash {
    if *adr == tx.main() && tx.ty() != hacash_params::TX_TYPE_1 {
        tx.hash_with_fee()
    } else {
        tx.hash()
    }
}

pub fn verify_one_sign(hash: &Hash, addr: &Address, signs: &[Sign]) -> Ret<bool> {
    for sig in signs {
        if sign_address(sig) == *addr
            && Account::verify_signature(&hash.0, &sig.publickey, &sig.signature)
        {
            return Ok(true);
        }
    }
    errf!("{:?} signature verification failed", addr)
}

pub fn verify_target_signature(adr: &Address, tx: &dyn TransactionSign) -> Ret<bool> {
    let hx = sign_hash_for(tx, adr);
    verify_one_sign(&hx, adr, tx.signs())
}

pub fn verify_tx_signature(tx: &dyn TransactionSign) -> Rerr {
    if tx.ty() == hacash_params::TX_TYPE_3 {
        if let Some(t3) = tx.as_any().downcast_ref::<StdTransaction>() {
            return verify_type3_signatures_exact(t3);
        }
    }
    for adr in tx.req_sign()? {
        let hx = sign_hash_for(tx, &adr);
        verify_one_sign(&hx, &adr, tx.signs())?;
    }
    Ok(())
}

pub fn check_tx_signature(tx: &dyn TransactionSign) -> Ret<HashMap<Address, bool>> {
    let mut ckres = HashMap::new();
    for sig in tx.signs() {
        ckres.insert(sign_address(sig), true);
    }
    for adr in tx.req_sign()? {
        let hx = sign_hash_for(tx, &adr);
        let sigok = verify_one_sign(&hx, &adr, tx.signs()).unwrap_or(false);
        ckres.insert(adr, sigok);
    }
    Ok(ckres)
}

pub fn signature_report(tx: &dyn TransactionSign) -> Ret<TxSignatureReport> {
    let mut required = tx.req_sign()?;
    sort_addresses(&mut required);
    let mut present = tx.signs().iter().map(sign_address).collect::<Vec<_>>();
    sort_addresses(&mut present);

    let mut valid = Vec::new();
    let mut missing = Vec::new();
    let mut invalid = Vec::new();
    for adr in &required {
        let hx = sign_hash_for(tx, adr);
        match verify_one_sign(&hx, adr, tx.signs()) {
            Ok(true) => valid.push(*adr),
            Ok(false) => invalid.push(*adr),
            Err(_) if !signature_present_for(adr, tx.signs()) => missing.push(*adr),
            Err(_) => invalid.push(*adr),
        }
    }
    sort_addresses(&mut valid);
    sort_addresses(&mut missing);
    sort_addresses(&mut invalid);
    Ok(TxSignatureReport {
        required,
        present,
        valid,
        missing,
        invalid,
    })
}

fn clone_std_tx(tx: &dyn Transaction) -> Option<StdTransaction> {
    tx.as_any().downcast_ref::<StdTransaction>().cloned()
}

/// Re-encode with the signature set cleared (type-2/3 wire order preserved);
/// used for the stable `unsigned_body_hash`.
pub fn encode_without_signs(tx: &dyn Transaction) -> Ret<Vec<u8>> {
    let Some(mut copy) = clone_std_tx(tx) else {
        return errf!("transaction type {} has no unsigned-body form", tx.ty());
    };
    copy.signs = SignW2::default();
    Ok(copy.encode())
}

/// Clone and insert one signature without digest verification; same-key
/// replacement lives in `insert_sign`, body validity is a separate capability.
pub fn insert_attached_sign(tx: &dyn Transaction, sign: Sign) -> Ret<base::TxRef> {
    use base::TransactionBuild;
    let Some(mut copy) = clone_std_tx(tx) else {
        return errf!(
            "transaction type {} does not support insert_attached_sign",
            tx.ty()
        );
    };
    copy.insert_sign(sign)?;
    Ok(Arc::new(copy))
}

/// Clone, insert one signature via `push_sign` (insert + digest verify) and return
/// the signed tx; node/API paths use this, the SDK uses `insert_attached_sign`.
pub fn attach_sign(tx: &dyn Transaction, sign: Sign) -> Ret<base::TxRef> {
    use base::TransactionBuild;
    let Some(mut copy) = clone_std_tx(tx) else {
        return errf!("transaction type {} does not support attach_sign", tx.ty());
    };
    copy.push_sign(sign)?;
    Ok(Arc::new(copy))
}

/// Protocol signer-cap rule: only type 3 caps its *required* signer set (D) at
/// `max` (execute-time); evaluated on required, not attached, signers.
pub fn check_signers_cap(tx: &dyn Transaction, max: usize) -> Rerr {
    if tx.ty() == hacash_params::TX_TYPE_3 {
        let t3 = tx
            .as_any()
            .downcast_ref::<StdTransaction>()
            .ok_or_else(|| sys::Error::fault("Type3 signer cap cast failed"))?;
        t3.validate_signer_limit(max)?;
    }
    Ok(())
}

fn insert_sign(signs: &mut SignW2, signobj: Sign) -> Ret<Address> {
    if signs.length() >= u16::MAX as usize - 1 {
        return errf!("too many sign objects");
    }
    let curaddr = sign_address(&signobj);
    let istid = signs
        .as_list()
        .iter()
        .position(|sg| sg.publickey == signobj.publickey);
    if let Some(i) = istid {
        signs.as_mut()[i] = signobj;
    } else {
        signs.push(signobj)?;
    }
    Ok(curaddr)
}

fn decode_std_tx(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(StdTransaction, usize)> {
    let mut r = Reader::new(buf);
    let tx = StdTransaction {
        ty: r.read()?,
        timestamp: r.read()?,
        addrlist: r.read()?,
        fee: r.read()?,
        actions: r.read_with(|rest| decode_action_list(reg, rest))?,
        signs: r.read()?,
        gas_max: r.read()?,
        ano_mark: r.read()?,
    };
    Ok((tx, r.used()))
}

fn create_std_tx_of_type(
    reg: &dyn BinaryCodecs,
    buf: &[u8],
    expected: u8,
) -> Ret<(base::TxRef, usize)> {
    let (tx, used) = decode_std_tx(reg, buf)?;
    if tx.ty.uint() != expected {
        return sys::normalf!("transaction type codec got type {}", tx.ty.uint());
    }
    Ok((Arc::new(tx), used))
}

/// Wire creator for type-1: shared field decode plus the type-byte check.
pub fn create_transaction_type1(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    create_std_tx_of_type(reg, buf, hacash_params::TX_TYPE_1)
}

/// Wire creator for type-2: shared field decode plus the type-byte check.
pub fn create_transaction_type2(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    create_std_tx_of_type(reg, buf, hacash_params::TX_TYPE_2)
}

/// Wire creator for type-3: shared field decode plus the type-byte check.
pub fn create_transaction_type3(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    create_std_tx_of_type(reg, buf, hacash_params::TX_TYPE_3)
}

impl Encode for StdTransaction {
    fn size(&self) -> usize {
        self.ty.size()
            + self.timestamp.size()
            + self.addrlist.size()
            + self.fee.size()
            + action_list_size(&self.actions)
            + self.signs.size()
            + self.gas_max.size()
            + self.ano_mark.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.ty.encode_to(out);
        self.timestamp.encode_to(out);
        self.addrlist.encode_to(out);
        self.fee.encode_to(out);
        encode_action_list(&self.actions, out);
        self.signs.encode_to(out);
        self.gas_max.encode_to(out);
        self.ano_mark.encode_to(out);
    }
}

impl Transaction for StdTransaction {
    fn ty(&self) -> u8 {
        self.ty.uint()
    }

    fn main(&self) -> Address {
        self.addrlist.to_list()[0]
    }

    fn addrs(&self) -> Vec<Address> {
        self.addrlist.to_list()
    }

    fn fee(&self) -> &Amount {
        &self.fee
    }

    fn fee_got(&self) -> Amount {
        if self.ty.uint() == hacash_params::TX_TYPE_3 {
            return self.fee.clone();
        }
        let mut fee = self.fee.clone();
        if self.actions.iter().any(|action| action.extra9()) && fee.unit() > 1 {
            fee = fee.unit_sub(1).expect("fee unit is greater than one");
        }
        fee
    }

    fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }

    fn gas_max_byte(&self) -> Option<u8> {
        if self.ty.uint() == hacash_params::TX_TYPE_3 {
            Some(self.gas_max.uint())
        } else {
            None
        }
    }

    fn actions(&self) -> &[ActionRef] {
        &self.actions
    }

    fn signs(&self) -> &[Sign] {
        self.signs.as_list()
    }

    fn fee_purity(&self) -> u64 {
        if self.ty.uint() == hacash_params::TX_TYPE_3 {
            return self.type3_fee_purity();
        }
        // Non-type3: `fee_got` (extra9-discounted) over the encoded size, in the
        // chain pricing unit (u232), saturated to u64.
        self.fee_purity_in(base::FEE_PRICING_UNIT)
    }

    fn fee_purity_checked(&self) -> Ret<u64> {
        let purity = if self.ty.uint() == hacash_params::TX_TYPE_3 {
            self.type3_fee_purity_u128()?
        } else {
            let size = self.billing_size()?;
            if size == 0 {
                return Ok(0);
            }
            self.fee_got().to_unit_u128(base::FEE_PRICING_UNIT)? / size as u128
        };
        u64::try_from(purity).map_err(|_| {
            sys::Error::fault(format!(
                "tx fee purity {} overflows u64 (unit {})",
                purity,
                base::FEE_PRICING_UNIT
            ))
        })
    }

    fn billing_size(&self) -> Ret<usize> {
        if self.ty.uint() == hacash_params::TX_TYPE_3 {
            return self.canonical_billing_size();
        }
        Ok(Encode::size(self))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl TransactionSign for StdTransaction {
    fn hash(&self) -> Hash {
        self.hash_ex(Vec::new())
    }

    fn hash_with_fee(&self) -> Hash {
        self.hash_ex(self.fee.encode())
    }

    fn req_sign(&self) -> Ret<Vec<Address>> {
        if self.ty.uint() == hacash_params::TX_TYPE_3 {
            return self.deterministic_signers_vec();
        }
        req_sign_for(self.main(), &self.addrlist, &self.actions)
    }

    fn verify_signature(&self) -> Rerr {
        verify_tx_signature(self)
    }

    #[cfg(feature = "execute")]
    fn as_execute(&self) -> Option<&dyn TransactionExecute> {
        Some(self)
    }
}

impl TransactionBuild for StdTransaction {
    fn set_fee(&mut self, fee: Amount) {
        self.fee = fee;
    }

    fn insert_sign(&mut self, sg: Sign) -> Rerr {
        insert_sign(&mut self.signs, sg).map(|_| ())
    }

    fn push_sign(&mut self, sg: Sign) -> Rerr {
        let curaddr = insert_sign(&mut self.signs, sg)?;
        if verify_target_signature(&curaddr, self).unwrap_or(false) {
            return Ok(());
        }
        errf!("address {:?} signature verification failed", curaddr)
    }

    fn push_action(&mut self, act: ActionRef) -> Rerr {
        self.try_push_action(act)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::action::{TransferHacFromTo, TransferHacTo};
    use base::{ActionRef, TransactionBuild};
    use field::AddrOrPtr;
    use sys::ToHex;

    fn scriptmh_address() -> Address {
        // VERSION_SCRIPTMH = 5; such addresses cannot produce signatures.
        let mut raw = [0u8; 21];
        raw[0] = 5;
        raw[1..].copy_from_slice(&[7u8; 20]);
        Address::from(raw)
    }

    fn fromto_tx(from: Address, to: Address, acc: &Account) -> StdTransaction {
        let main = Address::from(*acc.address());
        let mut tx = StdTransaction::new(hacash_params::TX_TYPE_1, main, Amount::mei(1));
        tx.push_action_in(Arc::new(TransferHacFromTo {
            kind: Uint2::from(TransferHacFromTo::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            hacash: Amount::mei(1),
        }));
        tx
    }

    /// Legacy: FromTo txs with a SCRIPTMH req_sign target (cannot sign) must drop it
    /// from the required set, like dev's `req_sign`, or replay fails verification.
    #[test]
    fn legacy_req_sign_drops_non_privkey_targets() {
        let acc = Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let main = Address::from(*acc.address());
        let mut tx = fromto_tx(scriptmh_address(), main, &acc);
        tx.fill_sign_account(&acc).unwrap();

        let required = tx.req_sign().unwrap();
        assert_eq!(required, vec![main], "scriptmh signer must be dropped");
        tx.verify_signature().unwrap();
    }

    /// A PRIVAKEY `from` stays required: missing its signature fails, and the
    /// tx verifies once the second sign is attached.
    #[test]
    fn legacy_req_sign_keeps_privkey_targets() {
        let acc_from = Account::create_by_secret_key_value([1u8; 32]).unwrap();
        let acc_to = Account::create_by_secret_key_value([2u8; 32]).unwrap();
        let main = Address::from(*acc_from.address());
        let from = Address::from(*acc_to.address());

        let mut tx = fromto_tx(from, main, &acc_from);
        tx.fill_sign_account(&acc_from).unwrap();
        assert_eq!(tx.req_sign().unwrap(), vec![main, from]);
        assert!(
            tx.verify_signature().is_err(),
            "missing the `from` signature"
        );

        let sign = Sign::create_by(&acc_to, &tx.hash());
        tx.push_sign(sign).unwrap();
        tx.verify_signature().unwrap();
    }

    fn w2_actions(n: usize) -> Vec<ActionRef> {
        let act: ActionRef = Arc::new(TransferHacFromTo {
            kind: Uint2::from(TransferHacFromTo::KIND),
            from: AddrOrPtr::Addr(Address::default()),
            to: AddrOrPtr::Addr(Address::default()),
            hacash: Amount::mei(1),
        });
        vec![act; n]
    }

    #[test]
    fn action_list_w2_count_bounds() {
        let empty = w2_actions(0);
        let mut out = Vec::new();
        encode_action_list(&empty, &mut out);
        assert_eq!(out, vec![0, 0]);
        assert_eq!(action_list_size(&empty), out.len());

        let one = w2_actions(1);
        let mut out = Vec::new();
        encode_action_list(&one, &mut out);
        assert_eq!(action_list_size(&one), out.len());

        let mut tx =
            StdTransaction::new(hacash_params::TX_TYPE_2, Address::default(), Amount::mei(1));
        tx.push_action(w2_actions(1).pop().unwrap()).unwrap();
        assert_eq!(tx.size(), tx.encode().len());

        let max = w2_actions(u16::MAX as usize);
        let mut out = Vec::new();
        encode_action_list(&max, &mut out);
        assert_eq!(&out[..2], &[0xff, 0xff]);
        assert_eq!(action_list_size(&max), out.len());

        let mut tx =
            StdTransaction::new(hacash_params::TX_TYPE_2, Address::default(), Amount::mei(1));
        tx.actions = w2_actions(u16::MAX as usize);
        assert!(tx.push_action(w2_actions(1).pop().unwrap()).is_err());
    }

    /// Fee purity is priced in the chain pricing unit (u232): the mempool view
    /// saturates to `u64::MAX`, the consensus checked view errors past that
    /// (~1844 HAC/byte), and the nested-floor relation between the 232 and 238
    /// purity views holds (`p238 × 10⁶ ≤ p232 < (p238+1) × 10⁶`).
    #[test]
    fn fee_purity_priced_in_232_saturates_and_tracks_238() {
        let acc = Account::create_by_secret_key_value([3u8; 32]).unwrap();
        let main = Address::from(*acc.address());

        // Saturation: fee ≈ u128::MAX/2 over a small billing size exceeds u64.
        // Mempool still ranks it maximal; consensus protocol-fee pricing errors.
        let huge = Amount::coin_u128(u128::MAX / 2, base::FEE_PRICING_UNIT);
        let tx = StdTransaction::new(hacash_params::TX_TYPE_2, main, huge);
        assert_eq!(tx.fee_purity(), u64::MAX);
        assert_eq!(tx.fee_purity_in(base::FEE_PRICING_UNIT), u64::MAX);
        assert!(
            tx.fee_purity_checked().is_err(),
            "protocol-fee path must reject purity above u64::MAX, not undercharge"
        );

        // Moderate fee: exact u232 quotient plus the 238 nested-floor relation.
        let fee = Amount::coin_u128(123_456_789_000_555, base::FEE_PRICING_UNIT);
        let tx = StdTransaction::new(hacash_params::TX_TYPE_2, main, fee);
        let size = tx.billing_size().unwrap() as u128;
        let p232 = tx.fee_purity_in(field::UNIT_SHUO);
        let p238 = tx.fee_purity_in(field::UNIT_238);
        assert_eq!(p232, (123_456_789_000_555 / size) as u64);
        assert!(p232 < u64::MAX);
        assert_eq!(tx.fee_purity_checked().unwrap(), p232);
        assert!(
            (p238 as u128) * 1_000_000 <= p232 as u128
                && (p232 as u128) < (p238 as u128 + 1) * 1_000_000,
            "p232={p232} p238={p238}"
        );
    }

    /// Locked wire / hash / sign-hash vectors for types 1/2/3, captured before
    /// merging into StdTransaction. Must stay byte-identical.
    #[test]
    fn std_tx_golden_vectors() {
        let acc = Account::create_by("123456").unwrap();
        let main = Address::from(*acc.address());
        let fee = Amount::from("1:244").unwrap();
        let ts = 1_755_223_764u64;
        let act: ActionRef = Arc::new(TransferHacTo::new(main, Amount::from("1:244").unwrap()));

        let mut t1 = StdTransaction::new_by(hacash_params::TX_TYPE_1, main, fee.clone(), ts);
        t1.push_action_in(act.clone());
        assert_eq!(
            t1.encode().to_hex(),
            "0100689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010001000100e63c33a796b3032ce6b856f68fccf06608d9ed18f4010100000000"
        );
        assert_eq!(
            t1.hash().as_ref().to_hex(),
            "1bab63647f5ded3b68b735157dc4da2e06edc4034451292ee549bc10748016fa"
        );
        assert_eq!(
            t1.hash_with_fee().as_ref().to_hex(),
            "8d4cd2288ae0cc095a94e0f870efcd24677a2ff0e8352a39c2ea6eb03aeab465"
        );
        t1.fill_sign_account(&acc).unwrap();
        assert_eq!(
            t1.encode().to_hex(),
            "0100689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010001000100e63c33a796b3032ce6b856f68fccf06608d9ed18f4010100010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263cd82e71e372d1309493c12789730efee19c93be3dd0f8c21d83012dfd56b61bdb7fc6e526fa93acae092eb13387d332fe8b74ab3b7a9162c0f664e42d8d1308a70000"
        );
        assert_eq!(
            sign_hash_for(&t1, &main).as_ref().to_hex(),
            "1bab63647f5ded3b68b735157dc4da2e06edc4034451292ee549bc10748016fa"
        );

        let mut t2 = StdTransaction::new_by(hacash_params::TX_TYPE_2, main, fee.clone(), ts);
        t2.push_action_in(act.clone());
        assert_eq!(
            t2.encode().to_hex(),
            "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010001000100e63c33a796b3032ce6b856f68fccf06608d9ed18f4010100000000"
        );
        assert_eq!(
            t2.hash().as_ref().to_hex(),
            "b22e61f3e6ebef5e35f6b2885965109ae410f08abf6060173b8e30565f8ec181"
        );
        assert_eq!(
            t2.hash_with_fee().as_ref().to_hex(),
            "c82b6f0e2cd75f9e450cb6750d6f95b1eced0decb360c4e26d476c9d380e5496"
        );
        t2.fill_sign_account(&acc).unwrap();
        assert_eq!(
            t2.encode().to_hex(),
            "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010001000100e63c33a796b3032ce6b856f68fccf06608d9ed18f4010100010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263ce786aa6d1bf37bc185661a28a27835ca28f400d7af348cd60abd75f3b0f484ad62a0ce5d6b47262ac53bed6c010fbc59ab4aa1ad2689c2097b0cfe34a3b600b50000"
        );
        assert_eq!(
            sign_hash_for(&t2, &main).as_ref().to_hex(),
            "c82b6f0e2cd75f9e450cb6750d6f95b1eced0decb360c4e26d476c9d380e5496"
        );

        let mut t3 = StdTransaction::new_by(hacash_params::TX_TYPE_3, main, fee, ts);
        t3.gas_max = Uint1::from(8);
        t3.push_action_in(act);
        assert_eq!(
            t3.encode().to_hex(),
            "0300689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010001000100e63c33a796b3032ce6b856f68fccf06608d9ed18f4010100000800"
        );
        assert_eq!(
            t3.hash().as_ref().to_hex(),
            "b493df21802eb0541f51903fa4c1bc4f1f192f8d667e48659c4e4b682063975d"
        );
        assert_eq!(
            t3.hash_with_fee().as_ref().to_hex(),
            "d54ffea812748050fcbf4dcacd8ed0a1571a9b4080ed20c0d9f105252a8305dd"
        );
        t3.fill_sign_account(&acc).unwrap();
        assert_eq!(
            t3.encode().to_hex(),
            "0300689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010001000100e63c33a796b3032ce6b856f68fccf06608d9ed18f4010100010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c72bf4a192a8cfb7f03a7e2aae6d645fc3f1ac94878b9177b56b0e29346364c315c814fae040b4017762504b759418d1d456850b0f911ba6cea0670e8f01611ff0800"
        );
        assert_eq!(
            sign_hash_for(&t3, &main).as_ref().to_hex(),
            "d54ffea812748050fcbf4dcacd8ed0a1571a9b4080ed20c0d9f105252a8305dd"
        );
    }
}
