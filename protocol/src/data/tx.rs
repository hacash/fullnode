
// TxPkg
#[derive(Clone)]
pub struct TxPkg {
	pub time: u64,
	pub hash: Hash,
	pub data: Vec<u8>,
    pub objc: Box<dyn Transaction>,
    pub orgi: TxOrigin,
}


impl TxPkg {
	pub fn fee_purity(&self) -> u64 {
		self.objc.fee_purity()
	}
}