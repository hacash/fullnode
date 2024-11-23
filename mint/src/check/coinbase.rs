
fn check_coinbase(height: u64, cbtx: &dyn Transaction) -> Rerr {
    let goot = cbtx.reward();
    let need = block_reward(height);
    if need == *goot {
        return Ok(())
    }
    // check fail
    errf!("block coinbase reward need {} but got {}", need, goot)
}

