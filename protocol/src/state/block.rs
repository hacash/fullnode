
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