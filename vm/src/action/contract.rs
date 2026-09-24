//! `ContractDeploy` (kind 40) + `ContractUpdate` (kind 41) wire codecs.
//! Execute bodies, store prechecks and `peek_vm_runtime_limits` live in `contract_exec.rs` (`execute` feature only).

use field::{Address, Amount, BytesW2, Encode, Fixed2, Fixed4, Uint2, Uint4};

use crate::contract::{ContractEdit, ContractSto};
use crate::rt::AbstCall;
use crate::value::ContractAddress;

/// Stable protocol accounting overhead for creating a new contract object.
/// This is a consensus resource unit, rather than a measurement of a specific
/// state backend's physical storage representation.
pub const CONTRACT_DEPLOY_CHARGE_OVERHEAD_BYTES: usize = 64;

/// Billable bytes for a deployment. Updates overwrite the existing contract
/// object and therefore use their edit payload size instead.
pub fn contract_deploy_charge_bytes(contract: &ContractSto) -> usize {
    contract.size() + CONTRACT_DEPLOY_CHARGE_OVERHEAD_BYTES
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractStoreAnalysis {
    pub address: ContractAddress,
    pub contract_size: usize,
    pub inherit_count: usize,
    pub library_count: usize,
    pub has_construct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractUpdateAnalysis {
    pub address: ContractAddress,
    pub old_contract_size: usize,
    pub new_contract_size: usize,
    pub edit_size: usize,
    pub did_structural_change: bool,
    pub did_effective_lookup_change: bool,
    pub update_hook: AbstCall,
    /// Full-price minimum: guaranteed inclusion regardless of discount quota.
    pub required_protocol_cost: Amount,
    /// Best-effort discount minimum under the current head/block budget: fees in
    /// `[discounted_protocol_cost, required_protocol_cost)` are discount-classified
    /// and consume quota. `None` when disabled or when no budget snapshot is in
    /// scope (offline analysis) — pay `required_protocol_cost` then.
    pub discounted_protocol_cost: Option<Amount>,
    /// The discount period basis behind `discounted_protocol_cost`.
    pub discounted_periods: Option<u64>,
}

// ================================ ContractDeploy ================================

// Scope is TOP: a deploy combines with other top actions (minting an asset and
// deploying its contract in one tx is the core case) and repeats at top, which is
// what a factory batching `[Deploy, Deploy, ...]` needs. Uniqueness was rejected
// because it forbids exactly that batch; the per-tx action-count cap and each
// deploy's protocol cost are what bound the batch instead.
#[base::action(kind = 40, tx_min = 3, scope = TOP, audit = "structured", code, ctor = none,
    description = |this: &ContractDeploy| format!("Deploy smart contract with nonce {}", this.nonce.uint()))]
#[derive(PartialEq, Eq)]
pub struct ContractDeploy {
    pub protocol_cost: Amount,
    pub nonce: Uint4,
    pub construct_argv: BytesW2, // checked by SpaceCap::value_size at runtime
    pub marks: Fixed4,           // zero
    pub contract: ContractSto,
}

impl ContractDeploy {
    pub fn new() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            protocol_cost: Amount::zero(),
            nonce: Uint4::from(0),
            construct_argv: BytesW2::default(),
            marks: Fixed4::default(),
            contract: ContractSto::default(),
        }
    }
}

// ================================ ContractUpdate ================================

#[base::action(kind = 41, tx_min = 3, scope = TOP_ONLY_CAN_WITH_GUARD, audit = "structured", code, ctor = none,
    description = |this: &ContractUpdate| format!("Update smart contract {}", this.address.to_readable()))]
#[derive(PartialEq, Eq)]
pub struct ContractUpdate {
    pub protocol_cost: Amount,
    pub address: Address, // contract address
    pub marks: Fixed2,    // zero
    pub edit: ContractEdit,
}

impl ContractUpdate {
    pub fn new() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            protocol_cost: Amount::zero(),
            address: Address::default(),
            marks: Fixed2::default(),
            edit: ContractEdit::default(),
        }
    }
}

#[cfg(test)]
mod scope_tests {
    use base::{ActScope, Action};

    use super::{ContractDeploy, ContractUpdate};

    // The protocol topology tests (`protocol::level`) reproduce these two actions with
    // local stubs, because `protocol` cannot depend on this crate. These assertions are
    // what keeps that reproduction honest: changing a scope here fails loudly next to
    // the declaration instead of quietly invalidating the combinations pinned there.

    #[test]
    fn deploy_is_top_so_a_factory_can_batch_deploys() {
        assert_eq!(ContractDeploy::KIND, 40);
        assert_eq!(ContractDeploy::SCOPE, ActScope::TOP);
        let deploy = ContractDeploy::new();
        assert_eq!(deploy.scope(), ActScope::TOP);
        assert_eq!(deploy.min_tx_type(), 3);
        assert_eq!(deploy.required_flags(), 0);
    }

    #[test]
    fn update_stays_a_top_only_guard_companion() {
        assert_eq!(ContractUpdate::KIND, 41);
        assert_eq!(ContractUpdate::SCOPE, ActScope::TOP_ONLY_CAN_WITH_GUARD);
        let update = ContractUpdate::new();
        assert_eq!(update.scope(), ActScope::TOP_ONLY_CAN_WITH_GUARD);
        assert_eq!(update.min_tx_type(), 3);
    }
}
