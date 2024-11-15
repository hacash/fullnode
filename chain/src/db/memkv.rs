
#[derive(Clone)]
pub enum  MemItem{
    Delete,   
    Value(Vec<u8>),
}

pub struct MemKV(HashMap<Vec<u8>, MemItem>);

impl MemKV {

    pub fn new() -> MemKV {
        Self(HashMap::default())
    }

    pub fn del(&mut self, k: Vec<u8>) {
        self.0.insert(k, MemItem::Delete);
    }

    pub fn put(&mut self, k: Vec<u8>, v: Vec<u8>) {
        self.0.insert(k, MemItem::Value(v));
    }

    pub fn get(&self, k: &Vec<u8>) -> Option<MemItem> {
        match self.0.get(k) {
            None => None,
            Some(item) => Some(item.clone()),
        }
    }

}