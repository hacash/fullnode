use field::Hash;

#[derive(Clone, Default, Debug)]
pub struct ChainStatus {
    pub latest_height: u64,
    pub latest_hash: Hash,
    pub immature_height: u64,
}
