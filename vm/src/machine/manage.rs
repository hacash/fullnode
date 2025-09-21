

#[derive(Default)]
pub struct MachineManage {
    lock: Mutex<()>,
    resoures: UnsafeCell<Vec<Resoure>>
}

unsafe impl Sync for MachineManage {}
unsafe impl Send for MachineManage {}

impl MachineManage {

    pub fn new() -> Self {
        Self::default()
    }

    /*
        create a vm machine
    */
    pub fn assign(&self, hei: u64) -> MachineBox {
        let lk = self.lock.lock().unwrap();
        let res = unsafe{ &mut *self.resoures.get() };
        let r = match res.pop() {
            Some(mut r) => { r.reset(hei); r },
            None => Resoure::create(hei),
        };
        drop(lk);
        let mbox = MachineBox::new(Machine::create(r));
        mbox
    }

    pub fn reclaim(&self, r: Resoure) {
        let lk = self.lock.lock().unwrap();
        let res = unsafe{ &mut *self.resoures.get() };
        res.push(r);
        drop(lk);
    } 


}