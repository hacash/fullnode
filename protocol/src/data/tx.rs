
// TxPkg
#[derive(Clone)]
pub struct TxPkg {
	pub hash: Hash,
	pub data: Vec<u8>,
    pub objc: Box<dyn Transaction>,
    pub orgi: TxOrigin,
}


impl TxPkg {

	pub fn create(objc: Box<dyn Transaction>) -> Self {
		let data = objc.serialize();
		Self {
			orgi: TxOrigin::UNKNOWN,
			hash: objc.hash(),
			data,
			objc,
		}
	}

	pub fn build(data: Vec<u8>) -> Ret<Self> {
		let (objc, _) = transaction::create(&data)?;
		Ok(Self {
			orgi: TxOrigin::UNKNOWN,
			hash: objc.hash(),
			data,
			objc,
		})
	}

	pub fn into_transaction(self) -> Box<dyn Transaction> {
		self.objc
	}

	
	pub fn fee_purity(&self) -> u64 {
		self.objc.fee_purity()
	}


}