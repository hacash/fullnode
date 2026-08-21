//! Action wire schema — Rust is the single source of truth for action field wire shapes.
//! Holds action/struct-level schema types, provider traits, validation, and deterministic hashing; pure static data shared by fullnode and SDK.

pub use field::schema::{
    ActionSchema, ActionSchemaProvider, AuditClass, FieldSchema, FieldWire, FieldWireShape,
    StructSchema, StructSchemaProvider,
};

/// (name, wire) pairs for every built-in leaf; the single table behind
/// `builtin_leaf_wire`, also rendered into the generated TS codec by SDK codegen.
pub const BUILTIN_LEAVES: &[(&str, FieldWire)] = &[
    ("U1", FieldWire::U8),
    ("Uint1", FieldWire::U8),
    ("U8", FieldWire::U8),
    ("U2", FieldWire::U2),
    ("Uint2", FieldWire::U2),
    ("U4", FieldWire::U4),
    ("Uint4", FieldWire::U4),
    ("U5", FieldWire::U5),
    ("Uint5", FieldWire::U5),
    ("BlockHeight", FieldWire::U5),
    ("Amount", FieldWire::Amount),
    ("WireAmount", FieldWire::WireAmount),
    ("Address", FieldWire::Address),
    ("ContractAddress", FieldWire::Address),
    ("AddrOrPtr", FieldWire::AddrOrPtr),
    ("AddrOrList", FieldWire::AddrOrList),
    ("BytesW1", FieldWire::BytesW1),
    ("BytesW2", FieldWire::BytesW2),
    ("Satoshi", FieldWire::Satoshi),
    ("Fold64", FieldWire::Fold64),
    ("Timestamp", FieldWire::Timestamp),
    ("DiamondName", FieldWire::DiamondName),
    ("DiamondNumber", FieldWire::DiamondNumber),
    ("AssetAmt", FieldWire::AssetAmt),
    ("Sign", FieldWire::Fixed(97)),
    ("Hash", FieldWire::Fixed(32)),
    ("PosiHash", FieldWire::Fixed(33)),
];

/// Built-in leaf element name → wire shape (`ListW1/ListW2` element references).
/// List elements may only reference a registered struct or the built-in leaves.
pub fn builtin_leaf_wire(name: &str) -> Option<FieldWire> {
    BUILTIN_LEAVES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, wire)| wire.clone())
}

/// Validate a schema set: unique kinds, unique names, complete closure over
/// nested references, and valid audit classes. `Err` describes the first violation.
pub fn validate_schema_set(
    schemas: &[ActionSchema],
    struct_schemas: &[StructSchema],
) -> Result<(), String> {
    use std::collections::HashSet;
    let mut kinds = HashSet::new();
    let mut names = HashSet::new();
    for schema in schemas {
        validate_field_names(schema.name, schema.fields)?;
        if !kinds.insert(schema.kind) {
            return Err(format!(
                "duplicate action kind {} ({} and another schema)",
                schema.kind, schema.name
            ));
        }
        if !names.insert(schema.name) {
            return Err(format!(
                "duplicate action name {} (kind {})",
                schema.name, schema.kind
            ));
        }
    }
    for struct_schema in struct_schemas {
        validate_field_names(struct_schema.name, struct_schema.fields)?;
        if !names.insert(struct_schema.name) {
            return Err(format!(
                "duplicate nested struct name {}",
                struct_schema.name
            ));
        }
    }
    // Nested closure: every name referenced by Struct(name) must exist in the set.
    let known = names;
    let all_schemas: Vec<ActionSchema> = schemas
        .iter()
        .cloned()
        .chain(struct_schemas.iter().map(|s| ActionSchema {
            kind: 0,
            name: s.name,
            audit_class: AuditClass::Full,
            blob: false,
            fields: s.fields,
        }))
        .collect();
    for schema in &all_schemas {
        for field in schema.fields {
            collect_struct_refs(&field.wire, &known).map_err(|missing| {
                format!(
                    "schema {} field {} references unknown nested struct {}",
                    schema.name, field.name, missing
                )
            })?;
        }
    }
    Ok(())
}

/// Field names are part of the wire contract: duplicates make generated object codecs
/// ambiguous (later value silently wins), so reject before any registry consumes the schema.
fn validate_field_names(name: &str, fields: &[FieldSchema]) -> Result<(), String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for field in fields {
        if !seen.insert(field.name) {
            return Err(format!(
                "schema {} has duplicate field {}",
                name, field.name
            ));
        }
    }
    Ok(())
}

/// Recursively collect `Struct(name)` references in a wire shape, returning
/// Err if any name is missing.
fn collect_struct_refs(
    wire: &FieldWire,
    known: &std::collections::HashSet<&str>,
) -> Result<(), String> {
    match wire {
        FieldWire::Struct(name) if !known.contains(name) => Err(name.to_string()),
        FieldWire::Struct(_) => Ok(()),
        FieldWire::ListW1(name) | FieldWire::ListW2(name)
            if !known.contains(name) && builtin_leaf_wire(name).is_none() =>
        {
            Err(name.to_string())
        }
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => Ok(()),
        _ => Ok(()),
    }
}

/// Deterministic sha3-256 hash of a schema set — a codec version fingerprint: any
/// field/enum order change alters it, so SDK codegen / TS detect drifted artifacts.
pub fn schema_set_hash(schemas: &[ActionSchema], struct_schemas: &[StructSchema]) -> [u8; 32] {
    use sha3::{Digest, Sha3_256};
    // Registration order is a composition detail, not wire contract: field order per schema
    // is wire-significant, so canonicalize the set before hashing (stable SDK profile).
    let mut schemas: Vec<&ActionSchema> = schemas.iter().collect();
    schemas.sort_by_key(|schema| (schema.kind, schema.name));
    let mut struct_schemas: Vec<&StructSchema> = struct_schemas.iter().collect();
    struct_schemas.sort_by_key(|schema| schema.name);
    let mut hasher = Sha3_256::new();
    for schema in schemas {
        hasher.update(&[(schema.kind >> 8) as u8, schema.kind as u8]);
        hasher.update(schema.name.as_bytes());
        hasher.update(&[0]);
        for field in schema.fields {
            hasher.update(field.name.as_bytes());
            hasher.update(&[0]);
            write_wire_hash(&mut hasher, &field.wire);
            hasher.update(&[field.optional as u8]);
        }
        // Review facts are part of the codec identity: a grading or blob-class
        // change rotates the SDK profile like any other codec change.
        hasher.update(schema.audit_class.as_str().as_bytes());
        hasher.update(&[0]);
        hasher.update(&[schema.blob as u8]);
        hasher.update([0xff]);
    }
    for struct_schema in struct_schemas {
        hasher.update(struct_schema.name.as_bytes());
        hasher.update(&[0]);
        for field in struct_schema.fields {
            hasher.update(field.name.as_bytes());
            hasher.update(&[0]);
            write_wire_hash(&mut hasher, &field.wire);
            hasher.update(&[field.optional as u8]);
        }
        hasher.update([0xfe]);
    }
    hasher.finalize().into()
}

fn write_wire_hash<D: sha3::digest::Update>(hasher: &mut D, wire: &FieldWire) {
    match wire {
        FieldWire::U1 => hasher.update(&[1]),
        FieldWire::U2 => hasher.update(&[2]),
        FieldWire::U4 => hasher.update(&[3]),
        FieldWire::U5 => hasher.update(&[4]),
        FieldWire::Fixed(n) => hasher.update(&[5, *n]),
        FieldWire::Amount => hasher.update(&[6]),
        FieldWire::WireAmount => hasher.update(&[7]),
        FieldWire::Address => hasher.update(&[8]),
        FieldWire::AddrOrPtr => hasher.update(&[9]),
        FieldWire::AddrOrList => hasher.update(&[10]),
        FieldWire::BytesW1 => hasher.update(&[11]),
        FieldWire::BytesW2 => hasher.update(&[12]),
        FieldWire::Satoshi => hasher.update(&[13]),
        FieldWire::Fold64 => hasher.update(&[14]),
        FieldWire::Timestamp => hasher.update(&[15]),
        FieldWire::DiamondName => hasher.update(&[16]),
        FieldWire::DiamondNumber => hasher.update(&[17]),
        FieldWire::DiamondNameList => hasher.update(&[18]),
        FieldWire::AssetAmt => hasher.update(&[19]),
        FieldWire::AssetAmtW1 => hasher.update(&[20]),
        FieldWire::ChainIDList => hasher.update(&[21]),
        FieldWire::ContractAddrListW1 => hasher.update(&[22]),
        FieldWire::SignW2 => hasher.update(&[23]),
        FieldWire::ListW1(name) => {
            hasher.update(&[24]);
            hasher.update(name.as_bytes());
            hasher.update(&[0]);
        }
        FieldWire::ListW2(name) => {
            hasher.update(&[25]);
            hasher.update(name.as_bytes());
            hasher.update(&[0]);
        }
        FieldWire::Struct(name) => {
            hasher.update(&[26]);
            hasher.update(name.as_bytes());
            hasher.update(&[0]);
        }
        FieldWire::ActionList => hasher.update(&[27]),
        FieldWire::ActionListW1 => hasher.update(&[28]),
        FieldWire::U8 => hasher.update(&[29]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_FIELDS: &[FieldSchema] = &[FieldSchema {
        name: "value",
        wire: FieldWire::U2,
        optional: false,
    }];
    const B_FIELDS: &[FieldSchema] = &[FieldSchema {
        name: "value",
        wire: FieldWire::U4,
        optional: true,
    }];

    #[test]
    fn schema_hash_ignores_registration_order() {
        let actions_a = [
            ActionSchema {
                kind: 20,
                name: "b",
                audit_class: AuditClass::Full,
                blob: false,
                fields: B_FIELDS,
            },
            ActionSchema {
                kind: 10,
                name: "a",
                audit_class: AuditClass::Opaque,
                blob: true,
                fields: A_FIELDS,
            },
        ];
        let actions_b = [actions_a[1].clone(), actions_a[0].clone()];
        let structs_a = [
            StructSchema {
                name: "z",
                fields: B_FIELDS,
            },
            StructSchema {
                name: "a",
                fields: A_FIELDS,
            },
        ];
        let structs_b = [structs_a[1].clone(), structs_a[0].clone()];
        assert_eq!(
            schema_set_hash(&actions_a, &structs_a),
            schema_set_hash(&actions_b, &structs_b)
        );
    }

    #[test]
    fn schema_validation_rejects_duplicate_field_names() {
        const DUP_FIELDS: &[FieldSchema] = &[
            FieldSchema::new("value", FieldWire::U1),
            FieldSchema::new("value", FieldWire::U2),
        ];
        let actions = [ActionSchema {
            kind: 1,
            name: "duplicate_fields",
            audit_class: AuditClass::Full,
            blob: false,
            fields: DUP_FIELDS,
        }];
        let error = validate_schema_set(&actions, &[]).unwrap_err();
        assert!(error.contains("duplicate field value"), "{error}");
    }
}
