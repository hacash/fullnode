use base::{ChainId, MintParams};

/// Fixed Hacash mainnet rules. Network identity and genesis form are supplied
/// separately by `MintConf`, because they select the chain a node joins.
pub const MINT_PARAMS: MintParams = MintParams {
    max_block_txs: 1000,
    max_block_size: 1024 * 1024,
    max_tx_size: base::MAX_TX_SIZE,
    difficulty_adjust_blocks: 288,
    difficulty_group_blocks: 4,
    each_block_target_time: 300,
};

pub const fn mint_params_for(_chain_id: ChainId) -> MintParams {
    MINT_PARAMS
}
