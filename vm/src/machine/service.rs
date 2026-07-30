use std::any::Any;
use std::time::Instant;

use base::{Context, GasBuckets, Vm, VmEntry};
use field::Address;
use sys::{Rerr, Ret};

use crate::rt::AbstCall;
use crate::value::{ContractAddress, Value};

use super::{StubVm, VmRequest, VolatileState};

#[derive(Clone)]
struct VmVolatileSnapshot {
    state: VolatileState,
}

impl Vm for StubVm {
    fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    fn call(&mut self, ctx: &mut dyn Context, entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)> {
        match entry {
            VmEntry::TransferAuthorize {
                owner,
                to,
                action_kind,
                payload,
                ..
            } => self.run_transfer_authorize(ctx, owner, to, action_kind, payload, None),
            VmEntry::TransferReceive {
                from, to, payload, ..
            } => self.run_transfer_receive(ctx, from, to, payload, None),
            VmEntry::Raw(req) => {
                let req = req
                    .downcast::<VmRequest>()
                    .map_err(|_| sys::Error::fault("vm raw request type mismatch"))?;
                self.run_request(ctx, *req)
            }
        }
    }

    fn snapshot_volatile(&mut self) -> Box<dyn Any> {
        Box::new(VmVolatileSnapshot {
            state: self.runtime.volatile.clone(),
        })
    }

    fn restore_volatile(&mut self, snap: Box<dyn Any>) {
        let snap = snap
            .downcast::<VmVolatileSnapshot>()
            .expect("volatile snapshot type mismatch");
        self.runtime.volatile = snap.state;
    }

    fn rollback_volatile_preserve_warm_and_gas(&mut self) {
        self.runtime.volatile.global_map.clear();
        self.runtime.volatile.memory_map.clear();
        self.runtime.volatile.intents.clear();
        self.runtime.volatile.deferred_registry.clear();
    }

    fn invalidate_contract_cache(&mut self, addr: &Address) {
        if let Ok(caddr) = ContractAddress::from_addr(*addr) {
            self.runtime.warm.contracts.remove(&caddr);
        }
    }

    fn runtime_config(&mut self) -> Option<Box<dyn Any>> {
        Some(Box::new((
            self.runtime.warm.gas_extra.clone(),
            self.runtime.warm.space_cap.clone(),
        )))
    }

    fn drain_deferred(&mut self, ctx: &mut dyn Context) -> Rerr {
        let callbacks = self.runtime.volatile.deferred_registry.drain_lifo();
        for caddr in callbacks {
            self.run_abst_entry(
                ctx,
                AbstCall::Deferred,
                caddr.addr,
                caddr.intent_scope,
                Value::Nil,
            )?;
        }
        Ok(())
    }
}
