
#[derive(Clone)]
pub enum MemItem{
    Delete,   
    Value(Vec<u8>),
}

impl Display for MemItem {
    fn fmt(&self,f: &mut Formatter) -> Result {
        write!(f,"{}", match self {
            Self::Delete => "[delete]".to_owned(),
            Self::Value(v) => v.hex(),
        })
    }
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