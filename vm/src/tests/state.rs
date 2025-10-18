use protocol::component::*;

#[derive(Default)]
pub struct StateMem {
    mem: MemKV
}

impl State for StateMem {
    fn get(&self, k: Vec<u8>) -> Option<Vec<u8>> {
        match self.mem.get(&k) {
            Some(v) => v,
            _ => None,
        }
    }
    fn set(&mut self, k: Vec<u8>, v: Vec<u8>) {
        self.mem.put(k, v)
    }
    fn del(&mut self, k: Vec<u8>) {
        self.mem.del(k) // add del mark
    }
}


#[derive(Default)]
pub struct ExtCallMem {
    hei: u64,
}

impl ExtActCal for ExtCallMem {
    fn height(&self) -> u64 {
        self.hei
    }
    fn action_call(&mut self, _: u16, _: Vec<u8>) -> Ret<(u32, Vec<u8>)> {
        Ok((8, vec![1]))
    }

}

