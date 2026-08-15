//! Registry

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use base::{
    ActionDispatcher, ActionRef, BinaryCodecs, Context, CoreState, ExecFrom, TX_ACTIONS_MAX,
    Transaction, TransactionBuild, TxCreateRequest, hac_add, hac_sub,
};
use field::{
    AddrOrList, Address, Amount, Encode, Fixed1, Fixed16, Hash, Reader, Sign, SignW2, Timestamp,
    Uint1, Uint2,
};
use sys::{Account, Rerr, Ret, errf};

use crate::codec::action::ReqSignList;
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

    fn hash(&self) -> Hash {
        Hash::from(sys::calculate_hash(self.encode()))
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

    fn verify_signature(&self) -> Rerr {
        errf!("cannot verify signature on prelude tx")
    }

    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        hac_add(ctx, &self.address, &self.reward)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl TransactionBuild for DefaultPreludeTx {
    fn set_mining_nonce(&mut self, nonce: Hash) {
        self.miner_nonce = nonce;
    }
}

pub fn create_default_prelude_tx(_reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    let mut r = Reader::new(buf);
    let ty: Uint1 = r.read()?;
    if ty.uint() != DefaultPreludeTx::TYPE {
        return sys::decodef!("prelude tx type must be 0, got {}", ty.uint());
    }
    let address: Address = r.read()?;
    let reward: Amount = r.read()?;
    let message: Fixed16 = r.read()?;
    let miner_nonce: Hash = r.read()?;
    Ok((
        Arc::new(DefaultPreludeTx {
            ty,
            address,
            reward,
            message,
            miner_nonce,
        }),
        r.used(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TxHashMode {
    Legacy,
    Type3,
}

#[derive(Debug, Clone)]
pub struct TransactionType1 {
    pub ty: Uint1,
    pub timestamp: Timestamp,
    pub addrlist: AddrOrList,
    pub fee: Amount,
    pub actions: Vec<ActionRef>,
    pub signs: SignW2,
    pub gas_max: Uint1,
    pub ano_mark: Fixed1,
}

#[derive(Debug, Clone)]
pub struct TransactionType2 {
    pub ty: Uint1,
    pub timestamp: Timestamp,
    pub addrlist: AddrOrList,
    pub fee: Amount,
    pub actions: Vec<ActionRef>,
    pub signs: SignW2,
    pub gas_max: Uint1,
    pub ano_mark: Fixed1,
}

#[derive(Debug, Clone)]
pub struct TransactionType3 {
    pub ty: Uint1,
    pub timestamp: Timestamp,
    pub addrlist: AddrOrList,
    pub fee: Amount,
    pub actions: Vec<ActionRef>,
    pub signs: SignW2,
    pub gas_max: Uint1,
    pub ano_mark: Fixed1,
}

pub type StdTransaction = TransactionType2;

/// Create an empty standard user transaction selected by its wire type.
///
/// This is the standard-protocol implementation of
/// [`base::TransactionCreator`]. It owns the concrete transaction types while
/// consumers depend only on base's request and creator interface.
pub fn create_standard_transaction(request: TxCreateRequest) -> Ret<base::TxRef> {
    let TxCreateRequest {
        ty,
        timestamp,
        addrlist,
        fee,
        gas_max,
    } = request;
    if ty != TransactionType3::TYPE && gas_max != 0 {
        return errf!("transaction type {} does not support gas", ty);
    }
    if ty == TransactionType1::TYPE {
        return Ok(Arc::new(TransactionType1 {
            ty: Uint1::from(ty),
            timestamp: Timestamp::from(timestamp),
            addrlist,
            fee,
            actions: Vec::new(),
            signs: SignW2::default(),
            gas_max: Uint1::from(gas_max),
            ano_mark: Fixed1::default(),
        }));
    }
    if ty == TransactionType2::TYPE {
        return Ok(Arc::new(TransactionType2 {
            ty: Uint1::from(ty),
            timestamp: Timestamp::from(timestamp),
            addrlist,
            fee,
            actions: Vec::new(),
            signs: SignW2::default(),
            gas_max: Uint1::from(gas_max),
            ano_mark: Fixed1::default(),
        }));
    }
    if ty == TransactionType3::TYPE {
        return Ok(Arc::new(TransactionType3 {
            ty: Uint1::from(ty),
            timestamp: Timestamp::from(timestamp),
            addrlist,
            fee,
            actions: Vec::new(),
            signs: SignW2::default(),
            gas_max: Uint1::from(gas_max),
            ano_mark: Fixed1::default(),
        }));
    }
    errf!("unsupported standard user transaction type {}", ty)
}

fn action_list_size(actions: &[ActionRef]) -> usize {
    Uint2::SIZE + actions.iter().map(|a| a.size()).sum::<usize>()
}

fn encode_action_list(actions: &[ActionRef], out: &mut Vec<u8>) {
    Uint2::from(actions.len() as u16).encode_to(out);
    for act in actions {
        act.encode_to(out);
    }
}

fn decode_action_list(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(Vec<ActionRef>, usize)> {
    let mut r = Reader::new(buf);
    let count: Uint2 = r.read()?;
    if count.uint() as usize > TX_ACTIONS_MAX {
        return sys::decodef!("tx actions count {} exceeds limit", count.uint());
    }
    let mut actions = Vec::with_capacity(count.uint() as usize);
    for _ in 0..count.uint() {
        let (act, used) = reg.decode_action(&buf[r.used()..])?;
        let _ = r.read_bytes(used)?;
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

fn precheck_tx(ctx: &dyn Context, tx: &dyn Transaction, actions: &[ActionRef]) -> Rerr {
    let params = crate::execution_params(ctx.services().as_ref())?;
    if let Some(tx) = tx.as_any().downcast_ref::<TransactionType3>() {
        tx.validate_signer_limit(params.max_type3_signers)?;
    }
    if ctx.env().chain.fast_sync {
        return Ok(());
    }
    if actions.is_empty() {
        return errf!("transaction actions cannot be empty");
    }
    if actions.len() > TX_ACTIONS_MAX {
        return errf!(
            "tx actions exceed limit {} > {}",
            actions.len(),
            TX_ACTIONS_MAX
        );
    }
    let need = tx.required_flags();
    if need & !ctx.env().chain.consensus_flags != 0 {
        return errf!("tx type {} not activated (flags need {:#x})", tx.ty(), need);
    }
    crate::level::precheck_tx_actions(
        tx.ty(),
        actions,
        ctx.env().chain.consensus_flags,
        params.ast_tree_depth_max,
    )
}

struct TxExecutePrep {
    block_height: u64,
    tx_hash: Hash,
    main: field::Address,
    fee: Amount,
    has_ast_control: bool,
}

fn prepare_tx_execute(tx: &dyn Transaction, ctx: &mut dyn Context) -> Ret<TxExecutePrep> {
    let env = ctx.env();
    let block_height = env.block.height;
    let tx_hash = tx.hash();
    let main = tx.main();
    let fee = tx.fee().clone();
    let has_ast_control = tx.actions().iter().any(|a| matches!(a.kind(), 25 | 26));
    if !env.chain.fast_sync {
        if !main.is_privkey() {
            return errf!("tx fee address version must be PRIVAKEY type");
        }
        if main.is_privkey_unknown() {
            return errf!(
                "tx main address {} is a system address with unknown private key",
                main.to_readable()
            );
        }
        for addr in tx.addrs() {
            if !addr.is_supported() {
                return errf!("address version {} not supported", addr.version());
            }
        }
        if block_height > 200_000 {
            if fee.size() > 6 {
                return errf!("tx fee size cannot exceed 6 bytes when block height above 200,000");
            }
        }
        if block_height > 33_033 && tx.ty() <= TransactionType1::TYPE {
            return errf!("Type 1 transactions have been deprecated after height 33,033");
        }
        tx.verify_signature()?;
        let existing = {
            let state = CoreState::wrap(ctx.layer());
            state.tx_exist(&tx_hash)
        };
        if let Some(existing) = existing? {
            // Preserve the historical dev exception for the one known duplicate
            // transaction replayed at height 63,448.
            const HISTORICAL_DUPLICATE_TX: [u8; Hash::SIZE] = [
                0xf2, 0x2d, 0xeb, 0x27, 0xdd, 0x28, 0x93, 0x39, 0x7c, 0x2b, 0xc2, 0x03, 0xdd, 0xc9,
                0xbc, 0x90, 0x34, 0xe4, 0x55, 0xfe, 0x63, 0x0d, 0x8e, 0xe3, 0x10, 0xe8, 0xb5, 0xec,
                0xc6, 0xdc, 0x56, 0x28,
            ];
            if existing.uint() != 63_448 || tx_hash != Hash::from(HISTORICAL_DUPLICATE_TX) {
                return errf!(
                    "tx {} already exists in height {}",
                    tx_hash,
                    existing.uint()
                );
            }
        }
    }
    Ok(TxExecutePrep {
        block_height,
        tx_hash,
        main,
        fee,
        has_ast_control,
    })
}

fn mark_tx_exist(ctx: &mut dyn Context, hash: &Hash, height: u64) {
    let mut state = CoreState::wrap(ctx.layer());
    state.tx_exist_set(hash, &field::BlockHeight::from(height));
}

fn record_tx_fee_totals(ctx: &mut dyn Context, tx: &dyn Transaction) -> Rerr {
    let fee_pay = tx.fee_pay().to_238_u64()? as u128;
    let fee_got = tx.fee_got().to_238_u64()? as u128;
    let mut state = CoreState::wrap(ctx.layer());
    base::with_base_total(&mut state, |total| {
        base::total_add_u12(
            &mut total.tx_fee_pay_total_238,
            fee_pay,
            "tx_fee_pay_total_238",
        )?;
        base::total_add_u12(
            &mut total.tx_fee_got_total_238,
            fee_got,
            "tx_fee_got_total_238",
        )
    })
}

fn record_legacy_extra9_burn(ctx: &mut dyn Context, fee: &Amount, fee_got: &Amount) -> Rerr {
    let burn = fee.sub_mode_u128(fee_got)?;
    if !burn.is_positive() {
        return Ok(());
    }
    let mut state = CoreState::wrap(ctx.layer());
    base::with_base_total(&mut state, |total| {
        base::total_add_amount_238(
            &mut total.tx_fee_burn90_238,
            &burn,
            "legacy_tx_extra9_burn_238",
        )
    })
}

fn execute_actions(ctx: &mut dyn Context, actions: &[ActionRef], charge_extra9: bool) -> Rerr {
    for act in actions {
        ctx.exec_from_set(ExecFrom::Top);
        if charge_extra9 {
            let _ = ActionDispatcher::dispatch_top(ctx, act)?;
        } else {
            let _ = ActionDispatcher::dispatch_top_without_extra9(ctx, act)?;
        }
    }
    Ok(())
}

fn req_sign_for(main: Address, addrlist: &AddrOrList, actions: &[ActionRef]) -> Ret<Vec<Address>> {
    let addrs = addrlist.to_list();
    let mut required = vec![main];
    for act in actions {
        for ptr in act.req_sign() {
            let addr = ptr.real(&addrs)?;
            // Legacy signer semantics: non-PRIVAKEY req_sign targets are not
            // required to sign. Mainnet history contains FromTo transfers
            // whose `from` is a SCRIPTMH address; those addresses cannot
            // produce a signature, so dev (transaction::macro req_sign) drops
            // them from the required set. Keep the same rule here or the
            // historical blocks fail signature verification on replay.
            if addr.is_privkey() && !required.contains(&addr) {
                required.push(addr);
            }
        }
    }
    Ok(required)
}

impl TransactionType3 {
    pub const SIGN_ITEM_SIZE: usize = 97;

    /// Intrinsic R0: main ∪ static action req_sign, excluding ReqSignList.
    pub fn intrinsic_req_sign(&self) -> Ret<HashSet<Address>> {
        let addrs = self.addrs();
        let mut adrsets = HashSet::from([self.main()]);
        for act in &self.actions {
            if act.kind() == ReqSignList::KIND {
                continue;
            }
            for ptr in act.req_sign() {
                let adr = ptr.real(&addrs)?;
                if adr.is_privkey() {
                    adrsets.insert(adr);
                }
            }
        }
        Ok(adrsets)
    }

    /// Extra signers E from the unique top-level ReqSignList (if any).
    pub fn declared_extra_signers(&self) -> Ret<HashSet<Address>> {
        let addrs = self.addrs();
        let mut found: Option<&ReqSignList> = None;
        for act in &self.actions {
            if let Some(list) = act.as_any().downcast_ref::<ReqSignList>() {
                if found.is_some() {
                    return errf!("ReqSignList must be TOP_GUARD_UNIQUE (duplicate found)");
                }
                found = Some(list);
            }
        }
        match found {
            None => Ok(HashSet::new()),
            Some(list) => list.validate_against(&addrs),
        }
    }

    /// D = R0 union E with overlap checks.
    pub fn deterministic_signers(&self) -> Ret<HashSet<Address>> {
        let r0 = self.intrinsic_req_sign()?;
        let e = self.declared_extra_signers()?;
        for adr in &e {
            if r0.contains(adr) {
                return errf!(
                    "ReqSignList address {} overlaps intrinsic req_sign",
                    adr.to_readable()
                );
            }
        }
        let mut d = r0;
        d.extend(e);
        Ok(d)
    }

    fn validate_signer_limit(&self, max: usize) -> Rerr {
        let count = self.deterministic_signers()?.len();
        if count > max {
            return errf!("Type3 signer count {} exceeds maximum {}", count, max);
        }
        Ok(())
    }

    pub fn deterministic_signers_vec(&self) -> Ret<Vec<Address>> {
        let mut v: Vec<_> = self.deterministic_signers()?.into_iter().collect();
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

    pub fn type3_fee_purity(&self) -> u64 {
        let Ok(txsz) = self.canonical_billing_size() else {
            return 0;
        };
        if txsz == 0 {
            return 0;
        }
        let fee238 = self.fee.to_238_u128().unwrap_or(u128::MAX);
        (fee238 / txsz as u128).min(u64::MAX as u128) as u64
    }
}

/// Exact Type3 signature verification: SignW2 must match D exactly.
pub fn verify_type3_signatures_exact(tx: &TransactionType3) -> Rerr {
    let d = tx.deterministic_signers()?;
    if tx.signs.length() != d.len() {
        return errf!(
            "Type3 SignW2 length {} != deterministic signer count {}",
            tx.signs.length(),
            d.len()
        );
    }
    let mut present = HashSet::new();
    for sig in tx.signs.as_list() {
        if sig.size() != TransactionType3::SIGN_ITEM_SIZE {
            return errf!(
                "Type3 Sign encoding size must be {}, got {}",
                TransactionType3::SIGN_ITEM_SIZE,
                sig.size()
            );
        }
        let adr = sign_address(sig);
        if !present.insert(sig.publickey) {
            return errf!("Type3 SignW2 contains duplicate public key");
        }
        // re-check address uniqueness via a second set
        let _ = adr;
    }
    let mut present_addrs = HashSet::new();
    for sig in tx.signs.as_list() {
        let adr = sign_address(sig);
        if !present_addrs.insert(adr) {
            return errf!(
                "Type3 SignW2 contains duplicate signer address {}",
                adr.to_readable()
            );
        }
        if !d.contains(&adr) {
            return errf!("undeclared Type3 signer {}", adr.to_readable());
        }
        let hx = sign_hash_for(tx, &adr);
        if !Account::verify_signature(&hx.0, &sig.publickey, &sig.signature) {
            return errf!("{:?} signature verification failed", adr);
        }
    }
    if present_addrs != d {
        return errf!("Type3 signer address set does not equal deterministic set D");
    }
    Ok(())
}

fn type3_req_sign(any: &dyn Any) -> Ret<Vec<Address>> {
    let t3 = any
        .downcast_ref::<TransactionType3>()
        .ok_or_else(|| sys::Error::fault("Type3 req_sign cast failed"))?;
    t3.deterministic_signers_vec()
}

fn type3_fee_purity(any: &dyn Any) -> u64 {
    any.downcast_ref::<TransactionType3>()
        .map(|t| t.type3_fee_purity())
        .unwrap_or(0)
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
    Address::from(Account::get_address_by_public_key(sign.publickey))
}

fn signature_present_for(addr: &Address, signs: &[Sign]) -> bool {
    signs.iter().any(|sig| sign_address(sig) == *addr)
}

fn sign_hash_for(tx: &dyn Transaction, adr: &Address) -> Hash {
    if *adr == tx.main() && tx.ty() != TransactionType1::TYPE {
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

pub fn verify_target_signature(adr: &Address, tx: &dyn Transaction) -> Ret<bool> {
    let hx = sign_hash_for(tx, adr);
    verify_one_sign(&hx, adr, tx.signs())
}

pub fn verify_tx_signature(tx: &dyn Transaction) -> Rerr {
    if tx.ty() == TransactionType3::TYPE {
        if let Some(t3) = tx.as_any().downcast_ref::<TransactionType3>() {
            return verify_type3_signatures_exact(t3);
        }
    }
    for adr in tx.req_sign()? {
        let hx = sign_hash_for(tx, &adr);
        verify_one_sign(&hx, &adr, tx.signs())?;
    }
    Ok(())
}

pub fn check_tx_signature(tx: &dyn Transaction) -> Ret<HashMap<Address, bool>> {
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

pub fn signature_report(tx: &dyn Transaction) -> Ret<TxSignatureReport> {
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

fn decode_tx_fields(
    reg: &dyn BinaryCodecs,
    buf: &[u8],
) -> Ret<(
    Uint1,
    Timestamp,
    AddrOrList,
    Amount,
    Vec<ActionRef>,
    SignW2,
    Uint1,
    Fixed1,
    usize,
)> {
    let mut r = Reader::new(buf);
    let ty: Uint1 = r.read()?;
    let timestamp: Timestamp = r.read()?;
    let addrlist: AddrOrList = r.read()?;
    let fee: Amount = r.read()?;
    let (actions, used) = decode_action_list(reg, &buf[r.used()..])?;
    let _ = r.read_bytes(used)?;
    let signs: SignW2 = r.read()?;
    let gas_max: Uint1 = r.read()?;
    let ano_mark: Fixed1 = r.read()?;
    Ok((
        ty,
        timestamp,
        addrlist,
        fee,
        actions,
        signs,
        gas_max,
        ano_mark,
        r.used(),
    ))
}

macro_rules! impl_tx_type {
    ($name:ident, $tyid:expr, $hash_mode:expr, $has_gas:expr) => {
        impl $name {
            pub const TYPE: u8 = $tyid;

            pub fn new(main: Address, fee: Amount) -> Self {
                Self::new_by(main, fee, 0)
            }

            pub fn new_by(main: Address, fee: Amount, ts: u64) -> Self {
                Self {
                    ty: Uint1::from(Self::TYPE),
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
                self.actions.push(act);
            }

            pub fn fill_sign_account(&mut self, acc: &Account) -> Ret<Sign> {
                let fhx = if acc.address() == self.main().as_bytes()
                    && Self::TYPE != TransactionType1::TYPE
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
                tx_hash(
                    $hash_mode,
                    &self.ty,
                    &self.timestamp,
                    &self.addrlist,
                    &fee_bytes,
                    &self.actions,
                    &self.gas_max,
                    &self.ano_mark,
                )
            }
        }

        impl Encode for $name {
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

        impl Transaction for $name {
            fn ty(&self) -> u8 {
                self.ty.uint()
            }

            fn hash(&self) -> Hash {
                self.hash_ex(Vec::new())
            }

            fn hash_with_fee(&self) -> Hash {
                self.hash_ex(self.fee.encode())
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
                if Self::TYPE == TransactionType3::TYPE {
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
                if $has_gas {
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
                if Self::TYPE == TransactionType3::TYPE {
                    return type3_fee_purity(self as &dyn Any);
                }
                let txsz = Encode::size(self) as u64;
                if txsz == 0 {
                    return 0;
                }
                let fee238 = self.fee_got().to_238_u128().unwrap_or(u128::MAX);
                let purity = fee238 / txsz as u128;
                purity.min(u64::MAX as u128) as u64
            }

            fn billing_size(&self) -> Ret<usize> {
                if Self::TYPE == TransactionType3::TYPE {
                    if let Some(t3) = (self as &dyn Any).downcast_ref::<TransactionType3>() {
                        return t3.canonical_billing_size();
                    }
                }
                Ok(Encode::size(self))
            }

            fn req_sign(&self) -> Ret<Vec<Address>> {
                if Self::TYPE == TransactionType3::TYPE {
                    return type3_req_sign(self as &dyn Any);
                }
                req_sign_for(self.main(), &self.addrlist, &self.actions)
            }

            fn verify_signature(&self) -> Rerr {
                verify_tx_signature(self)
            }

            fn execute(&self, ctx: &mut dyn Context) -> Rerr {
                precheck_tx(ctx, self, &self.actions)?;
                let prep = prepare_tx_execute(self, ctx)?;
                if !ctx.env().chain.fast_sync
                    && Self::TYPE != TransactionType3::TYPE
                    && prep.has_ast_control
                {
                    return errf!(
                        "tx type {} cannot include AST control-flow actions; requires at least type 3",
                        Self::TYPE
                    );
                }
                if !ctx.env().chain.fast_sync && self.ano_mark[0] != 0 {
                    return errf!("tx type {} ano_mark must be zero", Self::TYPE);
                }
                if !ctx.env().chain.fast_sync
                    && Self::TYPE != TransactionType3::TYPE
                    && self.gas_max.uint() != 0
                {
                    return errf!("tx type {} gas_max must be zero", Self::TYPE);
                }

                mark_tx_exist(ctx, &prep.tx_hash, prep.block_height);
                record_tx_fee_totals(ctx, self)?;
                if Self::TYPE == TransactionType3::TYPE {
                    let gas_initialized = crate::exec::gas::tx_gas_initialize(ctx)?;
                    execute_actions(ctx, &self.actions, true)?;
                    crate::exec::tex::do_settlement(ctx)?;
                    ctx.run_deferred_phase()?;
                    if gas_initialized {
                        ctx.gas_refund()?;
                    }
                } else {
                    execute_actions(ctx, &self.actions, false)?;
                    crate::exec::tex::do_settlement(ctx)?;
                }
                hac_sub(ctx, &prep.main, &prep.fee)?;
                if Self::TYPE != TransactionType3::TYPE {
                    record_legacy_extra9_burn(ctx, &prep.fee, &self.fee_got())?;
                }
                crate::exec::tex::settlement_addr_postsettle_cleanup(ctx)?;
                Ok(())
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl TransactionBuild for $name {
            fn set_fee(&mut self, fee: Amount) {
                self.fee = fee;
            }

            fn push_sign(&mut self, sg: Sign) -> Rerr {
                let curaddr = insert_sign(&mut self.signs, sg)?;
                if verify_target_signature(&curaddr, self).unwrap_or(false) {
                    return Ok(());
                }
                errf!("address {:?} signature verification failed", curaddr)
            }

            fn push_action(&mut self, act: ActionRef) -> Rerr {
                if self.actions.len() >= TX_ACTIONS_MAX {
                    return errf!("tx actions exceed limit {}", TX_ACTIONS_MAX);
                }
                self.actions.push(act);
                Ok(())
            }
        }
    };
}

impl_tx_type!(TransactionType1, 1, TxHashMode::Legacy, false);
impl_tx_type!(TransactionType2, 2, TxHashMode::Legacy, false);
impl_tx_type!(TransactionType3, 3, TxHashMode::Type3, true);

pub fn create_transaction_type1(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    let (ty, timestamp, addrlist, fee, actions, signs, gas_max, ano_mark, used) =
        decode_tx_fields(reg, buf)?;
    if ty.uint() != TransactionType1::TYPE {
        return sys::decodef!("transaction type1 codec got type {}", ty.uint());
    }
    Ok((
        Arc::new(TransactionType1 {
            ty,
            timestamp,
            addrlist,
            fee,
            actions,
            signs,
            gas_max,
            ano_mark,
        }),
        used,
    ))
}

pub fn create_transaction_type2(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    let (ty, timestamp, addrlist, fee, actions, signs, gas_max, ano_mark, used) =
        decode_tx_fields(reg, buf)?;
    if ty.uint() != TransactionType2::TYPE {
        return sys::decodef!("transaction type2 codec got type {}", ty.uint());
    }
    Ok((
        Arc::new(TransactionType2 {
            ty,
            timestamp,
            addrlist,
            fee,
            actions,
            signs,
            gas_max,
            ano_mark,
        }),
        used,
    ))
}

pub fn create_transaction_type3(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    let (ty, timestamp, addrlist, fee, actions, signs, gas_max, ano_mark, used) =
        decode_tx_fields(reg, buf)?;
    if ty.uint() != TransactionType3::TYPE {
        return sys::decodef!("transaction type3 codec got type {}", ty.uint());
    }
    Ok((
        Arc::new(TransactionType3 {
            ty,
            timestamp,
            addrlist,
            fee,
            actions,
            signs,
            gas_max,
            ano_mark,
        }),
        used,
    ))
}

pub fn create_std_tx(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    create_transaction_type2(reg, buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::action::HacFromToTrs;
    use field::AddrOrPtr;

    fn scriptmh_address() -> Address {
        // VERSION_SCRIPTMH = 5; such addresses cannot produce signatures.
        let mut raw = [0u8; 21];
        raw[0] = 5;
        raw[1..].copy_from_slice(&[7u8; 20]);
        Address::from(raw)
    }

    fn fromto_tx(from: Address, to: Address, acc: &Account) -> TransactionType1 {
        let main = Address::from(*acc.address());
        let mut tx = TransactionType1::new(main, Amount::mei(1));
        tx.push_action_in(Arc::new(HacFromToTrs {
            kind: Uint2::from(HacFromToTrs::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            hacash: Amount::mei(1),
        }));
        tx
    }

    /// Mainnet history contains FromTo txs whose action req_sign target is a
    /// SCRIPTMH address (which cannot sign). The legacy required-signer set
    /// must drop it, exactly like dev's `req_sign` — otherwise the historical
    /// block fails `verify_signature` on replay.
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
}
