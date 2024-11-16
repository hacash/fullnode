

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

	pub fn into_block(self) -> Box<dyn Block> {
		self.objc
	}

}



combi_struct!{ RecentBlockInfo, 
    height:  BlockHeight
    hash:    Hash
    prev:    Hash
    txs:     Uint4 // transaction_count
    miner:   Address
    message: Fixed16
    reward:  Amount
    time:    Timestamp
    arrive:  Timestamp
}


pub fn create_recent_block_info(blk: &dyn BlockRead) -> RecentBlockInfo {
    let coinbase = &blk.transactions()[0];
    RecentBlockInfo {
        height:  blk.height().clone(),
        hash:    blk.hash(),
        prev:    blk.prevhash().clone(),
        txs:     blk.transaction_count().clone(), // transaction_count
        miner:   coinbase.main(),
        message: coinbase.message().clone(),
        reward:  coinbase.reward().clone(),
        time:    blk.timestamp().clone(),
        arrive:  Timestamp::from(curtimes()),
    }
}

