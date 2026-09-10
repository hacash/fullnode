//! ContractDeploy / ContractUpdate execute bodies and store prechecks.

use std::any::Any;

use base::{
    Context, CoreState, VmEntry, consume_block_contract_storage_discount, hac_sub,
    read_block_contract_storage_snapshot, total_add_amount_238, total_add_u8, total_add_u12,
    with_base_total,
};
use field::{Address, Amount, Encode};
use sys::{Rerr, Ret, errf};

use super::contract::{
    ContractDeploy, ContractStoreAnalysis, ContractUpdate, ContractUpdateAnalysis,
    contract_deploy_charge_bytes,
};
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
    let charge_bytes = contract_deploy_charge_bytes(&this.contract);
    // Spend the protocol fee through the unified storage fee boundary: legacy fixed
    // periods before activation, two-phase full/discount classification after (§4.3);
    // fast-sync skips the checks but books quota identically (state convergence).
    enforce_contract_storage_fee(ctx, charge_bytes, &this.protocol_cost)?;
    if !fast_sync || this.protocol_cost.is_positive() {
        // strict deducts after the fee check; fast-sync replay still deducts the
        // paid protocol cost so the burn total below balances with the on-chain
        // deduction (dev parity).
        hac_sub(ctx, &maddr, &this.protocol_cost)?;
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
    // Structural replacements or selector-owner changes are Change; purely additive edits stay Append.
    let is_change = did_structural_change || did_effective_lookup_change;
    // Modification tax: charge the edit payload through the same unified storage
    // fee boundary as deploy, at edit.size() (A05/A06). Discount consumption is
    // never refunded, even for shrink/repair edits that reduce state (§4.3).
    let edit_bytes = this.edit.size();
    let pcost = &this.protocol_cost;
    enforce_contract_storage_fee(ctx, edit_bytes, pcost)?;
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
    let quote = quote_contract_storage_fee(ctx, edit_size)?;
    Ok(ContractUpdateAnalysis {
        address: *address,
        old_contract_size: contract.size(),
        new_contract_size: new_contract.size(),
        edit_size,
        did_structural_change,
        did_effective_lookup_change,
        update_hook: if is_change { Change } else { Append },
        required_protocol_cost: quote.full_price,
        discounted_protocol_cost: quote.discount.as_ref().map(|(amt, _)| amt.clone()),
        discounted_periods: quote.discount.as_ref().map(|(_, periods)| *periods),
    })
}

/// Read-only two-tier storage fee quote for one deploy/update payload (§8.D):
/// `full_price` guarantees inclusion regardless of quota contention; `discount`
/// is the best-effort price under the budget visible from the caller's layer and
/// is present only when the mechanism is active and a live budget snapshot is in
/// scope (block/pending execution). Analysis/tooling outside a block context gets
/// `discount = None` and must quote `full_price` (or fetch the head budget facts).
pub fn quote_contract_storage_fee(
    ctx: &mut dyn Context,
    charge_bytes: usize,
) -> Ret<ContractStorageFeeQuote> {
    let vp = {
        let services = ctx.services();
        *services.vm_params()?
    };
    let height = ctx.env().block.height;
    let p_max = vp.contract_store_perm_periods;
    let full_price = calc_contract_protocol_cost_min_with_periods(ctx, charge_bytes, p_max)?;
    let discount = match base::peek_block_budget_remaining(ctx.layer(), &vp, height)? {
        Some(remaining) => {
            let capacity = vp.contract_storage_fee.capacity_at(height)?;
            let periods = vp
                .contract_storage_fee
                .discount_periods(remaining, capacity, p_max)?;
            let minimum = calc_contract_protocol_cost_min_with_periods(ctx, charge_bytes, periods)?;
            Some((minimum, periods))
        }
        None => None,
    };
    Ok(ContractStorageFeeQuote {
        charge_bytes,
        full_price,
        full_price_periods: p_max,
        discount,
    })
}

/// Two-tier fee quote; see [`quote_contract_storage_fee`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractStorageFeeQuote {
    pub charge_bytes: usize,
    /// Full-price minimum: the guaranteed-inclusion fee.
    pub full_price: Amount,
    pub full_price_periods: u64,
    /// `(discounted minimum, periods)` under the current budget, when known.
    pub discount: Option<(Amount, u64)>,
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

/// Unified two-phase contract storage fee boundary for deploy and update (§8.B1).
///
/// Strict mode:
/// * Pre-activation (or disabled profiles): legacy fixed-period full-price check,
///   `protocol_cost >= required_cost` semantics preserved (A07).
/// * From `H0`: `protocol_cost >= F_full` pays full price and consumes no discount
///   quota; `F_discount <= protocol_cost < F_full` is an all-or-nothing discount
///   transaction that consumes `charge_bytes` from the block quota
///   `min(B_start, K_max)`; anything below `F_discount` is rejected — the user pays
///   full price or waits (§4.3).
///
/// Fast-sync replay performs no fee *checks* (no negative-fee or discount-floor
/// rejection, no discount price-curve evaluation) but must reproduce the exact
/// strict-mode state: a node may switch between fast-sync and strict at any
/// height, so the budget bookkeeping has to converge. Bookkeeping needs only the
/// legacy full-price minimum `F_full` (fixed periods, no curve): a canonical
/// transaction with `protocol_cost < F_full` is by definition discount-classified
/// (strict miners reject anything below the discount floor), so fast-sync books
/// `charge_bytes` for exactly the same transactions strict mode would. A fee below
/// the discount floor can only appear on non-canonical input; fast-sync accepts
/// it under the trusted-input model (same as its other skipped checks). The quota
/// guard inside `consume` stays shared, so over-quota input is refused by both
/// modes — even non-canonical outcomes converge.
///
/// A missing block budget snapshot while active is a lifecycle bug in whichever
/// strict execution path created the layer, not a user error: `Abort` stops the
/// node loudly rather than silently diverging (C6).
fn enforce_contract_storage_fee(
    ctx: &mut dyn Context,
    charge_bytes: usize,
    protocol_cost: &Amount,
) -> Rerr {
    let fast_sync = ctx.env().chain.fast_sync;
    if !fast_sync && protocol_cost.is_negative() {
        return errf!("protocol fee cannot be negative");
    }
    let height = ctx.env().block.height;
    let vp = {
        let services = ctx.services();
        *services.vm_params()?
    };
    let p_max = vp.contract_store_perm_periods;
    let min_full = calc_contract_protocol_cost_min_with_periods(ctx, charge_bytes, p_max)?;
    if !vp.contract_storage_fee.is_active_at(height) {
        if !fast_sync && protocol_cost < &min_full {
            return errf!(
                "protocol fee must be at least {} (bytes={}, periods={}) but got {}",
                &min_full,
                charge_bytes,
                p_max,
                protocol_cost
            );
        }
        return Ok(());
    }
    let snapshot = match read_block_contract_storage_snapshot(ctx.layer(), &vp, height)? {
        Some(snapshot) => snapshot,
        None => {
            return Err(sys::Error::abort(
                "contract storage budget snapshot missing at active height",
            )
            .with_code("core_failed"));
        }
    };
    if protocol_cost >= &min_full {
        // Full price: burns normally and never touches the discount quota (§4.4).
        return Ok(());
    }
    if !fast_sync {
        // Strict-only floor: below the discount price the transaction is invalid.
        let periods = vp.contract_storage_fee.discount_periods(
            snapshot.remaining_start,
            snapshot.capacity,
            p_max,
        )?;
        let min_discount =
            calc_contract_protocol_cost_min_with_periods(ctx, charge_bytes, periods)?;
        if protocol_cost < &min_discount {
            return errf!(
                "protocol fee {} is below the storage fee minimum: at least {} at the current {}-period discount, or {} at full price (bytes={})",
                protocol_cost,
                &min_discount,
                periods,
                &min_full,
                charge_bytes
            );
        }
    }
    // Discount classification: the whole transaction consumes quota or nothing
    // (all-or-nothing, §4.3); the write lands on the tx layer, so a failed
    // transaction rolls it back with the rest of its effects (A11). Fast-sync
    // books identically — state must converge across replay modes.
    consume_block_contract_storage_discount(ctx.layer(), &snapshot, charge_bytes)?;
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

/// Bridge to `VmRequest::Abst` (abst-call VM call). `intent_scope` is `None` for
/// top-level action entries — intent scopes are an intra-VM construct.
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

base::impl_action_execute! {
    ContractDeploy {
        (self, ctx) {
            contract_deploy_execute(self, ctx)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    ContractUpdate {
        (self, ctx) {
            contract_update_execute(self, ctx)?;
            Ok(vec![])
        }
    }
}

// ================================ tests ================================

#[cfg(all(test, feature = "execute"))]
mod contract_deploy_exec_tests {
    use super::*;
    use crate::machine::test_ctx::STUB_VM_PARAMS;
    use base::ActionExecute;
    use field::{BytesW2, Decode, UNIT_238, Uint1, Uint2, Uint4};

    use crate::contract::ContractUserFunc;
    use crate::machine::test_ctx::TestCtx;
    use crate::rt::{Bytecode, CodeConf, CodeStuff};

    /// Supported PRIVAKEY-version main address for the deploy tests.
    fn main_addr() -> Address {
        let mut bytes = [0u8; Address::SIZE];
        bytes[20] = 1;
        Address::from(bytes)
    }

    fn deploy_ctx(fast_sync: bool) -> TestCtx {
        let mut ctx = TestCtx::new();
        ctx.env.chain.fast_sync = fast_sync;
        let addr = main_addr();
        ctx.env.tx.main = addr;
        ctx.tx.0 = addr;
        ctx
    }

    fn prefund(ctx: &mut TestCtx, addr: &Address, amt: &Amount) {
        base::hac_add(ctx, addr, amt).unwrap();
    }

    fn balance_hac(ctx: &mut TestCtx, addr: &Address) -> Amount {
        CoreState::wrap(&mut ctx.layer)
            .balance(addr)
            .unwrap()
            .map(|b| b.hacash)
            .unwrap_or_default()
    }

    fn burn_total(ctx: &mut TestCtx) -> u128 {
        CoreState::wrap(&mut ctx.layer)
            .get_base_total()
            .unwrap()
            .contract_protocol_cost_burn_238
            .uint()
    }

    fn make_deploy(protocol_cost: Amount) -> ContractDeploy {
        let mut act = ContractDeploy::new();
        act.nonce = Uint4::from(7);
        act.protocol_cost = protocol_cost;
        act
    }

    /// Minimal valid non-empty contract: one userfunc with a single END bytecode
    /// instruction, so `size() > 0` and `contract.check` passes.
    fn nonempty_contract() -> ContractSto {
        let mut contract = ContractSto::default();
        let mut f = ContractUserFunc::default();
        f.code_stuff = CodeStuff {
            conf: Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw()),
            data: BytesW2::from(vec![Bytecode::END as u8]).unwrap(),
        };
        contract.userfuncs.push(f).unwrap();
        contract
    }

    /// fast_sync replay of a paid deploy must deduct the protocol cost from the
    /// main address so the burn total below stays symmetric (dev parity).
    #[test]
    fn fast_sync_deploy_deducts_positive_protocol_cost_and_burns() {
        let mut ctx = deploy_ctx(true);
        let addr = ctx.env.tx.main;
        let initial = Amount::coin_u128(1_000_000_000, UNIT_238);
        let cost = Amount::coin_u128(1_234, UNIT_238);
        prefund(&mut ctx, &addr, &initial);

        let mut act = make_deploy(cost.clone());
        act.contract = ContractSto::default();
        let (_, out) = act.execute(&mut ctx).unwrap();
        assert!(out.is_empty());

        let expect = initial.sub_mode_u128(&cost).unwrap();
        assert_eq!(balance_hac(&mut ctx, &addr), expect);
        assert_eq!(burn_total(&mut ctx), 1_234);
    }

    /// Zero protocol cost in fast_sync: no deduction, no burn.
    #[test]
    fn fast_sync_deploy_zero_protocol_cost_skips_deduction_and_burn() {
        let mut ctx = deploy_ctx(true);
        let addr = ctx.env.tx.main;
        let initial = Amount::coin_u128(1_000_000_000, UNIT_238);
        prefund(&mut ctx, &addr, &initial);

        let mut act = make_deploy(Amount::zero());
        act.contract = ContractSto::default();
        let (_, out) = act.execute(&mut ctx).unwrap();
        assert!(out.is_empty());

        assert_eq!(balance_hac(&mut ctx, &addr), initial);
        assert_eq!(burn_total(&mut ctx), 0);
    }

    /// Non-fast-sync (strict) path keeps deducting and burning through
    /// `check_sub_contract_protocol_cost` — regression guard for the untouched branch.
    #[test]
    fn non_fast_sync_deploy_deducts_and_burns() {
        let mut ctx = deploy_ctx(false);
        let addr = ctx.env.tx.main;
        let initial = Amount::coin_u128(1_000_000_000_000_000, UNIT_238);
        prefund(&mut ctx, &addr, &initial);

        let mut act = make_deploy(Amount::zero());
        act.contract = nonempty_contract();
        let size = contract_deploy_charge_bytes(&act.contract);
        let periods = ctx
            .services()
            .vm_params()
            .unwrap()
            .contract_store_perm_periods;
        let min_fee = contract_protocol_cost_min(&ctx, size, periods).unwrap();
        let cost = Amount::coin_u128(min_fee.to_238_u128().unwrap() + 1_000, UNIT_238);
        act.protocol_cost = cost.clone();

        let (_, out) = act.execute(&mut ctx).unwrap();
        assert!(out.is_empty());

        let expect = initial.sub_mode_u128(&cost).unwrap();
        assert_eq!(balance_hac(&mut ctx, &addr), expect);
        assert_eq!(burn_total(&mut ctx), cost.to_238_u128().unwrap());
    }

    // ======================= storage fee budget discount (post-H0) =======================

    /// Storage discount schedule used by the deploy classification tests. H0 is far
    /// above any other test height so the stub stays legacy for existing cases.
    const DISCOUNT_H0: u64 = 900_000;

    fn discount_profile() -> base::VmExecutionParams {
        let mut params = STUB_VM_PARAMS;
        params.contract_storage_fee = base::ContractStorageFeeParams {
            rule_version: base::CONTRACT_STORAGE_RULE_V1,
            activation_height: DISCOUNT_H0,
            target_capacity_blocks: 1_000,
            curve_steps: 1_000,
            period_floor: 10,
            max_block_discount_bytes: 16_384,
            supplement_schedule: Box::leak(vec![(DISCOUNT_H0, 1_000)].into_boxed_slice()),
        };
        params
    }

    fn discount_ctx(fast_sync: bool) -> TestCtx {
        let mut ctx = deploy_ctx(fast_sync);
        ctx.vm_params = discount_profile();
        ctx.env.block.height = DISCOUNT_H0;
        ctx
    }

    /// Seed the persisted budget record on the test layer so a block-start init at
    /// `height` yields `remaining` capacity. Returns (ctx, params).
    fn seeded_discount_ctx(
        fast_sync: bool,
        height: u64,
        remaining: u128,
        capacity: u128,
    ) -> TestCtx {
        let mut ctx = discount_ctx(fast_sync);
        ctx.env.block.height = height;
        let params = ctx.vm_params;
        {
            let layer: &mut dyn base::StateLayer = &mut ctx.layer;
            base::set_contract_storage_budget(
                layer,
                &base::ContractStorageBudget {
                    state_version: field::Uint1::from(base::CONTRACT_STORAGE_BUDGET_VERSION_V1),
                    remaining_bytes: field::Uint12::from_checked(remaining).unwrap(),
                    applied_capacity_bytes: field::Uint12::from_checked(capacity).unwrap(),
                },
            );
            base::init_block_contract_storage_budget(layer, &params, height).unwrap();
        }
        ctx
    }

    fn used_discount_bytes(ctx: &TestCtx) -> u128 {
        ctx.layer
            .0
            .get(base::BLOCK_BUDGET_USED_KEY)
            .map(|bytes| {
                let (v, _) = field::Uint12::decode(bytes.as_slice()).unwrap();
                v.uint()
            })
            .unwrap_or(0)
    }

    fn u238_of(amt: &Amount) -> u128 {
        amt.to_238_u128().unwrap()
    }

    /// A07/A04: before activation the deploy fee keeps the legacy fixed-period
    /// `>=` check; at H0 the missing budget record initializes the budget full
    /// (B0 = C), so the activation block itself already prices the floor
    /// discount: discount-band deploys succeed and book quota.
    #[test]
    fn deploy_before_and_at_activation_stays_full_price() {
        // pre-activation: legacy full price, `protocol_cost >= required`
        let mut ctx = deploy_ctx(false);
        ctx.vm_params = discount_profile();
        ctx.env.block.height = DISCOUNT_H0 - 1;
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(1_000_000_000_000, UNIT_238),
        );
        let size = contract_deploy_charge_bytes(&nonempty_contract());
        let min_full = contract_protocol_cost_min(&ctx, size, 10_000).unwrap();

        let mut below = make_deploy(Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238));
        below.contract = nonempty_contract();
        assert!(
            below.execute(&mut ctx).is_err(),
            "legacy minimum is enforced"
        );
        assert_eq!(used_discount_bytes(&ctx), 0);

        let mut ok = make_deploy(min_full.clone());
        ok.contract = nonempty_contract();
        ok.execute(&mut ctx)
            .unwrap_or_else(|e| panic!("legacy full-price deploy failed: {e}"));
        assert_eq!(used_discount_bytes(&ctx), 0, "legacy rule has no quota");

        // at H0 with the record missing, init derives B_start = C (B0 = C): the
        // budget opens full, so the discount is available at the floor price on
        // the activation block itself
        let mut ctx = discount_ctx(false);
        ctx.env.block.height = DISCOUNT_H0;
        {
            let params = ctx.vm_params;
            base::init_block_contract_storage_budget(&mut ctx.layer, &params, DISCOUNT_H0).unwrap();
        }
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(10_000_000_000_000, UNIT_238),
        );
        let charge_bytes = contract_deploy_charge_bytes(&nonempty_contract());
        let min_full = contract_protocol_cost_min(&ctx, charge_bytes, 10_000).unwrap();
        let min_disc = contract_protocol_cost_min(&ctx, charge_bytes, 10).unwrap();
        assert_eq!(
            u238_of(&min_disc) * 1000,
            u238_of(&min_full),
            "full budget floor price"
        );
        let mut nonce = 1u32;
        let mut deploy_at = |ctx: &mut TestCtx, cost: Amount, body: ContractSto| {
            let mut act = make_deploy(cost);
            act.nonce = Uint4::from(nonce);
            nonce += 1;
            act.contract = body;
            act.execute(ctx)
        };
        // a discount-band deploy succeeds on the activation block and books bytes
        deploy_at(
            &mut ctx,
            Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238),
            nonempty_contract(),
        )
        .unwrap();
        assert_eq!(used_discount_bytes(&ctx), charge_bytes as u128);
        // below the discount floor is still rejected
        assert!(
            deploy_at(
                &mut ctx,
                Amount::coin_u128(u238_of(&min_disc) - 1, UNIT_238),
                nonempty_contract(),
            )
            .is_err()
        );
        // full price passes and books nothing
        deploy_at(&mut ctx, min_full, nonempty_contract()).unwrap();
        assert_eq!(
            used_discount_bytes(&ctx),
            charge_bytes as u128,
            "full price skips quota"
        );
    }

    /// §4.3 classification at a full budget: discounted, full-price (no quota),
    /// and below-every-minimum rejection; the quota consumption equals the
    /// deploy's stable `contract.size() + 64` charge (A05/A08/A09).
    #[test]
    fn deploy_discount_consumes_contract_size_quota_only_below_full_price() {
        let mut ctx = seeded_discount_ctx(false, DISCOUNT_H0 + 1000, 1_000_000, 1_000_000);
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(10_000_000_000_000, UNIT_238),
        );
        let contract = nonempty_contract();
        let size = contract_deploy_charge_bytes(&contract);
        assert_eq!(size, contract.size() + 64);
        let min_full = contract_protocol_cost_min(&ctx, size, 10_000).unwrap();
        let min_disc = contract_protocol_cost_min(&ctx, size, 10).unwrap();
        // full budget ⇒ periods = 10 (floor); the discount minimum is 1000× below
        // the full-price minimum by construction (10 vs 10,000 periods)
        assert_eq!(u238_of(&min_disc) * 1000, u238_of(&min_full));

        let mut nonce = 1u32;
        // discounted deploy pays between the discount and the full minimum
        let mut deploy_at = |ctx: &mut TestCtx, cost: Amount, body: ContractSto| {
            let mut act = make_deploy(cost);
            act.nonce = Uint4::from(nonce);
            nonce += 1;
            act.contract = body;
            act.execute(ctx)
        };
        deploy_at(
            &mut ctx,
            Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238),
            contract.clone(),
        )
        .unwrap();
        assert_eq!(used_discount_bytes(&ctx), size as u128);

        // full-price deploy does not consume any quota
        deploy_at(&mut ctx, min_full.clone(), contract.clone()).unwrap();
        assert_eq!(
            used_discount_bytes(&ctx),
            size as u128,
            "full price skips quota"
        );

        // below the discount minimum is rejected and consumes nothing
        assert!(
            deploy_at(
                &mut ctx,
                Amount::coin_u128(u238_of(&min_disc) - 1, UNIT_238),
                contract,
            )
            .is_err()
        );
        assert_eq!(used_discount_bytes(&ctx), size as u128);
    }

    /// A discount-classified deploy must respect the block quota cap: when B is
    /// low the price is full anyway (F_disc = F_full); when a large deploy cannot
    /// fit in `min(B_start, K_max)` it is rejected below full price.
    #[test]
    fn deploy_rejected_when_discount_does_not_fit_remaining_quota() {
        // B_start = 20,000 ⇒ quota = min(20_000, K_max 16_384) = 16_384;
        // periods at used 980,000 ≈ 9,800, so the discount is almost full price.
        let mut ctx = seeded_discount_ctx(false, DISCOUNT_H0 + 1000, 20_000, 1_000_000);
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(10_000_000_000_000, UNIT_238),
        );
        // craft a contract larger than the block quota (fills the wire via a long
        // dead-code PBUF? simpler: many zero userfuncs inflate size past 16 KiB is
        // overkill — instead use the quota math with a payload above K_max by
        // charging B-start-sized edits). Use a ~10-byte contract and assert the
        // near-full price equals full price minus nothing meaningful is skipped:
        let size = contract_deploy_charge_bytes(&nonempty_contract());
        let min_full = contract_protocol_cost_min(&ctx, size, 10_000).unwrap();
        // remaining quota is 16_384 but B only 20_000; a 17_000-byte deploy
        // would exceed quota, but building one inflates the test; instead confirm
        // that a discounted fee below full price is only granted while D+size<=K:
        // each discounted deploy needs its own nonce (fresh contract address)
        let mut nonce = 1u32;
        let mut discounted_deploy = |ctx: &mut TestCtx| {
            let mut act = make_deploy(Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238));
            act.nonce = Uint4::from(nonce);
            nonce += 1;
            act.contract = nonempty_contract();
            act.execute(ctx)
        };
        discounted_deploy(&mut ctx).expect("first discounted deploy fits the quota");
        assert_eq!(used_discount_bytes(&ctx), size as u128);
        // fill the remaining quota with discounted deploys of the same size
        while (used_discount_bytes(&ctx) as usize) + size <= 16_384 {
            discounted_deploy(&mut ctx).unwrap();
        }
        let used = used_discount_bytes(&ctx);
        let err = discounted_deploy(&mut ctx).unwrap_err();
        assert!(
            !err.to_string().contains("already exists"),
            "expected a quota error, got: {err}"
        );
        assert_eq!(
            used_discount_bytes(&ctx),
            used,
            "rejected deploy must not consume"
        );
    }

    /// §3.4 fixed fee vector: a 10,000-byte deploy at the 50,000 u238/byte floor
    /// prices at the design-note HAC values across the five period rows.
    #[test]
    fn ten_kb_fee_vector_matches_design_notes() {
        let ctx = deploy_ctx(false); // STUB floor 50,000 u238/byte
        let bytes = 10_000usize;
        // periods → HAC, per the §3.4 table (1 HAC = 10^10 u238)
        let table: [(u64, u128); 5] = [
            (10, 5_000_000_000),         // 0.5 HAC
            (1_000, 500_000_000_000),    // 50 HAC
            (5_000, 2_500_000_000_000),  // 250 HAC
            (9_000, 4_500_000_000_000),  // 450 HAC
            (10_000, 5_000_000_000_000), // 500 HAC
        ];
        for (periods, expected_u238) in table {
            let min = contract_protocol_cost_min(&ctx, bytes, periods).unwrap();
            assert_eq!(
                min.to_238_u128().unwrap(),
                expected_u238,
                "10 KB fee vector at periods {periods}"
            );
        }
        // purity above the floor scales linearly (§3.4 closing note)
        let disc = contract_protocol_cost_min(&ctx, bytes, 10)
            .unwrap()
            .to_238_u128()
            .unwrap();
        let full = contract_protocol_cost_min(&ctx, bytes, 10_000)
            .unwrap()
            .to_238_u128()
            .unwrap();
        assert_eq!(disc * 1000, full);
    }

    /// Fast-sync replay runs no fee *checks* but must reproduce the strict-mode
    /// state exactly — a node can switch between fast-sync and strict at any
    /// height: discount-classified deploys (fee below the legacy full-price
    /// minimum) book their quota identically, full-price deploys book nothing.
    /// The discount price-curve floor is not evaluated in fast sync: a fee below
    /// it is accepted and booked (such input cannot occur on a canonical chain —
    /// strict miners reject it — and fast sync trusts its input).
    #[test]
    fn fast_sync_books_identical_quota_without_floor_check() {
        let mut ctx = seeded_discount_ctx(true, DISCOUNT_H0 + 1000, 1_000_000, 1_000_000);
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(10_000_000_000_000, UNIT_238),
        );
        let size = contract_deploy_charge_bytes(&nonempty_contract());
        let min_full = contract_protocol_cost_min(&ctx, size, 10_000).unwrap();
        // discount-band fee (below full price) books quota exactly like strict mode
        let mut disc = make_deploy(Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238));
        disc.contract = nonempty_contract();
        disc.execute(&mut ctx).unwrap();
        assert_eq!(used_discount_bytes(&ctx), size as u128);
        // full price books nothing, same as strict mode
        let mut full = make_deploy(min_full);
        full.contract = nonempty_contract();
        full.execute(&mut ctx).unwrap();
        assert_eq!(
            used_discount_bytes(&ctx),
            size as u128,
            "full price skips quota"
        );
        // below the discount floor: no price-curve check in fast sync (still booked)
        let min_disc = contract_protocol_cost_min(&ctx, size, 10).unwrap();
        let mut below = make_deploy(Amount::coin_u128(u238_of(&min_disc) - 1, UNIT_238));
        below.contract = nonempty_contract();
        below.execute(&mut ctx).unwrap();
        assert_eq!(used_discount_bytes(&ctx), size as u128 * 2);
    }

    // ======================= update path: equal-length / shrink / repair =======================

    /// Build a deployable contract with one user function (default sign) whose code is
    /// `codes`, plus Change/Append abst calls so the update hook execution succeeds.
    fn contract_with_codes(codes: &[u8]) -> ContractSto {
        use crate::contract::{ContractAbstCall, ContractUserFunc};
        use crate::rt::AbstCall as Ac;
        let mut contract = ContractSto::default();
        let mut f = ContractUserFunc::default();
        f.code_stuff = CodeStuff {
            conf: Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw()),
            data: BytesW2::from(codes.to_vec()).unwrap(),
        };
        contract.userfuncs.push(f).unwrap();
        for kind in [Ac::Change as u8, Ac::Append as u8] {
            let mut a = ContractAbstCall::default();
            a.sign = field::Fixed1::from([kind]);
            a.code_stuff = CodeStuff {
                conf: Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw()),
                data: BytesW2::from(vec![Bytecode::END as u8]).unwrap(),
            };
            contract.abstcalls.push(a).unwrap();
        }
        contract
    }

    /// Replace the deployed default-sign function's code with `new_codes` through a
    /// `ContractUpdate`, and return (used delta, stored contract, edit).
    fn run_code_replacing_update(
        ctx: &mut TestCtx,
        caddr: &ContractAddress,
        old_revision: u64,
        new_codes: &[u8],
    ) -> (u128, ContractSto) {
        use crate::contract::ContractUserFunc;
        let addr = caddr.to_addr();
        let edit = {
            let mut edit = crate::contract::ContractEdit::default();
            edit.new_revision = Uint2::from(old_revision as u16 + 1);
            let mut f = ContractUserFunc::default();
            f.code_stuff = CodeStuff {
                conf: Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw()),
                data: BytesW2::from(new_codes.to_vec()).unwrap(),
            };
            edit.userfuncs.push(f).unwrap();
            edit
        };
        let edit_size = edit.size();
        let before = used_discount_bytes(ctx);
        // discounted fee: between the discount and full minimums
        let quote = quote_contract_storage_fee(ctx, edit_size).unwrap();
        let Some((disc_min, _periods)) = &quote.discount else {
            panic!("update analysis must see an active budget snapshot");
        };
        let fee = Amount::coin_u128(u238_of(disc_min) + 1, UNIT_238);
        let mut update = ContractUpdate::new();
        update.protocol_cost = fee;
        update.address = addr;
        update.edit = edit;
        update.execute(ctx).unwrap();
        let delta = used_discount_bytes(ctx) - before;
        assert_eq!(
            delta, edit_size as u128,
            "update consumes edit.size() exactly"
        );
        let stored = crate::state::VMState::wrap(&mut ctx.layer)
            .contract(caddr)
            .unwrap()
            .expect("updated contract must exist");
        (delta, stored)
    }

    /// A06: equal-length replacement consumes `edit.size()`, charges, and neither
    /// the discount quota nor the fee is refunded even though storage may shrink.
    #[test]
    fn update_equal_length_and_shrink_consume_edit_size_without_refund() {
        // full budget ctx, discount floor periods = 10
        let mut ctx = seeded_discount_ctx(false, DISCOUNT_H0 + 1000, 1_000_000, 1_000_000);
        ctx.install_native_vm();
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(10_000_000_000_000, UNIT_238),
        );

        // deploy with two-byte code [P0, END]
        let deploy_codes = vec![Bytecode::P0 as u8, Bytecode::END as u8];
        let contract = contract_with_codes(&deploy_codes);
        let contract_size = contract.size();
        let deploy_charge_bytes = contract_deploy_charge_bytes(&contract);
        let nonce = Uint4::from(1);
        let caddr = ContractAddress::calculate(&addr, &nonce);
        let min_full = contract_protocol_cost_min(&ctx, deploy_charge_bytes, 10_000).unwrap();
        let mut deploy = make_deploy(Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238));
        deploy.nonce = nonce;
        deploy.contract = contract;
        deploy.execute(&mut ctx).unwrap();
        let used_after_deploy = used_discount_bytes(&ctx);
        assert_eq!(used_after_deploy, deploy_charge_bytes as u128);

        // equal-length replacement: same code length, new op. `run_code_replacing_update`
        // asserts the quota delta equals the full encoded edit size; the stored size
        // must stay identical because the replacement payload has the same length.
        let equal_codes = vec![Bytecode::P1 as u8, Bytecode::END as u8];
        let (_delta, stored_eq) = run_code_replacing_update(&mut ctx, &caddr, 0, &equal_codes);
        assert_eq!(
            stored_eq.size(),
            contract_size,
            "equal-length keeps the same size"
        );

        // shrink: replace with a one-byte code
        let shrink_codes = vec![Bytecode::END as u8];
        let (delta_shrink, stored_shrink) =
            run_code_replacing_update(&mut ctx, &caddr, 1, &shrink_codes);
        assert_eq!(
            stored_shrink.size(),
            contract_size - 1,
            "contract actually shrank"
        );
        assert_eq!(
            delta_shrink,
            // edit.size() is the full replacement payload length, not the size delta
            {
                // recompute the shrink edit size directly
                let mut edit = crate::contract::ContractEdit::default();
                edit.new_revision = Uint2::from(2u16);
                let mut f = crate::contract::ContractUserFunc::default();
                f.code_stuff = CodeStuff {
                    conf: Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw()),
                    data: BytesW2::from(shrink_codes).unwrap(),
                };
                edit.userfuncs.push(f).unwrap();
                edit.size() as u128
            },
            "shrink edit consumes its edit.size(), never a negative refund"
        );
        assert!(
            used_discount_bytes(&ctx) > used_after_deploy,
            "no discount credit is ever returned for a shrink"
        );
    }

    /// The update analysis layer quotes both tiers: the discount price applies
    /// when a live budget snapshot is in scope; offline contexts get full price.
    #[test]
    fn analyze_contract_update_reports_discount_quote() {
        let mut ctx = seeded_discount_ctx(false, DISCOUNT_H0 + 1000, 1_000_000, 1_000_000);
        ctx.install_native_vm();
        let addr = ctx.env.tx.main;
        prefund(
            &mut ctx,
            &addr,
            &Amount::coin_u128(10_000_000_000_000, UNIT_238),
        );
        let contract = contract_with_codes(&[Bytecode::P0 as u8, Bytecode::END as u8]);
        let nonce = Uint4::from(1);
        let caddr = ContractAddress::calculate(&addr, &nonce);
        let min_full =
            contract_protocol_cost_min(&ctx, contract_deploy_charge_bytes(&contract), 10_000)
                .unwrap();
        let mut deploy = make_deploy(Amount::coin_u128(u238_of(&min_full) - 1, UNIT_238));
        deploy.nonce = nonce;
        deploy.contract = contract;
        deploy.execute(&mut ctx).unwrap();

        let mut edit = crate::contract::ContractEdit::default();
        edit.new_revision = Uint2::from(1u16);
        let mut f = crate::contract::ContractUserFunc::default();
        f.code_stuff = CodeStuff {
            conf: Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw()),
            data: BytesW2::from(vec![Bytecode::P1 as u8, Bytecode::END as u8]).unwrap(),
        };
        edit.userfuncs.push(f).unwrap();
        let analysis = analyze_contract_update(&mut ctx, &caddr, &edit).unwrap();
        let discount = analysis
            .discounted_protocol_cost
            .expect("live snapshot must produce a discount quote");
        assert!(u238_of(&discount) < u238_of(&analysis.required_protocol_cost));
        assert!(analysis.discounted_periods.is_some());
        // the quoted discount fee must be sufficient to pass the update fee check
        let mut update = ContractUpdate::new();
        update.protocol_cost = discount;
        update.address = caddr.to_addr();
        update.edit = edit;
        update.execute(&mut ctx).unwrap();
    }
}
