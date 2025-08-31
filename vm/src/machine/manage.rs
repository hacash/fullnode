
#[derive(Default)]
pub struct MachineManage {
    
    resoures: Mutex<Vec<Resoure>>
}


impl MachineManage {

    pub fn new() -> Self {
        Self::default()
    }

    /*
        create a vm machine
    */
    pub fn assign(&self, hei: u64) -> MachineBox {
        let r = match self.resoures.lock().unwrap().pop() {
            Some(mut r) => { r.reset(hei); r },
            None => Resoure::create(hei),
        };
        MachineBox::new(Machine::create(r))
    }

    pub fn reclaim(&self, r: Resoure) {
        self.resoures.lock().unwrap().push(r);
    } 


}