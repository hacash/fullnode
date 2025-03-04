
fn verify_coinbase(height: u64, cbtx: &dyn TransactionRead) -> Rerr {
    let goot = cbtx.reward();
    let need = genesis::block_reward(height);
    if need != *goot {
        // check fail
        return errf!("block coinbase reward need {} but got {}", need, goot)
    }
    // ok    
    Ok(())
}
