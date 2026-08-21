use base::{ActionCodecBinding, StructSchema, WireRegistry};

use crate::action::{
    ContractDeploy, ContractMainCall, ContractUpdate, P2SHScriptProve, create_contract_action,
    create_p2sh_script_prove,
};

/// VM-owned transaction action codecs.
pub const ACTION_CODECS: &[ActionCodecBinding] = &[
    base::action_codec_binding!(ContractDeploy, create_contract_action),
    base::action_codec_binding!(ContractUpdate, create_contract_action),
    base::action_codec_binding!(ContractMainCall, create_contract_action),
    base::action_codec_binding!(P2SHScriptProve, create_p2sh_script_prove),
];

/// Nested structs referenced by VM action schemas.
pub const STRUCT_SCHEMAS: &[StructSchema] = &[
    <crate::contract::ContractMeta as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::contract::ContractAbstCall as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::contract::ContractUserFunc as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::contract::ContractCalcFunc as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::contract::ContractAddrReplaceAt as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::contract::ContractEdit as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::contract::ContractSto as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::rt::CodeStuff as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <crate::rt::FuncArgvTypes as base::StructSchemaProvider>::STRUCT_SCHEMA,
];

/// Installs the VM-owned transaction action codecs into a dynamic wire profile.
pub fn register_wire(reg: &mut dyn WireRegistry) -> sys::Rerr {
    for binding in ACTION_CODECS {
        reg.register_action_codec(*binding)?;
    }
    Ok(())
}
