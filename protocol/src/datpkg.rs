
// TxPkg
#[derive(Default, Clone)]
pub struct TxPkg {
	pub time: u64,
	pub hash: Hash,
	pub data: Vec<u8>,
    // pub objc: Box<dyn Transaction>,
    pub orgi: TxOrigin,
}




// BlockPkg
#[derive(Clone)]
pub struct BlockPkg {
	pub hein: u64,
	pub hash: Hash,
	pub data: Vec<u8>,
    pub objc: Box<dyn Block>,
    pub orgi: BlkOrigin,
}




