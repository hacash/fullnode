//! Wire schemas for mint-core actions/structs.
//!
//! The `ActionSchemaProvider` of `DiamondMint` (hand-written codec) is
//! registered here; the nested struct schema of `DiamondMintData` is generated
//! by `field::wire_struct_schema!` next to the type in `action::diamond`, and
//! the schemas of `AddrHac`/`AssetSmelt` (field types) live next to the types
//! in `field::schema`. `codec-schema-gen` collects them to generate TS metadata.

use base::{ActionSchema, ActionSchemaProvider, FieldSchema, FieldWire};

impl ActionSchemaProvider for crate::action::diamond::DiamondMint {
    const ACTION_SCHEMA: ActionSchema = ActionSchema {
        kind: Self::KIND,
        name: "diamond_mint",
        audit_class: "full",
        blob: false,
        fields: &[
            FieldSchema::new("kind", FieldWire::U2),
            FieldSchema::new("d", FieldWire::Struct("DiamondMintData")),
        ],
    };
}

/// All nested struct schemas (collected by `codec-schema-gen`; merged with the
/// vm/tex lists). Single registration point: new nested structs are appended
/// here instead of writing the provider lookups by hand.
pub fn struct_schemas() -> Vec<base::StructSchema> {
    field::collect_struct_schemas!(
        field::AddrHac,
        field::AssetSmelt,
        crate::action::diamond::DiamondMintData,
    )
}
