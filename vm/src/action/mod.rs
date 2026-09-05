//! VM top-level transaction actions: `ContractDeploy` (40), `ContractUpdate` (41),
//! `ContractMainCall` (44), `P2SHScriptProve` (46); params come from `Registry.vm_params()`.
pub(crate) mod contract;
#[cfg(feature = "execute")]
pub(crate) mod contract_exec;
pub(crate) mod maincall;
#[cfg(feature = "execute")]
pub(crate) mod maincall_exec;
pub(crate) mod p2sh;
#[cfg(feature = "execute")]
pub(crate) mod p2sh_exec;
pub(crate) mod p2sh_tool;

pub use contract::{ContractDeploy, ContractStoreAnalysis, ContractUpdate, ContractUpdateAnalysis};
#[cfg(feature = "execute")]
pub use contract_exec::{
    analyze_contract_store, analyze_contract_update, contract_protocol_cost_min,
    quote_contract_storage_fee, ContractStorageFeeQuote,
};
pub use maincall::ContractMainCall;
pub use p2sh::{P2SHScriptProve, P2shEntryPayload, ScriptmhCalc, UnlockScript};
pub use p2sh_tool::{P2shLeaf, P2shLeafSpec, P2shMerkleTree, P2shTool, P2shTreeCalc};

/// `codeconf` type bits. Authority: `crate::rt::CodeType::TYPE_MASK`.
pub const CODECONF_TYPE_MASK: u8 = crate::rt::CodeType::TYPE_MASK;
/// `codeconf` reserved bits (must be zero). Authority: `crate::rt::CodeConf::RESERVED_MASK`.
pub const CODECONF_RESERVED_MASK: u8 = crate::rt::CodeConf::RESERVED_MASK;
