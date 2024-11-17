

// BlockPkg
#[derive(Clone)]
pub struct BlockPkg {
	pub hein: u64,
	pub hash: Hash,
	pub data: Vec<u8>,
    pub objc: Box<dyn Block>,
    pub orgi: BlkOrigin,
}

impl BlockPkg {

	pub fn build(data: Vec<u8>) -> Ret<Self> {
		let (objc, _) = block::create(&data)?;
		Ok(Self {
			orgi: BlkOrigin::UNKNOWN,
            hein: objc.height().to_uint(),
			hash: objc.hash(),
			data,
			objc,
		})
	}

	pub fn into_block(self) -> Box<dyn Block> {
		self.objc
	}

}



pub struct RecentBlockInfo { 
    pub height:  u64,
    pub hash:    Hash,
    pub prev:    Hash,
    pub txs:     u32, /* transaction_count */
    pub miner:   Address,
    pub message: String,
    pub reward:  Amount,
    pub time:    u64,
    pub arrive:  u64,
}


pub fn create_recent_block_info(blk: &dyn BlockRead) -> RecentBlockInfo {
    let coinbase = &blk.transactions()[0];
    RecentBlockInfo {
        height:  blk.height().to_uint(),
        hash:    blk.hash(),
        prev:    blk.prevhash().clone(),
        txs:     blk.transaction_count().to_uint(), // transaction_count
        miner:   coinbase.main(),
        message: coinbase.message().to_readable(),
        reward:  coinbase.reward().clone(),
        time:    blk.timestamp().to_uint(),
        arrive:  curtimes(),
    }
}

