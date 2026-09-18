use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use base::{Context, GasBuckets, Vm, VmEntry};
use sys::Ret;

/// A VM double for checking ownership/snapshot wiring without interpreting code.
pub struct CounterMockVm {
    counter: Arc<AtomicI64>,
}

impl CounterMockVm {
    pub fn new(counter: Arc<AtomicI64>) -> Self {
        Self { counter }
    }
    pub fn create() -> (Box<dyn Vm>, Arc<AtomicI64>) {
        let counter = Arc::new(AtomicI64::new(0));
        (Box::new(Self::new(counter.clone())), counter)
    }
}
impl Vm for CounterMockVm {
    fn call(&mut self, _ctx: &mut dyn Context, _entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok((GasBuckets::default(), Box::new(())))
    }
    fn snapshot_volatile(&mut self) -> Box<dyn Any> {
        Box::new(self.counter.load(Ordering::SeqCst))
    }
    fn restore_volatile(&mut self, snapshot: Box<dyn Any>) {
        let count = snapshot
            .downcast::<i64>()
            .expect("counter VM snapshot type mismatch");
        self.counter.store(*count, Ordering::SeqCst);
    }
}
pub fn new_counter_vm() -> (Box<dyn Vm>, Arc<AtomicI64>) {
    CounterMockVm::create()
}
