//! Node / worker version banner constants (aligned with fullnodedev `app::version`).

#[allow(unused)]
pub const HACASH_NODE_VERSION: &str = "1.1.0";
#[allow(unused)]
pub const HACASH_NODE_BUILD_TIME: &str = "2026/7/26 #1";

/// Persistent chain-state format version (`state_v{N}`). Bump this whenever a
/// state layout or rebuild result changes.
pub const DB_VERSION: u32 = 1;
