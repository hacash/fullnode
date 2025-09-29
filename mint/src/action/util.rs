


/*****************************************/


pub fn pickout_diamond_mint_action(tx: &dyn TransactionRead) -> Option<DiamondMint> {
    if tx.ty() == TransactionCoinbase::TYPE {
        return None // ignore coinbase tx
    }
    let mut res: Option<DiamondMint> = None;
    for a in tx.actions() {
        if a.kind() == DiamondMint::KIND {
            let act = DiamondMint::must(&a.serialize());
            res = Some(act);
            break // find ok
        }
    }
    res
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
    const DMINT: u16 = DiamondMint::KIND;
    for act in tx.actions() {
        if act.kind() == DMINT {
            let dm = DiamondMint::must(&act.serialize());
            return *dm.d.number;
        }
    }
    0
}