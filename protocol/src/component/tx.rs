
// TxPkg
#[derive(Clone)]
pub struct TxPkg {
	pub hash: Hash,
	pub data: Vec<u8>,
    pub objc: Box<dyn Transaction>,
	pub fepr: u64, // fee_purity
    pub orgi: TxOrigin,
}


impl TxPkg {

	pub fn create(objc: Box<dyn Transaction>) -> Self {
		let data = objc.serialize();
		let pkg = Self {
			orgi: TxOrigin::Unknown,
			hash: objc.hash(),
			fepr: objc.fee_purity(),
			data,
			objc,
		};
		pkg
	}

	pub fn into_transaction(self) -> Box<dyn Transaction> {
		self.objc
	}
	

}