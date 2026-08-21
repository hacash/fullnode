use base::ChainId;
use sys::{IniSec, ini_bool, ini_deny_unknown, ini_u32, ini_u64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MintConf {
    pub chain_id: ChainId,
    /// Genesis consensus configuration, persisted in state after initialization.
    pub diamond_form: bool,
    /// Sync block of max height; `0` means unlimited (config `[mint].height_max`,
    /// dev parity). Blocks above this height are rejected at insert.
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

    /// Decode the `[mint]` INI section. `height_max` (old key) maps onto `sync_maxh`;
    /// unknown keys with present values are rejected (serde `deny_unknown_fields` equivalent).
    pub fn from_ini(sec: &IniSec) -> sys::Ret<Self> {
        ini_deny_unknown(sec, "mint", &["chain_id", "diamond_form", "height_max"])?;
        let mut cfg = Self::default();
        cfg.chain_id = ChainId::new(ini_u32(sec, "chain_id", cfg.chain_id.get())?);
        cfg.diamond_form = ini_bool(sec, "diamond_form", cfg.diamond_form)?;
        cfg.sync_maxh = ini_u64(sec, "height_max", cfg.sync_maxh)?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sec(pairs: &[(&str, Option<&str>)]) -> IniSec {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_owned)))
            .collect()
    }

    /// `[mint].height_max` (OLD config key) must map onto `sync_maxh`.
    #[test]
    fn height_max_key_maps_to_sync_maxh() {
        let conf = MintConf::from_ini(&sec(&[
            ("chain_id", Some("0")),
            ("diamond_form", Some("true")),
            ("height_max", Some("12345")),
        ]))
        .expect("decode MintConf");
        assert_eq!(conf.chain_id, ChainId::MAINNET);
        assert!(conf.diamond_form);
        assert_eq!(conf.sync_maxh, 12345);
    }

    /// Missing keys fall back to the defaults.
    #[test]
    fn missing_keys_default() {
        let conf = MintConf::from_ini(&sec(&[])).expect("decode MintConf");
        assert_eq!(conf, MintConf::default());
    }

    /// Unknown keys with present values are rejected; bare empty unknown
    /// keys stay tolerated (the old section iterator skipped them).
    #[test]
    fn unknown_key_rejected() {
        assert!(MintConf::from_ini(&sec(&[("bogus", None)])).is_ok());
        assert!(MintConf::from_ini(&sec(&[("bogus", Some("1"))])).is_err());
    }
}
