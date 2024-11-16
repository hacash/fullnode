
pub const KNOWLEDGE_SIZE: usize = 32;
pub type KnowKey = [u8; KNOWLEDGE_SIZE];

#[derive(Debug)]
pub struct Knowledge {
    size: usize,
    data: StdMutex<VecDeque<KnowKey>>,
}


impl Knowledge {
    pub fn new(sz: usize) -> Knowledge {
        Knowledge{
            size: sz,
            data: VecDeque::with_capacity(sz+1).into(),
        }
    }

    pub fn add(&self, key: KnowKey) {
        let mut dt = self.data.lock().unwrap();
        if dt.len() >= self.size {
            dt.back(); // drop tail
        }
        dt.push_front(key);
    }

    pub fn check(&self, key: &KnowKey) -> bool {
        self.data.lock().unwrap().iter().any(|a|a==key)
    }






}