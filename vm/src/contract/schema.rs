//! Wire schema for VM contract types that `field::wire_struct_schema!` cannot generate (composites,
//! value-type leaf names). Plain structs get providers from the macro; field order must match `Encode`.

use field::{FieldWire, FieldWireShape, StructSchema, StructSchemaProvider, WireElementName};

impl FieldWireShape for crate::rt::FuncArgvTypes {
    // Composite type (typnum + variable-length body); `ContractUserFunc.pmdf`
    // references it via this `Struct("FuncArgvTypes")` shape.
    const WIRE: FieldWire = FieldWire::Struct("FuncArgvTypes");
}

impl StructSchemaProvider for crate::rt::FuncArgvTypes {
    // Composite type; only the name is registered here to close the reference,
    // field-level expansion is added when TS generates the enum/composite variants.
    const STRUCT_SCHEMA: StructSchema = StructSchema {
        name: "FuncArgvTypes",
        fields: &[],
    };
}

impl WireElementName for crate::value::ContractAddress {
    const NAME: &'static str = "ContractAddress";
}

impl FieldWireShape for crate::value::ContractAddress {
    // Thin wrapper over `field::Address`; its wire shape is a plain address.
    const WIRE: FieldWire = FieldWire::Address;
}
