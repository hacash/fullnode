use base::ChainId;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MintConf {
    pub chain_id: ChainId,
    /// Genesis consensus configuration, persisted in state after initialization.
    pub diamond_form: bool,
    /// Sync block of max height; `0` means unlimited (config `[mint].height_max`,
    /// dev parity). Blocks above this height are rejected at insert.
    #[serde(rename = "height_max")]
    pub sync_maxh: u64,
}

impl Default for MintConf {
    fn default() -> Self {
        Self {
            chain_id: ChainId::MAINNET,
            diamond_form: true,
            sync_maxh: 0,
        }
    }
}

impl MintConf {
    pub fn is_mainnet(&self) -> bool {
        self.chain_id.is_mainnet()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[mint].height_max` (OLD config key) must map onto `sync_maxh`.
    #[test]
    fn height_max_key_maps_to_sync_maxh() {
        let conf: MintConf = serde_json::from_str(
            r#"{"chain_id":0,"diamond_form":true,"height_max":12345}"#,
        )
        .expect("parse MintConf");
        assert_eq!(conf.sync_maxh, 12345);
    }

    /// Missing key falls back to unlimited (0).
    #[test]
    fn missing_height_max_defaults_to_unlimited() {
        let conf = MintConf::default();
        assert_eq!(conf.sync_maxh, 0);
    }
}
