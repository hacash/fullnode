/// Stable chain identifier shared by config, execution and consensus; a transparent
/// u32 keeps existing numeric config files compatible (INI decoding: hand-written, no serde).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChainId(u32);

impl ChainId {
    pub const MAINNET: Self = Self(0);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn is_mainnet(self) -> bool {
        self.0 == Self::MAINNET.0
    }
}

impl From<u32> for ChainId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<ChainId> for u32 {
    fn from(value: ChainId) -> Self {
        value.get()
    }
}

impl std::fmt::Display for ChainId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Chain engine configuration (`[engine]` section). Decoded hand-written in the
/// app composition layer (no serde); `vm` is populated from `[vm]` at load time.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub data_dir: String,
    pub fast_sync: bool,
    pub unstable_block: u64,
    /// Max side branches the live fork tree and boot replay may retain; over-capacity
    /// subtrees are dropped in deterministic order (side state is rebuildable).
    pub side_tree_capacity: usize,
    pub recent_blocks: bool,
    pub average_fee_purity: bool,
    pub show_miner_name: bool,
    /// VM logging config; populated from the `[vm]` section at load time so the
    /// two sections stay independently configured.
    pub vm: VmConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            data_dir: String::new(),
            fast_sync: false,
            unstable_block: 4,
            side_tree_capacity: 256,
            recent_blocks: true,
            average_fee_purity: true,
            show_miner_name: false,
            vm: VmConfig::default(),
        }
    }
}

impl EngineConfig {
    pub fn is_open_vmlog(&self, height: u64) -> bool {
        self.vm.is_open(height)
    }
}

/// VM logging and log-management configuration (`[vm]` section).
#[derive(Clone, Debug, Default)]
pub struct VmConfig {
    pub log_enable: bool,
    pub log_open_height: u64,
    /// Authorization hash required by the `vm_logs_delete` HTTP API endpoint.
    /// An empty string disables authorization for that endpoint.
    pub log_delete_auth_hash: String,
}

impl VmConfig {
    /// Whether VM execution logging is active at the given block height.
    pub fn is_open(&self, height: u64) -> bool {
        self.log_enable && height >= self.log_open_height
    }
}

#[cfg(test)]
mod tests {
    use super::ChainId;

    #[test]
    fn chain_id_is_a_transparent_u32() {
        let id = ChainId::new(u32::MAX);
        assert_eq!(id.get(), u32::MAX);
        assert_eq!(ChainId::from(u32::MAX), id);
        assert_eq!(u32::from(id), u32::MAX);
        assert!(ChainId::MAINNET.is_mainnet());
        assert!(!id.is_mainnet());
    }
}
