//! `chain-codec` — the single assembly of the chain's action/tx codec surface.
//!
//! The full node (`app::standard_registry`), the WASM SDK (`sdk::codec`) and
//! `codec-schema-gen` each used to hand-assemble `protocol + vm + mint-core`
//! (in different call shapes and orders), so a new action-hosting crate had to
//! be wired into three places. This crate owns that assembly once:
//!
//! - `register_standard` installs the full standard action/tx wire codec set
//!   (binary + JSON + schema + friendly family) through the same registration
//!   macros the full node runs; execution-only services are installed by the
//!   full node separately (`protocol::register_exec`, `mint-core`, `vm`);
//! - `struct_schemas` aggregates every nested struct schema of the chain;
//! - `collect_action_schemas` / `collect_action_families` capture the schema
//!   and friendly-family sets through the shared registration entry (same
//!   source as `codec-schema-gen`), so the SDK's codec profile and the
//!   generated TS codec can never drift apart.
//!
//! In the SDK build the protocol/vm/mint-core `execute` features stay off
//! (default-features = false), keeping this crate's graph codec-only; the full
//! node enables `execute` itself and feature unification turns it on.

use base::{ActionSchema, StructSchema, WireRegistry};
use sys::Rerr;

/// Install the standard chain wire codec surface: protocol standard
/// actions/txs, mint-core actions (inscription/channel/asset/diamond mint) and
/// the four VM actions. Execution services (VM assigner, block creator, VM
/// host defs, context creator, protocol profile) are not part of the codec
/// surface and are installed by the full node itself.
pub fn register_standard(reg: &mut dyn WireRegistry) -> Rerr {
    protocol::register_wire(reg)?;
    mint_core::setup::register_wire(reg)?;
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

/// One SDK-facing friendly family: the friendly kind name and the wire kinds
/// that share it (e.g. `hac_transfer` ← transfer_hac_to/from/from_to).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFamily {
    pub friendly: &'static str,
    pub kinds: Vec<u16>,
}

/// Registration capture: the register macros forward `ACTION_SCHEMA` and the
/// friendly family on every `register_action` call; all other registration
/// calls are ignored.
pub struct SchemaCollector {
    pub action_schemas: Vec<ActionSchema>,
    pub families: Vec<ActionFamily>,
}

impl SchemaCollector {
    pub fn new() -> Self {
        Self {
            action_schemas: Vec::new(),
            families: Vec::new(),
        }
    }
}

impl WireRegistry for SchemaCollector {
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
    fn register_action_schema(&mut self, schema: ActionSchema) -> Rerr {
        self.action_schemas.push(schema);
        Ok(())
    }
    fn register_action_family(&mut self, friendly: &'static str, kinds: &[u16]) -> Rerr {
        if self.families.iter().any(|f| f.friendly == friendly) {
            return sys::errf!("friendly family {:?} registered more than once", friendly);
        }
        self.families.push(ActionFamily {
            friendly,
            kinds: kinds.to_vec(),
        });
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

/// Capture the friendly-family set through the same registration entry as the
/// runtime registry (`register_standard`), in the same order. Every family
/// kind must be backed by a registered action schema (the macros emit both
/// from the same kind list, so this holds by construction).
pub fn collect_action_families() -> Vec<ActionFamily> {
    let mut registry = SchemaCollector::new();
    register_standard(&mut registry).expect("chain codec assembly");
    registry.families
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

    /// Every family kind must resolve to a captured action schema, and every
    /// family must cover at least one kind.
    #[test]
    fn captured_families_resolve_to_action_schemas() {
        let actions = collect_action_schemas();
        let families = collect_action_families();
        assert!(!families.is_empty(), "expected friendly families");
        for family in &families {
            assert!(!family.kinds.is_empty(), "empty family {:?}", family.friendly);
            for kind in &family.kinds {
                assert!(
                    actions.iter().any(|a| a.kind == *kind),
                    "family {:?} kind {} has no action schema",
                    family.friendly,
                    kind
                );
            }
        }
    }
}
