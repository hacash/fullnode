

pub trait DiskDB : Send + Sync {
    fn read(&self, _: &[u8]) -> Option<Vec<u8>>;
    fn save(&self, _: &[u8], _: &[u8] );
    fn drop(&self, _: &[u8]);
    fn write(&self, _: &MemKV);
    fn write_batch(&self, _: MemBatch);
}
