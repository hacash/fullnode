use std::sync::Arc;

use field::Address;

use super::{Runtime, VmHost};
use crate::contract::{ContractEdition, ContractObj, ContractSto};
use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::ContractAddress;

#[derive(Debug, Clone)]
pub struct ResolvedFn {
    pub owner: ContractAddress,
    pub fnobj: Arc<FnObj>,
    pub lib_table: Arc<[Address]>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCallPlan {
    pub next_bindings: FrameBindings,
    pub fnobj: Arc<FnObj>,
}

impl Runtime {
    #[inline(always)]
    fn require_resolved(found: Option<ResolvedFn>) -> VmrtRes<ResolvedFn> {
        let Some(got) = found else {
            return itr_err_code!(CallNotExist);
        };
        Ok(got)
    }

    fn build_resolved(
        owner: &ContractAddress,
        fnobj: Arc<FnObj>,
        cobj: &ContractObj,
    ) -> ResolvedFn {
        ResolvedFn {
            owner: *owner,
            fnobj,
            lib_table: cobj
                .sto
                .library
                .as_list()
                .iter()
                .map(ContractAddress::to_addr)
                .collect::<Vec<_>>()
                .into(),
        }
    }

    fn load_contract_from_state(
        &mut self,
        addr: &ContractAddress,
        state_ed: &ContractEdition,
        csto: ContractSto,
    ) -> VmrtRes<Arc<ContractObj>> {
        let cobj = Arc::new(csto.into_obj()?);
        if cobj.edition != *state_ed {
            return itr_err_fmt!(
                ContractError,
                "contract edition mismatch {}",
                addr.to_readable()
            );
        }
        Ok(cobj)
    }

    fn resolve_contract<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        addr: &ContractAddress,
    ) -> VmrtRes<Arc<ContractObj>> {
        let Some(state_ed) = host.contract_edition(addr)? else {
            return itr_err_fmt!(
                NotFindContract,
                "cannot find contract edition {}",
                addr.to_readable()
            );
        };
        if let Some(cached) = self.warm.contracts.get(addr) {
            if cached.edition == state_ed {
                return Ok(cached.clone());
            }
            self.warm.contracts.remove(addr);
        }
        if self.warm.contracts.len() >= self.warm.space_cap.loaded_contract {
            return itr_err_code!(OutOfLoadContract);
        }
        let Some(csto) = host.contract(addr)? else {
            return itr_err_fmt!(
                NotFindContract,
                "cannot find contract {}",
                addr.to_readable()
            );
        };
        let cbytes = state_ed.raw_size.uint() as usize;
        let cobj = self.load_contract_from_state(addr, &state_ed, csto)?;
        self.settle_new_contract_load_gas(host, cbytes)?;
        self.warm.contracts.insert(*addr, cobj.clone());
        Ok(cobj)
    }

    pub fn load_contract<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        addr: &ContractAddress,
    ) -> VmrtRes<Arc<ContractObj>> {
        self.resolve_contract(host, addr)
    }

    fn resolve_user_on_owner<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        owner: &ContractAddress,
        selector: FnSign,
    ) -> VmrtRes<Option<ResolvedFn>> {
        let cobj = self.resolve_contract(host, owner)?;
        let Some(fnobj) = cobj.userfns.get(&selector).cloned() else {
            return Ok(None);
        };
        Ok(Some(Self::build_resolved(owner, fnobj, cobj.as_ref())))
    }

    fn resolve_abst_on_owner<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        owner: &ContractAddress,
        selector: AbstCall,
    ) -> VmrtRes<Option<ResolvedFn>> {
        let cobj = self.resolve_contract(host, owner)?;
        let Some(fnobj) = cobj.abstfns.get(&selector).cloned() else {
            return Ok(None);
        };
        Ok(Some(Self::build_resolved(owner, fnobj, cobj.as_ref())))
    }

    pub fn resolve_abstfn<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        addr: &ContractAddress,
        selector: AbstCall,
    ) -> VmrtRes<Option<ResolvedFn>> {
        if let Some(found) = self.resolve_abst_on_owner(host, addr, selector)? {
            return Ok(Some(found));
        }
        let cobj = self.resolve_contract(host, addr)?;
        for parent in cobj.sto.inherit.as_list() {
            if let Some(found) = self.resolve_abst_on_owner(host, parent, selector)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn resolve_lookup_candidates<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        anchor: &ContractAddress,
        call: &CallSpec,
    ) -> VmrtRes<Vec<ContractAddress>> {
        let parents = if call.needs_inherit_chain() {
            self.resolve_contract(host, anchor)?
                .sto
                .inherit
                .as_list()
                .to_vec()
        } else {
            vec![]
        };
        Ok(call.resolve_candidates(anchor, &parents))
    }

    fn resolve_user_call<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        call: &CallSpec,
        bindings: &FrameBindings,
    ) -> VmrtRes<(ContractAddress, ResolvedFn)> {
        let anchor = call.resolve_anchor(bindings)?;
        let entries = self.resolve_lookup_candidates(host, &anchor, call)?;
        let mut found = None;
        for owner in entries {
            if let Some(hit) = self.resolve_user_on_owner(host, &owner, call.selector())? {
                found = Some(hit);
                break;
            }
        }
        Ok((anchor, Self::require_resolved(found)?))
    }

    pub fn plan_user_call<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        call: &CallSpec,
        bindings: &FrameBindings,
    ) -> VmrtRes<ResolvedCallPlan> {
        let (anchor, hit) = self.resolve_user_call(host, call, bindings)?;
        if call.requires_external_visibility() && !hit.fnobj.check_conf(FnConf::External) {
            let vis = &anchor;
            let owner = &hit.owner;
            let impl_in = maybe!(
                vis == owner,
                s!(""),
                format!(" (impl in {})", owner.to_readable())
            );
            return itr_err_fmt!(
                CallNotExternal,
                "contract {}{} func sign {}",
                vis.to_readable(),
                impl_in,
                hex::encode(call.selector())
            );
        }
        let next_bindings =
            bindings.next_after_call(call.switches_context(), anchor, hit.owner, hit.lib_table);
        Ok(ResolvedCallPlan {
            next_bindings,
            fnobj: hit.fnobj,
        })
    }
}
