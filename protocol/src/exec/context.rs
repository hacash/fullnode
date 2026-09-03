//! `ContextInst` — standard Hacash execution context.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use base::{
    ActOut, ActionDispatcher, Context, Env, ExecFrom, ExecutionServices, LogEntry, P2sh,
    StateChunkRef, StateLayer, TexLedger, Transaction, TxRef, Vm,
};
use field::Address;
use sys::{Rerr, Ret, errf};

use super::gas::{GasDiag, TxGasMeter, gas_initialize_on, gas_refund_on};

/// Diagnostic snapshot of `ContextInst` (reserved for future debug tooling).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDiag {
    pub exec_from: ExecFrom,
    pub p2sh_count: usize,
    pub gas: GasDiag,
    pub vm_initialized: bool,
}

pub struct ContextInst {
    env: Env,
    services: Arc<dyn ExecutionServices>,
    chunk: StateChunkRef,
    tx: TxRef,
    gas: TxGasMeter,
    exec_from: ExecFrom,
    psh: HashMap<Address, Box<dyn P2sh>>,
    check_sign_cache: HashMap<Address, Ret<()>>,
    /// `None` with `vm_active == false` means lazily uninitialized; `None`
    /// with `vm_active == true` means the VM currently owns the call stack.
    vm: Option<Box<dyn Vm>>,
    vm_active: bool,
    tex: TexLedger,
}

#[allow(dead_code)] // diagnostic / reserved methods for future debug tooling
impl ContextInst {
    pub fn new(
        env: Env,
        services: Arc<dyn ExecutionServices>,
        chunk: StateChunkRef,
        tx: TxRef,
    ) -> Ret<Self> {
        chunk.validate_tx_identity(&tx.hash())?;
        Ok(Self {
            env,
            services,
            chunk,
            tx,
            gas: TxGasMeter::new(),
            exec_from: ExecFrom::Top,
            psh: HashMap::new(),
            check_sign_cache: HashMap::new(),
            vm: None,
            vm_active: false,
            tex: TexLedger::default(),
        })
    }

    pub fn services_ref(&self) -> &Arc<dyn ExecutionServices> {
        &self.services
    }

    pub fn gas_diag(&self) -> GasDiag {
        self.gas.diag()
    }

    pub fn diag(&self) -> ContextDiag {
        ContextDiag {
            exec_from: self.exec_from,
            p2sh_count: self.psh.len(),
            gas: self.gas_diag(),
            vm_initialized: self.vm.is_some(),
        }
    }

    fn ensure_vm_ready(&mut self) {
        if self.vm.is_none() && !self.vm_active {
            self.vm = Some(
                self.services
                    .assign_vm(self.env.block.height)
                    .unwrap_or_else(|| Box::new(base::EmptyVm)),
            );
        }
    }

    fn check_sign_cached(&mut self, adr: &Address) -> Rerr {
        if self.env.chain.fast_sync {
            return Ok(());
        }
        if let Some(cached) = self.check_sign_cache.get(adr) {
            return cached.clone();
        }
        if adr.is_privkey_unknown() {
            let err: Rerr = errf!(
                "address {} is a system address (value < u32::MAX) with unknown private key",
                adr.to_readable()
            );
            self.check_sign_cache.insert(*adr, err.clone());
            return err;
        }
        if let Err(e) = adr.must_privkey() {
            self.check_sign_cache.insert(*adr, Err(e.clone()));
            return Err(e);
        }
        let isok = crate::tx_std::verify_target_signature(adr, self.tx.as_ref()).map(|_| ());
        self.check_sign_cache.insert(*adr, isok.clone());
        isok
    }
}

impl Context for ContextInst {
    fn services(&self) -> Arc<dyn ExecutionServices> {
        self.services.clone()
    }
    fn env(&self) -> &Env {
        &self.env
    }
    fn tx(&self) -> &dyn Transaction {
        self.tx.as_ref()
    }
    fn exec_from(&self) -> ExecFrom {
        self.exec_from
    }
    fn exec_from_set(&mut self, from: ExecFrom) {
        self.exec_from = from;
    }
    fn check_sign(&mut self, adr: &Address) -> Rerr {
        self.check_sign_cached(adr)
    }

    fn layer(&mut self) -> &mut dyn StateLayer {
        &mut self.chunk
    }

    fn emit_log(&mut self, entry: LogEntry) {
        self.chunk.emit_log(entry);
    }

    fn gas_remaining(&self) -> i64 {
        self.gas.remaining()
    }
    fn gas_charge(&mut self, gas: i64) -> Rerr {
        self.gas.charge(gas)
    }
    fn gas_rebate(&mut self, gas: i64) -> Rerr {
        // Allowed in any exec_from (VM CALL sites rebate; matches mainnet).
        self.gas.rebate(gas)
    }
    fn gas_initialize(&mut self, budget: i64) -> Rerr {
        if self.exec_from != ExecFrom::Top {
            return errf!("gas_initialize only allowed in TOP context");
        }
        let mut gas = std::mem::take(&mut self.gas);
        let res = gas_initialize_on(&mut gas, self, budget);
        self.gas = gas;
        res
    }
    fn gas_refund(&mut self) -> Rerr {
        if self.exec_from != ExecFrom::Top {
            return errf!("gas_refund only allowed in TOP context");
        }
        let mut gas = std::mem::take(&mut self.gas);
        let res = gas_refund_on(&mut gas, self);
        self.gas = gas;
        res
    }

    fn snapshot_volatile(&self) -> Box<dyn Any> {
        Box::new(self.gas.rebated_checkpoint())
    }
    fn restore_volatile(&mut self, snap: Box<dyn Any>) {
        let rebated = *snap.downcast::<i64>().expect("gas snapshot type mismatch");
        // AST rollback: keep gas_charge effects, roll back rebate only.
        self.gas.restore_rebated(rebated);
    }

    fn action_call(&mut self, kind: u16, body: Vec<u8>) -> Ret<ActOut> {
        let mut buf = Vec::with_capacity(2 + body.len());
        buf.extend_from_slice(&kind.to_be_bytes());
        buf.extend_from_slice(&body);

        let reg = self.services.clone();
        let (action, used) = reg.decode_action(&buf)?;
        if used != buf.len() {
            return errf!(
                "action parse length mismatch: consumed {} but body length is {}",
                used,
                buf.len()
            );
        }
        ActionDispatcher::dispatch_call(self, &action)
    }

    fn exec_ast_child(
        &mut self,
        run: &mut dyn FnMut(&mut dyn Context) -> Ret<ActOut>,
    ) -> Ret<ActOut> {
        let child = self.chunk.spawn_ast_child()?;
        let parent = std::mem::replace(&mut self.chunk, child);
        let res = run(self);
        let child = std::mem::replace(&mut self.chunk, parent);
        match res {
            Ok(out) => {
                let parent = child.commit_to_parent()?;
                debug_assert!(parent.ptr_eq(&self.chunk));
                Ok(out)
            }
            Err(err) => {
                child.discard()?;
                Err(err)
            }
        }
    }

    fn vm_take(&mut self) -> Option<Box<dyn Vm>> {
        if self.vm_active {
            return None;
        }
        self.ensure_vm_ready();
        self.vm_active = true;
        self.vm.take()
    }
    fn vm_put(&mut self, vm: Box<dyn Vm>) {
        debug_assert!(self.vm_active, "VM returned without an active take");
        self.vm = Some(vm);
        self.vm_active = false;
    }
    fn vm_peek(&mut self) -> Option<&mut (dyn Vm + 'static)> {
        if self.vm_active {
            return None;
        }
        self.ensure_vm_ready();
        self.vm.as_deref_mut()
    }
    fn as_context_mut(&mut self) -> &mut dyn Context {
        self
    }

    fn release_chunk(self: Box<Self>) -> Ret<StateChunkRef> {
        Ok(self.chunk)
    }

    fn tex_ledger(&self) -> &TexLedger {
        &self.tex
    }
    fn tex_ledger_mut_top(&mut self) -> Ret<&mut TexLedger> {
        if self.exec_from != ExecFrom::Top {
            return errf!(
                "tex ledger write only allowed in TOP context, got {}",
                self.exec_from
            );
        }
        Ok(&mut self.tex)
    }

    fn p2sh(&self, addr: &Address) -> Ret<&dyn P2sh> {
        match self.psh.get(addr) {
            Some(b) => Ok(b.as_ref()),
            None => errf!("p2sh '{}' not found", addr.to_readable()),
        }
    }
    fn p2sh_count(&self) -> usize {
        self.psh.len()
    }
    fn p2sh_set(&mut self, addr: Address, p2sh: Box<dyn P2sh>) -> Rerr {
        if self.exec_from != ExecFrom::Top {
            return errf!(
                "p2sh_set only allowed in TOP context, got {}",
                self.exec_from
            );
        }
        addr.must_scriptmh()?;
        if self.psh.contains_key(&addr) {
            return errf!("p2sh '{}' already proved in current tx", addr.to_readable());
        }
        self.psh.insert(addr, p2sh);
        Ok(())
    }
}
