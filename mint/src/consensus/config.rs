use base::ChainId;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MintConf {
    pub chain_id: ChainId,
    /// Genesis consensus configuration, persisted in state after initialization.
    pub diamond_form: bool,
}

impl Default for MintConf {
    fn default() -> Self {
        Self {
            chain_id: ChainId::MAINNET,
            diamond_form: true,
        }
    }
}

impl MintConf {
    pub fn is_mainnet(&self) -> bool {
        self.chain_id.is_mainnet()
    }
}
