use transaction::TransactionCoinbase;


impl ChainEngine {

    fn check_all_for_insert(&self, isrt_blk: &BlockPkg, prev_blk: Arc<dyn Block>) -> Rerr {
        
        let cnf = &self.cnf;
        let block = &isrt_blk.objc;
        // check prev hash
        let prev_hx = block.prevhash();
        let base_hx = prev_blk.hash();
        if *prev_hx != base_hx {
            return errf!("need prev hash {} but got {}", base_hx, prev_hx)
        };
        // check time
        let prev_blk_time = prev_blk.timestamp().uint();
        let blk_time = block.timestamp().uint();
        let cur_time = curtimes();
        if blk_time > cur_time {
            return errf!("block timestamp {} cannot more than system timestamp {}", blk_time, cur_time)
        }
        if blk_time <= prev_blk_time {
            return errf!("block timestamp {} cannot less than prev block timestamp {}", blk_time, prev_blk_time)
        }
        // check size
        let blk_size = isrt_blk.data.len();
        if blk_size > cnf.max_block_size + 100 { // may 1MB + headmeta size
            return errf!("block size cannot over {} bytes", cnf.max_block_size + 100)
        }
        // check tx count
        let is_hash_with_fee = true;
        let txhxs = block.transaction_hash_list(is_hash_with_fee); // hash with fee
        let txcount = block.transaction_count().uint() as usize;
        if txcount < 1 {
            return err!("block txs cannot empty, need coinbase tx")
        }
        if txcount > cnf.max_block_txs { // may 1000
            return errf!("block txs cannot more than {}", cnf.max_block_txs)
        }
        if txcount != txhxs.len() {
            return errf!("block tx count need {} but got {}", txhxs.len(), txcount)
        }
        // check tx total size and count
        let alltxs = block.transactions();
        let mut txttsize = 0usize;
        let mut txttnum = 0usize;
        const CBTY: u8 =  TransactionCoinbase::TYPE;
        for tx in alltxs {
            let txty = tx.ty();
            // check only one coinbase at first
            if txttnum == 0 && txty != CBTY { // coinbase check
                return errf!("tx({}) type must be coinbase", txttnum)
            }
            if txttnum >= 1 && txty == CBTY { // must not be coinbase
                return errf!("tx({}) type cannot be coinbase", txttnum)  
            }
            let txsz = tx.size();
            if txsz > cnf.max_tx_size {
                return errf!("tx size cannot more than {} bytes", cnf.max_tx_size);
            }
            // size count
            txttnum += 1;
            txttsize += txsz;
            if txty == CBTY {
                continue // igonre coinbase other check
            }
            let an = tx.action_count().uint() as usize;
            if an != tx.actions().len() {
                return errf!("tx action count not match")
            }
            if an > cnf.max_tx_actions {
                return errf!("tx action count cannot more than {}", cnf.max_tx_actions);
            }
            // check time
            if tx.timestamp().uint() > cur_time {
                return errf!("tx timestamp {} cannot more than now {}", tx.timestamp(), cur_time)
            }
            // verify signature
            tx.as_ref().as_read().verify_signature()?; 
        }
        // check size
        if txttnum != txcount {
            return errf!("block tx count need {} but got {}", txcount, txttnum)        
        }
        if txttsize > cnf.max_block_size { // may 1MB
            return errf!("block txs total size cannot over {} bytes", cnf.max_block_size)
        }
        // check mrkl root
        let mkroot = block::calculate_mrklroot(&txhxs);
        let mrklrt = block.mrklroot();
        if *mrklrt != mkroot {
            return errf!("block mrkl root need {} but got {}", mkroot, mrklrt)
        }
        // check mint consensus & coinbase
        self.minter.consensus(prev_blk.as_read(), block.as_read(), &self.store)?;
        // coinbase tx id = 0, if coinbase error
        self.minter.coinbase(isrt_blk.hein, block.coinbase_transaction()?)?;
        // ok 
        Ok(())

    }


}