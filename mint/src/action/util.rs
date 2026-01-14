


/*****************************************/


pub fn pickout_diamond_mint_action(tx: &dyn TransactionRead) -> Option<DiamondMint> {
    if tx.ty() == TransactionCoinbase::TYPE {
        return None // ignore coinbase tx
    }
    for act in tx.actions() {
        if let Some(dm) = DiamondMint::downcast(act) {
            return Some(dm.clone());
        }
    }
    None
}


pub fn pickout_diamond_mint_action_from_block(blk: &dyn BlockRead) -> Option<(usize, Box<dyn Transaction>, DiamondMint)> {
    let mut txposi: usize = 0;
    for tx in blk.transactions() {
        if let Some(act) = pickout_diamond_mint_action(tx.as_read()) {
            return Some((txposi, tx.clone(), act))
        }
        txposi += 1;
    }
    None
}


// for diamond create action
pub fn get_diamond_mint_number(tx: &dyn TransactionRead) -> u32 {
    for act in tx.actions() {
        if let Some(dm) = DiamondMint::downcast(act) {
            return *dm.d.number;
        }
    }
    0
}