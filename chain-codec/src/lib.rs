//! `chain-codec` — the single assembly of the chain's action/tx codec surface.
//!
//! The full node (`app::standard_registry`), the WASM SDK (`sdk::codec`) and
//! `codec-schema-gen` each used to hand-assemble `protocol + vm + mint-core`
//! (in different call shapes and orders), so a new action-hosting crate had to
//! be wired into three places. This crate owns that assembly once:
//!
//! - `register_standard` installs the full standard action/tx codec set (the
//!   same registration macros the full node runs; execution-only hooks are
//!   no-ops on registries that do not implement them);
//! - `struct_schemas` aggregates every nested struct schema of the chain;
//! - `collect_action_schemas` captures the schema set through the shared
//!   registration entry (same source as `codec-schema-gen`), so the SDK's
//!   codec profile and the generated TS codec can never drift apart.
//!
//! Feature unification keeps this crate small in the SDK build (vm
//! `codec-only`/mint-core `codec-only`) while the full node's own `full` /
//! `execute` features take precedence there.

use base::{ActionSchema, RegistryWriter, StructSchema};
use sys::Rerr;

/// Install the standard chain codec surface: protocol standard actions/txs,
/// mint-core actions (inscription/channel/asset/diamond mint) and the four VM
/// actions. Execution services (VM assigner, block creator, ...) are not part
/// of the codec surface and are installed by the full node itself.
pub fn register_standard(reg: &mut dyn RegistryWriter) -> Rerr {
    protocol::register_standard(reg, &protocol::PROTOCOL_PARAMS)?;
    mint_core::setup::register(reg)?;
    vm::action::register_actions(reg)?;
    Ok(())
}

/// All nested struct schemas of the chain (vm contract structs + protocol
/// TexCell + mint-core structs), in one list. Single aggregation point for
/// `codec-schema-gen`, the SDK codec profile and the spec codec registry.
pub fn struct_schemas() -> Vec<StructSchema> {
    let mut v = vm::codec_schema::struct_schemas();
    v.push(protocol::action_std::tex_cell_schema());
    v.extend(mint_core::schema::struct_schemas());
    v
}

/// Registration capture: the register macros forward `ACTION_SCHEMA` on every
/// `register_action`; all other registration calls are ignored.
pub struct SchemaCollector {
    pub action_schemas: Vec<ActionSchema>,
}

impl SchemaCollector {
    pub fn new() -> Self {
        Self {
            action_schemas: Vec::new(),
        }
    }
}

impl RegistryWriter for SchemaCollector {
    fn set_block_creator(&mut self, _f: base::BlockCreateFn) -> Rerr {
        Ok(())
    }
    fn set_block_sizer(&mut self, _f: base::BlockSizeFn) -> Rerr {
        Ok(())
    }
    fn set_vm_assigner(&mut self, _f: base::VmAssignFn) -> Rerr {
        Ok(())
    }
    fn register_tx(&mut self, _ty: u8, _f: base::TxCreateFn) -> Rerr {
        Ok(())
    }
    fn register_tx_json(&mut self, _ty: u8, _f: base::TxJsonDecodeFn) -> Rerr {
        Ok(())
    }
    fn register_action(&mut self, _kinds: &[u16], _f: base::ActionCreateFn) -> Rerr {
        Ok(())
    }
    fn register_action_json(&mut self, _kinds: &[u16], _f: base::ActionJsonDecodeFn) -> Rerr {
        Ok(())
    }
    fn register_vm_host_def(&mut self, _def: base::VmHostActionDef) -> Rerr {
        Ok(())
    }
    fn set_context_creator(&mut self, _f: base::ContextCreateFn, _gas_budget: i64) -> Rerr {
        Ok(())
    }
    fn set_vm_params(&mut self, _params: base::VmExecutionParams) -> Rerr {
        Ok(())
    }
    fn set_execution_profile(&mut self, _profile: &'static (dyn std::any::Any + Send + Sync)) -> Rerr {
        Ok(())
    }
    fn register_action_schema(&mut self, schema: ActionSchema) -> Rerr {
        self.action_schemas.push(schema);
        Ok(())
    }
}

/// Capture the action schema set through the same registration entry as the
/// runtime registry (`register_standard`), in the same order.
pub fn collect_action_schemas() -> Vec<ActionSchema> {
    let mut registry = SchemaCollector::new();
    register_standard(&mut registry).expect("chain codec assembly");
    registry.action_schemas
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The captured schema set must validate: unique kinds/names and a closed
    /// nested-struct reference graph. A new action whose schema references an
    /// unregistered struct fails here, not in the generated artifacts.
    #[test]
    fn captured_schema_set_validates() {
        let actions = collect_action_schemas();
        let structs = struct_schemas();
        base::validate_schema_set(&actions, &structs).expect("chain schema set valid");
        assert!(actions.len() > 30, "expected the full standard action set");
    }
}
