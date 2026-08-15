//! `ContractDeploy` (kind 40) + `ContractUpdate` (kind 41) top-level actions.
//!
//! Ported from fullnodedev `vm/src/action/contract.rs`. Adaptations:
//! - VM execution params via `Registry.vm_params()` (incl. `effective_fee_purity`)
//! - ledger ops via `base::{CoreState, operate::*}` (no `protocol` dependency)
//! - `ctx.state()` -> `ctx.layer()`
//! - `vmsto!(ctx).contract_exist(addr)` -> `vmsto.contract(&addr).is_some()`
//! - `ctx.vm_invalidate_contract_cache(addr)` -> `ctx.vm_peek()` + `Vm::invalidate_contract_cache`
//! - `peek_vm_runtime_limits(ctx, hei)` for live warm GasExtra/SpaceCap when VM is slotted
//! - `run_abst_entry`/`run_main_entry` -> `ctx.vm_call(VmEntry::Raw(Box::new(VmRequest::Abst{...}/Main{...})))`
//! - transfer-notification hook mechanism dropped (handled natively by ActionDispatcher::dispatch tail)

use std::any::Any;
use std::sync::Arc;

use base::{
    ActScope, ActionRef, Context, CoreState, VmEntry, hac_sub, total_add_amount_238, total_add_u8,
    total_add_u12, with_base_total,
};
use field::{Address, Amount, BytesW2, Decode, Encode, Fixed2, Fixed4, Uint2, Uint4};
use sys::{Rerr, Ret, errf};

use crate::contract::{ContractEdit, ContractSto};
use crate::machine::{VmRequest, peek_vm_runtime_limits};
use crate::rt::{
    AbstCall, CallSpec, CodePkg, CodeType, GasExtra, decode_user_call_site, is_user_call_inst,
};
use crate::state::VMState;
use crate::value::{ContractAddress, Value};

macro_rules! vmsto {
    ($ctx: expr) => {
        VMState::wrap($ctx.layer())
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractStoreAnalysis {
    pub address: ContractAddress,
    pub contract_size: usize,
    pub inherit_count: usize,
    pub library_count: usize,
    pub has_construct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractUpdateAnalysis {
    pub address: ContractAddress,
    pub old_contract_size: usize,
    pub new_contract_size: usize,
    pub edit_size: usize,
    pub did_structural_change: bool,
    pub did_effective_lookup_change: bool,
    pub update_hook: AbstCall,
    pub required_protocol_cost: Amount,
}

// ================================ ContractDeploy ================================

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
pub struct ContractDeploy {
    pub kind: Uint2,
    pub protocol_cost: Amount,
    pub nonce: Uint4,
    pub construct_argv: BytesW2, // checked by SpaceCap::value_size at runtime
    pub marks: Fixed4,           // zero
    pub contract: ContractSto,
}

impl ContractDeploy {
    pub const KIND: u16 = 40;

    pub fn new() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            protocol_cost: Amount::zero(),
            nonce: Uint4::from(0),
            construct_argv: BytesW2::default(),
            marks: Fixed4::default(),
            contract: ContractSto::default(),
        }
    }
}

impl Default for ContractDeploy {
    fn default() -> Self {
        Self::new()
    }
}

base::impl_action! {
    ContractDeploy {
        name: "contract_deploy",
        scope: ActScope::TOP_ONLY_CAN_WITH_GUARD,
        min_tx_type: 3,
        extra9: |_: &ContractDeploy| false,
        req_sign: |_: &ContractDeploy| vec![],
        as_transfer_like: none,
        description: |this: &ContractDeploy| {
            format!("Deploy smart contract with nonce {}", this.nonce.uint())
        },
        execute: (self, ctx) {
        contract_deploy_execute(self, ctx)?;
        Ok(vec![])
        }
    }
}

fn contract_deploy_execute(this: &ContractDeploy, ctx: &mut dyn Context) -> Rerr {
    let fast_sync = ctx.env().chain.fast_sync;
    if !fast_sync && !this.marks.is_zero() {
        // reserved marks must stay zero
        return errf!("marks bytes invalid");
    }
    let hei = ctx.env().block.height;
    let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
    let maddr = ctx.env().tx.main;
    // check contract
    let caddr = ContractAddress::calculate(&maddr, &this.nonce);
    if !fast_sync && vmsto!(ctx).contract(&caddr)?.is_some() {
        return errf!("contract {} already exists", caddr.to_readable());
    }
    // check
    if !fast_sync {
        this.contract
            .check(hei, &cap, &gst, ctx.services().as_ref())
            .map_err(sys::Error::from)?;
        if this.contract.metas.revision.uint() != 0 {
            return errf!("contract revision must be 0 on deploy");
        }
    }
    let has_construct = precheck_contract_store(&caddr, &this.contract, &gst, ctx)?;
    let cargv = this.construct_argv.to_vec();
    if !fast_sync && cargv.len() > cap.value_size {
        return errf!("construct argv size overflow");
    }
    if !fast_sync && !has_construct && !cargv.is_empty() {
        return errf!("construct argv provided but Construct hook not found");
    }
    if !fast_sync && this.contract.size() == 0 {
        return errf!("contract content cannot be empty");
    }
    let charge_bytes = this.contract.size();
    // spend protocol fee
    let periods = ctx.services().vm_params()?.contract_store_perm_periods;
    if !fast_sync {
        check_sub_contract_protocol_cost(ctx, &this.protocol_cost, charge_bytes, periods)?;
    }
    if this.protocol_cost.is_positive() {
        let mut state = CoreState::wrap(ctx.layer());
        with_base_total(&mut state, |ttcount| {
            total_add_amount_238(
                &mut ttcount.contract_protocol_cost_burn_238,
                &this.protocol_cost,
                "contract_protocol_cost_burn_238",
            )?;
            total_add_u8(
                &mut ttcount.contract_deploy_count,
                1,
                "contract_deploy_count",
            )?;
            total_add_u12(
                &mut ttcount.contract_charge_bytes_total,
                charge_bytes as u128,
                "contract_charge_bytes_total",
            )?;
            Ok(())
        })?;
    } else {
        let mut state = CoreState::wrap(ctx.layer());
        with_base_total(&mut state, |ttcount| {
            total_add_u8(
                &mut ttcount.contract_deploy_count,
                1,
                "contract_deploy_count",
            )
        })?;
    }
    // save the contract first; tx-level rollback owns final unwind if Construct fails.
    vmsto!(ctx).contract_set_sync_edition(&caddr, &this.contract);
    if has_construct {
        let _ = run_abst_entry(ctx, AbstCall::Construct, caddr, Value::Bytes(cargv))?;
    }
    // ok finish
    Ok(())
}

// ================================ ContractUpdate ================================

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
pub struct ContractUpdate {
    pub kind: Uint2,
    pub protocol_cost: Amount,
    pub address: Address, // contract address
    pub marks: Fixed2,    // zero
    pub edit: ContractEdit,
}

impl ContractUpdate {
    pub const KIND: u16 = 41;

    pub fn new() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            protocol_cost: Amount::zero(),
            address: Address::default(),
            marks: Fixed2::default(),
            edit: ContractEdit::default(),
        }
    }
}

impl Default for ContractUpdate {
    fn default() -> Self {
        Self::new()
    }
}

base::impl_action! {
    ContractUpdate {
        name: "contract_update",
        scope: ActScope::TOP_ONLY_CAN_WITH_GUARD,
        min_tx_type: 3,
        extra9: |_: &ContractUpdate| false,
        req_sign: |_: &ContractUpdate| vec![],
        as_transfer_like: none,
        description: |this: &ContractUpdate| {
            format!("Update smart contract {}", this.address.to_readable())
        },
        execute: (self, ctx) {
        contract_update_execute(self, ctx)?;
        Ok(vec![])
        }
    }
}

fn contract_update_execute(this: &ContractUpdate, ctx: &mut dyn Context) -> Rerr {
    use AbstCall::*;
    let fast_sync = ctx.env().chain.fast_sync;
    if !fast_sync && !this.marks.is_zero() {
        return errf!("marks bytes invalid");
    }
    let hei = ctx.env().block.height;
    let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
    // load old
    let caddr = ContractAddress::from_addr(this.address)?;
    let Some(contract) = vmsto!(ctx).contract(&caddr)? else {
        return errf!("contract {} does not exist", caddr.to_readable());
    };
    // apply edit (in memory)
    let mut new_contract = contract.clone();
    let did_structural_change = new_contract
        .apply_edit(&this.edit, hei, &cap, &gst, ctx.services().as_ref())
        .map_err(sys::Error::from)?;
    let _ = precheck_contract_store(&caddr, &new_contract, &gst, ctx)?;
    if new_contract.size() == 0 {
        return errf!("contract content cannot be empty");
    }
    let did_effective_lookup_change =
        effective_userfn_lookup_changed(&mut vmsto!(ctx), &caddr, &contract, &new_contract)?;
    // Final dispatch is driven by whether any existing visible selector semantics changed.
    // Purely additive edits (e.g. inherit/library append, or new local funcs with no shadowing)
    // stay Append; structural replacements or selector-owner changes are Change.
    let is_change = did_structural_change || did_effective_lookup_change;
    // Modification tax: charge the edit payload at perm periods (edit.size() >= chain delta).
    let edit_bytes = this.edit.size();
    let edit_periods = ctx.services().vm_params()?.contract_store_perm_periods;
    let total_fee = (!fast_sync)
        .then(|| calc_contract_protocol_cost_min_with_periods(ctx, edit_bytes, edit_periods))
        .transpose()?;
    let pcost = &this.protocol_cost;
    if !fast_sync && pcost.is_negative() {
        return errf!("protocol fee cannot be negative");
    }
    if let Some(total_fee) = total_fee
        && *pcost < total_fee
    {
        return errf!(
            "protocol fee must be at least {} (edit_bytes={}, edit_periods={}) but got {}",
            &total_fee,
            edit_bytes,
            edit_periods,
            &this.protocol_cost
        );
    }
    if !pcost.is_zero() {
        let maddr = ctx.env().tx.main;
        hac_sub(ctx, &maddr, pcost)?;
    }
    {
        let mut state = CoreState::wrap(ctx.layer());
        with_base_total(&mut state, |ttcount| {
            total_add_u8(
                &mut ttcount.contract_update_count,
                1,
                "contract_update_count",
            )?;
            total_add_u12(
                &mut ttcount.contract_charge_bytes_total,
                edit_bytes as u128,
                "contract_charge_bytes_total",
            )?;
            if pcost.is_positive() {
                total_add_amount_238(
                    &mut ttcount.contract_protocol_cost_burn_238,
                    pcost,
                    "contract_protocol_cost_burn_238",
                )?;
            }
            Ok(())
        })?;
    }
    let sys_hook = if is_change { Change } else { Append }; // Change or Append
    // Authorization is intentionally delegated to the current contract's Change/Append hook.
    // Run the selected hook on the current on-chain contract; failure means the update is not allowed.
    let _ = run_abst_entry(ctx, sys_hook, caddr, Value::Nil)?;
    // save the new
    vmsto!(ctx).contract_set_sync_edition(&caddr, &new_contract);
    let caddr_real = caddr.to_addr();
    if let Some(vm) = ctx.vm_peek() {
        vm.invalidate_contract_cache(&caddr_real);
    }
    Ok(())
}

/**************************************/

fn check_contract_self_reference(root_addr: &ContractAddress, root_contract: &ContractSto) -> Rerr {
    macro_rules! any_same {
        ($key: ident) => {
            root_contract.$key.as_list().iter().any(|a| a == root_addr)
        };
    }
    if any_same!(inherit) {
        return errf!("contract cannot inherit itself {}", root_addr.to_readable());
    }
    if any_same!(library) {
        return errf!(
            "contract cannot link itself as library {}",
            root_addr.to_readable()
        );
    }
    Ok(())
}

fn precheck_contract_store(
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    gst: &GasExtra,
    ctx: &mut dyn Context,
) -> Ret<bool> {
    Ok(analyze_contract_store(ctx, root_addr, root_contract, gst)?.has_construct)
}

pub fn analyze_contract_store(
    ctx: &mut dyn Context,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    gst: &GasExtra,
) -> Ret<ContractStoreAnalysis> {
    check_contract_self_reference(root_addr, root_contract)?;
    let mut vmsta = VMState::wrap(ctx.layer());
    check_link_contracts_exist(&mut vmsta, root_addr, root_contract)?;
    check_inherits_direct_parents_flat(&mut vmsta, root_addr, root_contract)?;
    let has_construct =
        detect_effective_abst_presence(&mut vmsta, root_addr, root_contract, AbstCall::Construct)?;
    check_static_call_targets(&mut vmsta, root_addr, root_contract, gst)?;
    Ok(ContractStoreAnalysis {
        address: *root_addr,
        contract_size: root_contract.size(),
        inherit_count: root_contract.inherit.length(),
        library_count: root_contract.library.length(),
        has_construct,
    })
}

pub fn analyze_contract_update(
    ctx: &mut dyn Context,
    address: &ContractAddress,
    edit: &ContractEdit,
) -> Ret<ContractUpdateAnalysis> {
    use AbstCall::*;
    let hei = ctx.env().block.height;
    let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
    let Some(contract) = VMState::wrap(ctx.layer()).contract(address)? else {
        return errf!("contract {} does not exist", address.to_readable());
    };
    let mut new_contract = contract.clone();
    let did_structural_change = new_contract
        .apply_edit(edit, hei, &cap, &gst, ctx.services().as_ref())
        .map_err(sys::Error::from)?;
    let _ = analyze_contract_store(ctx, address, &new_contract, &gst)?;
    if new_contract.size() == 0 {
        return errf!("contract content cannot be empty");
    }
    let did_effective_lookup_change = effective_userfn_lookup_changed(
        &mut VMState::wrap(ctx.layer()),
        address,
        &contract,
        &new_contract,
    )?;
    let is_change = did_structural_change || did_effective_lookup_change;
    let edit_size = edit.size();
    Ok(ContractUpdateAnalysis {
        address: *address,
        old_contract_size: contract.size(),
        new_contract_size: new_contract.size(),
        edit_size,
        did_structural_change,
        did_effective_lookup_change,
        update_hook: if is_change { Change } else { Append },
        required_protocol_cost: calc_contract_protocol_cost_min_with_periods(
            ctx,
            edit_size,
            ctx.services().vm_params()?.contract_store_perm_periods,
        )?,
    })
}

fn load_contract_for_check(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    addr: &ContractAddress,
    role: &str,
) -> Ret<ContractSto> {
    if addr == root_addr {
        return Ok(root_contract.clone());
    }
    match vmsta.contract(addr)? {
        Some(c) => Ok(c),
        None => errf!("{} contract {} does not exist", role, addr.to_readable()),
    }
}

fn detect_effective_abst_presence(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    abst: AbstCall,
) -> Ret<bool> {
    if root_contract.have_abst_call(abst) {
        return Ok(true);
    }
    for parent in root_contract.inherit.as_list() {
        let sto = load_contract_for_check(vmsta, root_addr, root_contract, parent, "inherit")?;
        if sto.have_abst_call(abst) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn check_link_contracts_exist(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
) -> Rerr {
    for a in root_contract.library.as_list() {
        let _ = load_contract_for_check(vmsta, root_addr, root_contract, a, "library")?;
    }
    for a in root_contract.inherit.as_list() {
        let _ = load_contract_for_check(vmsta, root_addr, root_contract, a, "inherit")?;
    }
    Ok(())
}

fn check_inherits_direct_parents_flat(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
) -> Rerr {
    for p in root_contract.inherit.as_list() {
        let sto = load_contract_for_check(vmsta, root_addr, root_contract, p, "inherit")?;
        if sto.inherit.length() > 0 {
            return errf!(
                "inherit parent {} cannot have parent inherit",
                p.to_readable()
            );
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct UserfnMeta {
    is_external: bool,
}

fn contract_userfn_meta(contract: &ContractSto, sign: &crate::rt::FnSign) -> Option<UserfnMeta> {
    let f = contract
        .userfuncs
        .as_list()
        .iter()
        .find(|f| f.sign.into_array() == *sign)?;
    let ext_mark = crate::rt::FnConf::External as u8;
    Some(UserfnMeta {
        is_external: f.fncnf[0] & ext_mark == ext_mark,
    })
}

fn collect_effective_userfn_owners(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
) -> Ret<std::collections::HashMap<crate::rt::FnSign, ContractAddress>> {
    let mut owners = std::collections::HashMap::new();
    for f in root_contract.userfuncs.as_list() {
        owners.entry(f.sign.into_array()).or_insert(*root_addr);
    }
    for parent in root_contract.inherit.as_list() {
        let sto = load_contract_for_check(vmsta, root_addr, root_contract, parent, "inherit")?;
        for f in sto.userfuncs.as_list() {
            owners.entry(f.sign.into_array()).or_insert(*parent);
        }
    }
    Ok(owners)
}

fn effective_userfn_lookup_changed(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    old_contract: &ContractSto,
    new_contract: &ContractSto,
) -> Ret<bool> {
    let old_table = collect_effective_userfn_owners(vmsta, root_addr, old_contract)?;
    let new_table = collect_effective_userfn_owners(vmsta, root_addr, new_contract)?;
    for (sign, old_owner) in old_table {
        match new_table.get(&sign) {
            Some(new_owner) if new_owner == &old_owner => {}
            _ => return Ok(true),
        }
    }
    Ok(false)
}

fn scan_call_sites(
    codes: &[u8],
    mut check: impl FnMut(crate::rt::Bytecode, &[u8]) -> Rerr,
) -> Rerr {
    let mut i = 0usize;
    while i < codes.len() {
        let inst = crate::rt::Bytecode::try_from_u8(codes[i]).map_err(sys::Error::from)?;
        let meta = inst.metadata();
        if !meta.valid {
            return errf!("invalid bytecode {}", codes[i]);
        }
        i += 1;
        let pms = meta.param as usize;
        if i + pms > codes.len() {
            return errf!("instruction param overflow at {}", i - 1);
        }
        let params = &codes[i..i + pms];
        match inst {
            _ if is_user_call_inst(inst) => {
                check(inst, params)?;
            }
            crate::rt::Bytecode::PBUF => {
                let l = params[0] as usize;
                if i + pms + l > codes.len() {
                    return errf!("PBUF overflow at {}", i - 1);
                }
                i += l;
            }
            crate::rt::Bytecode::PBUFL => {
                let l = u16::from_be_bytes([params[0], params[1]]) as usize;
                if i + pms + l > codes.len() {
                    return errf!("PBUFL overflow at {}", i - 1);
                }
                i += l;
            }
            _ => {}
        }
        i += pms;
    }
    Ok(())
}

fn resolve_userfn_meta_on_owner(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    owner: &ContractAddress,
    sign: &crate::rt::FnSign,
) -> Ret<Option<(ContractAddress, UserfnMeta)>> {
    let sto = load_contract_for_check(vmsta, root_addr, root_contract, owner, "lookup")?;
    Ok(contract_userfn_meta(&sto, sign).map(|meta| (*owner, meta)))
}

fn resolve_lookup_anchor_for_check(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    func_tag: &str,
    call: &CallSpec,
) -> Ret<ContractAddress> {
    let lib_addrs: Vec<Address> = root_contract
        .library
        .as_list()
        .iter()
        .map(|a| a.to_addr())
        .collect();
    // Static precheck binds `this` to the contract being stored, so `this.*` must not be purely
    // virtual: a default implementation must already exist on self or an inherited parent.
    let anchor = call
        .resolve_anchor_from(Some(root_addr), Some(root_addr), &lib_addrs)
        .map_err(|e| sys::Error::fault(format!("{}: {}", func_tag, e)))?;
    if call.lib_index().is_some() {
        let _ = load_contract_for_check(vmsta, root_addr, root_contract, &anchor, "lookup")?;
    }
    Ok(anchor)
}

fn resolve_lookup_entries_for_check(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    anchor: &ContractAddress,
    call: &CallSpec,
) -> Ret<Vec<ContractAddress>> {
    let parents = if call.needs_inherit_chain() {
        load_contract_for_check(vmsta, root_addr, root_contract, anchor, "inherit")?
            .inherit
            .as_list()
            .to_vec()
    } else {
        vec![]
    };
    Ok(call.resolve_candidates(anchor, &parents))
}

fn resolve_userfn_meta_by_lookup_for_check(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    func_tag: &str,
    call: &CallSpec,
    sign: &crate::rt::FnSign,
) -> Ret<Option<(ContractAddress, UserfnMeta)>> {
    let anchor = resolve_lookup_anchor_for_check(vmsta, root_addr, root_contract, func_tag, call)?;
    let entries = resolve_lookup_entries_for_check(vmsta, root_addr, root_contract, &anchor, call)?;
    for owner in entries {
        if let Some(hit) =
            resolve_userfn_meta_on_owner(vmsta, root_addr, root_contract, &owner, sign)?
        {
            return Ok(Some(hit));
        }
    }
    Ok(None)
}

fn check_static_call_targets(
    vmsta: &mut VMState,
    root_addr: &ContractAddress,
    root_contract: &ContractSto,
    gst: &GasExtra,
) -> Rerr {
    let check_one = |func_tag: String, codes: &[u8], vmsta: &mut VMState| -> Rerr {
        let check_call = |call: CallSpec, vmsta: &mut VMState| -> Rerr {
            let sign = call.selector();
            let found = resolve_userfn_meta_by_lookup_for_check(
                vmsta,
                root_addr,
                root_contract,
                &func_tag,
                &call,
                &sign,
            )?;
            let Some((owner, meta)) = found else {
                return errf!(
                    "{}: call target function 0x{} not found",
                    func_tag,
                    hex::encode(sign)
                );
            };
            if call.requires_external_visibility() && !meta.is_external {
                return errf!(
                    "{}: target function 0x{} resolved in {} is not external",
                    func_tag,
                    hex::encode(sign),
                    owner.to_readable()
                );
            }
            Ok(())
        };
        scan_call_sites(codes, |inst, params| {
            check_call(
                decode_user_call_site(inst, params).map_err(|e| e.to_string())?,
                vmsta,
            )
        })
    };

    for f in root_contract.userfuncs.as_list() {
        let code_pkg = CodePkg::try_from(&f.code_stuff).map_err(|e| e.to_string())?;
        let ctype = code_pkg.code_type().map_err(|e| e.to_string())?;
        let codes = match ctype {
            CodeType::Bytecode => code_pkg.data,
            CodeType::IRNode => crate::ir::runtime_irs_to_exec_bytecodes(&code_pkg.data, gst)
                .map_err(|e| e.to_string())?,
        };
        let tag = format!("userfn 0x{}", hex::encode(f.sign.into_array()));
        check_one(tag, &codes, vmsta)?;
    }

    for f in root_contract.abstcalls.as_list() {
        let code_pkg = CodePkg::try_from(&f.code_stuff).map_err(|e| e.to_string())?;
        let ctype = code_pkg.code_type().map_err(|e| e.to_string())?;
        let codes = match ctype {
            CodeType::Bytecode => code_pkg.data,
            CodeType::IRNode => crate::ir::runtime_irs_to_exec_bytecodes(&code_pkg.data, gst)
                .map_err(|e| e.to_string())?,
        };
        let tag = format!("abstcall {}", f.sign[0]);
        check_one(tag, &codes, vmsta)?;
    }

    Ok(())
}

fn check_sub_contract_protocol_cost(
    ctx: &mut dyn Context,
    pfee: &Amount,
    charge_bytes: usize,
    periods: u64,
) -> Rerr {
    if pfee.is_negative() {
        return errf!("protocol fee cannot be negative");
    }
    if charge_bytes == 0 {
        return Ok(());
    }
    let min_fee = calc_contract_protocol_cost_min_with_periods(ctx, charge_bytes, periods)?;
    let maddr = ctx.env().tx.main;
    if pfee < &min_fee {
        return errf!(
            "protocol fee must be at least {} (bytes={}, periods={}) but got {}",
            &min_fee,
            charge_bytes,
            periods,
            pfee
        );
    }
    hac_sub(ctx, &maddr, pfee)?;
    Ok(())
}

fn calc_contract_protocol_cost_min_with_periods(
    ctx: &dyn Context,
    charge_bytes: usize,
    periods: u64,
) -> Ret<Amount> {
    if charge_bytes == 0 {
        return Ok(Amount::zero());
    }
    // Height-gated floor via the application-selected VM execution params.
    let fee_purity =
        ctx.services()
            .vm_params()?
            .effective_fee_purity(ctx.env().block.height, ctx.tx().fee_purity()) as u128; // unit-238 per tx byte
    let periods = periods as u128;
    if periods == 0 || fee_purity == 0 {
        return errf!(
            "contract protocol fee calculate failed: periods={} fee_purity={}",
            periods,
            fee_purity
        );
    }
    let bytes = charge_bytes as u128;
    let Some(need) = fee_purity.checked_mul(bytes) else {
        return errf!(
            "contract protocol fee calculate failed: fee_purity * bytes overflow ({} * {})",
            fee_purity,
            bytes
        );
    };
    let Some(need) = need.checked_mul(periods) else {
        return errf!(
            "contract protocol fee calculate failed: required * periods overflow ({} * {})",
            need,
            periods
        );
    };
    Ok(Amount::coin_u128(need, field::UNIT_238))
}

/// Minimum on-chain `protocol_cost` for `charge_bytes` stored `periods` times.
pub fn contract_protocol_cost_min(
    ctx: &dyn Context,
    charge_bytes: usize,
    periods: u64,
) -> Ret<Amount> {
    calc_contract_protocol_cost_min_with_periods(ctx, charge_bytes, periods)
}

// ================================ VM entry bridges ================================

/// Bridge to `VmRequest::Abst` (abst-call VM call). `contract_addr` is the contract,
/// `intent_scope` is `None` (top-level action entries carry no intent binding;
/// intent scopes are an intra-VM construct).
pub(crate) fn run_abst_entry(
    ctx: &mut dyn Context,
    kind: AbstCall,
    contract_addr: ContractAddress,
    param: Value,
) -> Ret<(base::GasBuckets, Box<dyn Any>)> {
    ctx.vm_call(VmEntry::Raw(Box::new(VmRequest::Abst {
        kind,
        contract_addr,
        intent_scope: None,
        param,
    })))
}

// Decoders ----------------------------------------------------------------

pub fn create_contract_deploy(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = ContractDeploy::decode(buf)?;
    Ok((Arc::new(action), used))
}

pub fn create_contract_update(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = ContractUpdate::decode(buf)?;
    Ok((Arc::new(action), used))
}
