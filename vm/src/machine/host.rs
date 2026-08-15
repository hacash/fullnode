use base::{Context, LogEntry, TransferRouting, VmHostActionDef, VmHostCallKind};
use field::{Address, Encode};
use sys::Ret;

use crate::contract::{ContractEdition, ContractSto};
use crate::rt::{FrameBindings, GasExtra, ItrErr, ItrErrCode, SpaceCap, VmrtErr, VmrtRes};
use crate::state::{VMState, VmLog};
use crate::value::{ContractAddress, Value};

pub trait VmHost {
    fn height(&self) -> u64;
    fn main_entry_bindings(&self) -> FrameBindings;
    fn gas_remaining(&self) -> i64;
    fn gas_charge(&mut self, gas: i64) -> VmrtErr;
    fn gas_rebate(&mut self, gas: i64) -> VmrtErr;
    fn contract_edition(&mut self, addr: &ContractAddress) -> VmrtRes<Option<ContractEdition>>;
    fn contract(&mut self, addr: &ContractAddress) -> VmrtRes<Option<ContractSto>>;
    /// Host capability metadata from Registry (action / env / view).
    fn vm_host_def(&self, kind: VmHostCallKind, id: u8) -> Option<VmHostActionDef>;
    fn action_call(&mut self, kind: u16, body: Vec<u8>) -> Ret<(u32, Vec<u8>)>;

    /// Resolve the transfer-hook routing for a runtime-dispatched action
    /// (`kind` || `body`), unified with the Top/Ast dispatch tail through
    /// `base::resolve_transfer_routing`. Returns `Some` when the action is a
    /// transfer whose from/to requires VM authorize/receive hooks; `None`
    /// otherwise (non-transfer, or EOA-to-EOA).
    ///
    /// Used by the Call-path interpreter: after `action_call` dispatches (and
    /// the dispatch tail skips hooks per slot law), the interpreter resolves
    /// owned routing data and immediately asks its owning StubVm to recurse.
    fn action_transfer_routing(&self, kind: u16, body: &[u8]) -> Ret<Option<TransferRouting>> {
        let _ = (kind, body);
        Ok(None)
    }

    fn log_push(&mut self, addr: &Address, items: Vec<Value>) -> VmrtErr;

    fn sget_gas(&mut self, gas: &GasExtra, value: &Value) -> i64 {
        VMState::status_get_gas(gas, value)
    }

    fn sput_gas(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        key: &Value,
        value: &Value,
    ) -> VmrtRes<i64> {
        VMState::status_put_gas(gas, cap, key, value)
    }

    fn sget(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: &Value,
    ) -> VmrtRes<Value>;

    fn sput(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        value: Value,
    ) -> VmrtErr;

    fn sstat(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: &Value,
    ) -> VmrtRes<Value>;

    fn sload(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: &Value,
    ) -> VmrtRes<Value>;

    fn sdel(&mut self, gas: &GasExtra, cap: &SpaceCap, addr: &Address, key: Value) -> VmrtRes<i64>;

    fn snew(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        value: Value,
        period: Value,
    ) -> VmrtRes<i64>;

    fn sedit(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        value: Value,
    ) -> VmrtRes<(i64, i64)>;

    fn srent(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        period: Value,
    ) -> VmrtRes<i64>;

    fn srecv(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        period: Value,
    ) -> VmrtRes<i64>;
}

impl<T: Context + ?Sized> VmHost for T {
    fn height(&self) -> u64 {
        self.env().block.height
    }

    fn main_entry_bindings(&self) -> FrameBindings {
        FrameBindings::root(self.tx().main(), self.env().tx.addrs.clone().into())
    }

    fn gas_remaining(&self) -> i64 {
        Context::gas_remaining(self)
    }

    fn gas_charge(&mut self, gas: i64) -> VmrtErr {
        Context::gas_charge(self, gas)
            .map_err(|e| ItrErr::new(ItrErrCode::OutOfGas, &e.to_string()))
    }

    fn gas_rebate(&mut self, gas: i64) -> VmrtErr {
        Context::gas_rebate(self, gas)
            .map_err(|e| ItrErr::new(ItrErrCode::GasError, &e.to_string()))
    }

    fn contract_edition(&mut self, addr: &ContractAddress) -> VmrtRes<Option<ContractEdition>> {
        VMState::wrap(self.layer()).contract_edition(addr)
    }

    fn contract(&mut self, addr: &ContractAddress) -> VmrtRes<Option<ContractSto>> {
        VMState::wrap(self.layer()).contract(addr)
    }

    fn vm_host_def(&self, kind: VmHostCallKind, id: u8) -> Option<VmHostActionDef> {
        self.services().vm_host_def(kind, id).cloned()
    }

    fn action_call(&mut self, kind: u16, body: Vec<u8>) -> Ret<(u32, Vec<u8>)> {
        Context::action_call(self, kind, body)
    }

    fn action_transfer_routing(&self, kind: u16, body: &[u8]) -> Ret<Option<TransferRouting>> {
        // Rebuild the action wire bytes (kind || body) and decode, then resolve
        // routing via the unified `base::resolve_transfer_routing` -- the same
        // function used by the Top/Ast dispatch tail, so from/to/amount/payload
        // extraction and `is_scriptmh`/`is_contract` gating are identical.
        let mut buf = Vec::with_capacity(2 + body.len());
        buf.extend_from_slice(&kind.to_be_bytes());
        buf.extend_from_slice(body);
        let reg = self.services();
        let (action, used) = reg.decode_action(&buf)?;
        if used != buf.len() {
            return sys::errf!(
                "action parse length mismatch: consumed {} but body length is {}",
                used,
                buf.len()
            );
        }
        base::resolve_transfer_routing_on(action.as_ref(), self)
    }

    fn log_push(&mut self, addr: &Address, items: Vec<Value>) -> VmrtErr {
        let log = VmLog::new(*addr, items)?;
        self.emit_log(LogEntry {
            topic: "vm".to_owned(),
            data: log.encode(),
        });
        Ok(())
    }

    fn sget(
        &mut self,
        _gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: &Value,
    ) -> VmrtRes<Value> {
        VMState::wrap(self.layer()).sget(cap, addr, key)
    }

    fn sput(
        &mut self,
        _gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        value: Value,
    ) -> VmrtErr {
        VMState::wrap(self.layer()).sput(cap, addr, key, value)
    }

    fn sstat(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: &Value,
    ) -> VmrtRes<Value> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).sstat(gas, cap, height, addr, key)
    }

    fn sload(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: &Value,
    ) -> VmrtRes<Value> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).sload(gas, cap, height, addr, key)
    }

    fn sdel(&mut self, gas: &GasExtra, cap: &SpaceCap, addr: &Address, key: Value) -> VmrtRes<i64> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).sdel(gas, cap, height, addr, key)
    }

    fn snew(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        value: Value,
        period: Value,
    ) -> VmrtRes<i64> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).snew(gas, cap, height, addr, key, value, period)
    }

    fn sedit(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        value: Value,
    ) -> VmrtRes<(i64, i64)> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).sedit(gas, cap, height, addr, key, value)
    }

    fn srent(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        period: Value,
    ) -> VmrtRes<i64> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).srent(gas, cap, height, addr, key, period)
    }

    fn srecv(
        &mut self,
        gas: &GasExtra,
        cap: &SpaceCap,
        addr: &Address,
        key: Value,
        period: Value,
    ) -> VmrtRes<i64> {
        let height = self.env().block.height;
        VMState::wrap(self.layer()).srecv(gas, cap, height, addr, key, period)
    }
}
