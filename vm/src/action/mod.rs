//! VM top-level transaction actions.
//!
//! Implements `base::Action` for the four consensus-relevant VM entry actions:
//! - `ContractDeploy`   (kind 40)
//! - `ContractUpdate`   (kind 41)
//! - `ContractMainCall` (kind 44)
//! - `P2SHScriptProve`  (kind 46)
//!
//! These are top-level transaction actions (exactly like `HacToTrs` / `DiaInscEdit`),
//! NOT VM-internal opcodes. The VM-internal host calls (`env`/`view`/`EXTACTION`) were
//! already migrated as bytecode opcodes elsewhere.
//!
//! # Params
//! Hacash VM execution params are read from `Registry.vm_params()`
//! (selected and injected by the application). Fee floors use
//! `VmExecutionParams::effective_fee_purity`.
//!
//! # Registration
//! `crate::setup::register` calls `register_actions` to install all four action codecs.

pub(crate) mod contract;
pub(crate) mod maincall;
pub(crate) mod p2sh;
pub(crate) mod p2sh_tool;

pub use contract::{ContractDeploy, ContractStoreAnalysis, ContractUpdate, ContractUpdateAnalysis};
#[cfg(feature = "full")]
pub use contract::{analyze_contract_store, analyze_contract_update, contract_protocol_cost_min};
pub use maincall::ContractMainCall;
pub use p2sh::{P2SHScriptProve, P2shEntryPayload, ScriptmhCalc, UnlockScript};
pub use p2sh_tool::{P2shLeaf, P2shLeafSpec, P2shMerkleTree, P2shTool, P2shTreeCalc};

use base::{ActionRef, RegistryWriter};
use sys::Ret;

use contract::{create_contract_deploy, create_contract_update};
use maincall::create_contract_main_call;
use p2sh::create_p2sh_script_prove;

/// Register all four VM action codecs. Mirrors `mint::setup::register`'s
/// `register_action` pattern. Idempotent per-kind (Registry rejects duplicate kinds).
pub fn register_actions(reg: &mut dyn RegistryWriter) -> Ret<()> {
    base::register_regular_actions!(
        reg,
        create_contract_action => [ContractDeploy, ContractUpdate, ContractMainCall],
        create_p2sh_script_prove => [P2SHScriptProve],
    )?;
    Ok(())
}

/// Decoder dispatch for `ContractDeploy`/`ContractUpdate`/`ContractMainCall`.
/// `P2SHScriptProve` uses its own `create_p2sh_script_prove` (registered separately)
/// because it shares no decode path with the contract family.
fn create_contract_action(
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
