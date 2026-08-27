//! `ContractDeploy` (kind 40) + `ContractUpdate` (kind 41) wire codecs.
//! Execute bodies, store prechecks and `peek_vm_runtime_limits` live in `contract_exec.rs` (`execute` feature only).

use field::{Address, Amount, BytesW2, Fixed2, Fixed4, Uint2, Uint4};

use crate::contract::{ContractEdit, ContractSto};
use crate::rt::AbstCall;
use crate::value::ContractAddress;

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
    pub required_protocol_cost: Amount,
}

// ================================ ContractDeploy ================================

#[base::action(kind = 40, tx_min = 3, scope = TOP_ONLY_CAN_WITH_GUARD, audit = "structured", code, ctor = none,
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
