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

pub use contract::{
    ContractDeploy, ContractStoreAnalysis, ContractUpdate, ContractUpdateAnalysis,
    create_contract_deploy, create_contract_update,
};
#[cfg(feature = "execute")]
pub use contract_exec::{
    analyze_contract_store, analyze_contract_update, contract_protocol_cost_min,
};
pub use maincall::{ContractMainCall, create_contract_main_call};
pub use p2sh::{
    P2SHScriptProve, P2shEntryPayload, ScriptmhCalc, UnlockScript, create_p2sh_script_prove,
};
pub use p2sh_tool::{P2shLeaf, P2shLeafSpec, P2shMerkleTree, P2shTool, P2shTreeCalc};

use base::ActionRef;
use sys::Ret;

/// Decoder dispatch for `ContractDeploy`/`ContractUpdate`/`ContractMainCall`.
/// `P2SHScriptProve` decodes via its own separately-registered `create_p2sh_script_prove`.
pub fn create_contract_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    match kind {
        ContractDeploy::KIND => create_contract_deploy(_reg, kind, buf),
        ContractUpdate::KIND => create_contract_update(_reg, kind, buf),
        ContractMainCall::KIND => create_contract_main_call(_reg, kind, buf),
        _ => sys::normalf!("create_contract_action: unknown kind {}", kind),
    }
}
