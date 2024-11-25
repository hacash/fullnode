
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
		let mut pkg = Self {
			orgi: TxOrigin::UNKNOWN,
			hash: objc.hash(),
			fepr: 0,
			data,
			objc,
		};
		pkg.fepr = pkg.calc_fee_purity();
		pkg
	}

	pub fn build(data: Vec<u8>) -> Ret<Self> {
		let (objc, _) = transaction::create(&data)?;
		let mut pkg = Self {
			orgi: TxOrigin::UNKNOWN,
			hash: objc.hash(),
			fepr: 0,
			data,
			objc,
		};
		pkg.fepr = pkg.calc_fee_purity();
		Ok(pkg)
	}

	pub fn into_transaction(self) -> Box<dyn Transaction> {
		self.objc
	}

	fn calc_fee_purity(&self) -> u64 {
		let txsz = self.data.len() as u64;
		let feeshuo = self.objc.fee_got().to_shuo_unsafe() as u64;
		feeshuo / txsz
	}

}