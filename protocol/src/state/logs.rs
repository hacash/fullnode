
pub struct EmptyLogs {}
impl Logs for EmptyLogs {}


pub struct BlockLogs {
    bhei: u64,
    logs: Vec<Vec<u8>>,
    disk: Arc<dyn DiskDB>
}


impl Logs for BlockLogs {
    fn push(&mut self, stuff: &dyn Serialize) {
        if self.bhei == 0 {
            return // not save
        }
        self.logs.push(stuff.serialize())
    }

    fn load(&self, hei: u64, idx: usize) -> Option<Vec<u8>> {
        let key = Self::wnk(hei, idx);
        self.disk.read(&key)
    }

    fn remove(&self, height: u64) {
        let hei = Uint8::from(height);
        let lnk = [hei.serialize(), b"n".to_vec()].concat();
        let num = self.read_len(&lnk);
        for i in 0 .. num {
            let k = Self::wnk(height, i);
            self.disk.remove(&k);
        }
    }

    fn height(&self) -> u64 {
        self.bhei
    }

    fn write_to_disk(&self) {
        let m = self.logs.len();
        for i in 0 .. m {
            self.disk.save(&self.nk(i), &self.logs[i]);
        }
        if m > 0 {
            self.update_len(self.logs.len());
        }
    }

}


impl BlockLogs {

    fn lnk(&self) -> Vec<u8> {
        let hei = Uint8::from(self.bhei);
        [hei.serialize(), b"n".to_vec()].concat()
    }

    fn wnk(hei: u64, idx: usize) -> Vec<u8> {
        let hei = Uint8::from(hei);
        let num = Uint8::from(idx as u64);
        [hei.serialize(), num.serialize()].concat()
    }

    fn nk(&self, n: usize) -> Vec<u8> {
        Self::wnk(self.bhei, n)
    }

    pub fn wrap(disk: Arc<dyn DiskDB>) -> Self {
        Self { disk, bhei: 0, logs: Vec::new() }
    }

    pub fn next(&self, hei: u64) -> Self {
        Self { disk: self.disk.clone(), bhei: hei, logs: Vec::new() }
    }

}

impl BlockLogs {

    fn read_len(&self, lnk: &Vec<u8>) -> usize {
        let mut num = Uint8::from(0);
        match self.disk.read(lnk) {
            None => num,
            Some(v) => {
                num.parse(&v).unwrap(); // must
                num
            }
        }.uint() as usize
    }

    pub fn len(&self) -> usize {
        let l = self.logs.len();
        if l > 0 {
            return l
        }
        self.read_len(&self.lnk())
    }

    // load
    pub fn read(&self, idx: usize) -> Option<Vec<u8>> {
        if idx < self.logs.len() {
            return Some(self.logs[idx].clone())
        }
        let i = &self.nk(idx);
        self.disk.read(i)
    }


    // write

    fn update_len(&self, n: usize) {
        self.disk.save(&self.lnk(), &self.nk(n));
    }


}