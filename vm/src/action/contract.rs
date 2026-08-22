//! `ContractDeploy` (kind 40) + `ContractUpdate` (kind 41) wire codecs.
//! Execute bodies, store prechecks and `peek_vm_runtime_limits` live in `contract_exec.rs` (`execute` feature only).

use base::ActScope;
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

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
#[action_codec(audit = "structured", code)]
pub struct ContractDeploy {
    pub kind: Uint2,
    pub protocol_cost: Amount,
    pub nonce: Uint4,
    pub construct_argv: BytesW2, // checked by SpaceCap::value_size at runtime
    pub marks: Fixed4,           // zero
    pub contract: ContractSto,
}

impl ContractDeploy {
    pub const KIND: u16 = 40;

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

impl Default for ContractDeploy {
    fn default() -> Self {
        Self::new()
    }
}

base::impl_action_facts! {
    ContractDeploy {
        name: "contract_deploy",
        scope: ActScope::TOP_ONLY_CAN_WITH_GUARD,
        min_tx_type: 3,
        extra9: |_: &ContractDeploy| false,
        req_sign: |_: &ContractDeploy| vec![],
        as_transfer_like: none,
        description: |this: &ContractDeploy| {
            format!("Deploy smart contract with nonce {}", this.nonce.uint())
        },

    }
}

// ================================ ContractUpdate ================================

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
#[action_codec(audit = "structured", code)]
pub struct ContractUpdate {
    pub kind: Uint2,
    pub protocol_cost: Amount,
    pub address: Address, // contract address
    pub marks: Fixed2,    // zero
    pub edit: ContractEdit,
}

impl ContractUpdate {
    pub const KIND: u16 = 41;

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

impl Default for ContractUpdate {
    fn default() -> Self {
        Self::new()
    }
}

base::impl_action_facts! {
    ContractUpdate {
        name: "contract_update",
        scope: ActScope::TOP_ONLY_CAN_WITH_GUARD,
        min_tx_type: 3,
        extra9: |_: &ContractUpdate| false,
        req_sign: |_: &ContractUpdate| vec![],
        as_transfer_like: none,
        description: |this: &ContractUpdate| {
            format!("Update smart contract {}", this.address.to_readable())
        },

    }
}
