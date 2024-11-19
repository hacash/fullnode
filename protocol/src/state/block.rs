
pub struct BlockDisk {
    disk: Arc<dyn DiskDB>
}

impl BlockDisk {

    pub fn wrap(disk: Arc<dyn DiskDB>) -> BlockDisk {
        Self { disk }
    }

    pub fn status(&self) -> ChainStatus {
        const CSK: &[u8] = b"chain_status";
        let mut stat = ChainStatus::default();
        match self.disk.load(CSK) {
            None => stat,
            Some(v) => {
                stat.parse(&v).unwrap(); // must
                stat
            }
        }
    }

    // save

    pub fn save_block_data(&self, hx: &Hash, data: &Vec<u8>) {
        self.disk.save(hx.as_ref(), &data)
    }

    pub fn save_block_hash(&self, hei: &BlockHeight, hx: &Hash) {
        self.disk.save(&hei.to_bytes(), hx.as_ref())
    }

    pub fn save_block_hash_path(&self, kvs: Vec<(BlockHeight, Hash)>) {
        let pmv: Vec<(Vec<u8>, Vec<u8>)> = kvs.iter().map(|(k, v)|(k.to_vec(), v.into_vec())).collect();
        self.disk.save_batch(pmv)
    }

    // read

    pub fn block_data(&self, hx: &Hash) -> Option<Vec<u8>> {
        self.disk.load(hx.as_ref())
    }

    pub fn block_hash(&self, hei: &BlockHeight) -> Option<Hash> {
        let Some(hx) = self.disk.load(&hei.to_bytes()) else {
            return None
        };
        Some(Hash::must(&hx))
    }
    
    pub fn block_data_by_height(&self, hei: &BlockHeight) -> Option<(Hash, Vec<u8>)> {
        let Some(hx) = self.block_hash(hei) else {
            return None
        };
        self.block_data(&hx).map(|d|(hx, d))
    }

    pub fn block(&self, hx: &Hash) -> Option<(Vec<u8>, Box<dyn Block>)> {
        let Some(data) = self.block_data(&hx) else {
            return None
        };
        // parse
        match block::create(&data).map(|(b,_)|b) {
            Err(..) => None,
            Ok(b) => Some((data, b))
        }
    }
    pub fn block_by_height(&self, hei: &BlockHeight) -> Option<(Hash, Vec<u8>, Box<dyn Block>)> {
        let Some(hx) = self.block_hash(hei) else {
            return None
        };
        let Some((data, block)) = self.block(&hx) else {
            return None
        };
        Some((hx, data, block))
    }


}