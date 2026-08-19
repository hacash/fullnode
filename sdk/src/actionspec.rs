//! Friendly `ActionSpec` ↔ wire action mapping — single source of truth.
//!
//! One declarative list (`actionspec_table!`) drives:
//! - the wire → friendly decoder (`map_action_spec`, generated as a match over
//!   the same table the TS adapter reads),
//! - the friendly → wire JS adapter (`adaptActionSpec` in the generated
//!   `sdk/js/generated/actionspec.mjs`, emitted by `sdk_codegen` from
//!   `ACTION_SPECS`),
//! - golden vectors and the schema-validation tests.
//!
//! Each field carries both directions explicitly: how the wire value is read
//! (`RustConv`) and how the JS adapter writes it back (`JsConv`). The
//! conversions are the semantic primitives (hex ↔ readable, nested struct
//! flattening/nesting, defaults); everything else is table data. `sdk_codegen`
//! (src/bin/sdk_codegen.rs) regenerates the JS side; a test verifies the
//! checked-in artifact matches.
//!
//! The table is a *friendly overlay*, never a filter: kinds not in it decode
//! to `ActionSpec::RawAction` (wire-shaped fields) and build through the
//! generic schema-driven path in `build.rs`. The SDK exposes every action kind
//! the codec schema registry knows; what the chain can decode is decided by
//! the protocol registry, not here.

use crate::build::ActionSpec;
use crate::error::SdkError;
use crate::spec_codec::WireValue;

// ================================ table data types ================================

/// One wire action kind → friendly `ActionSpec` mapping.
#[derive(Debug, Clone, Copy)]
pub struct ActionSpecDef {
    /// Wire action name (schema `name`, e.g. `transfer_hac_to`).
    pub kind: &'static str,
    /// Friendly SDK kind the JS facade accepts (e.g. `hac_transfer`);
    /// identical to `kind` when there is no separate friendly form.
    pub friendly: &'static str,
    /// Friendly `ActionSpec` variant name (Rust enum / TS union tag).
    pub variant: &'static str,
    pub fields: &'static [FieldDef],
}

#[derive(Debug, Clone, Copy)]
pub struct FieldDef {
    /// Friendly field name (both the Rust `ActionSpec` field and the JS input).
    pub friendly: &'static str,
    pub rust: RustConv,
    pub js: JsConv,
}

/// How a field is read from the wire (Rust decode direction).
#[derive(Debug, Clone, Copy)]
pub enum RustConv {
    /// No value on the wire; `None`.
    ConstNone,
    /// No value on the wire; empty string.
    ConstEmpty,
    /// Plain string field.
    Str(&'static str),
    /// Optional string field (`Some(field_str(...)?)`).
    SomeStr(&'static str),
    /// Numeric field.
    Num(&'static str),
    /// Optional field (0 on the wire → `None`); the type marker is for
    /// the generated TS union (`"str"`/`"num"`).
    Opt(&'static str, &'static str),
    /// String list.
    StrList(&'static str),
    /// Numeric list.
    NumList(&'static str),
    /// Diamond name by explicit wire field (`diamond_field_readable`).
    Dia(&'static str),
    /// Diamond name field `diamond` (`diamond_name_readable`).
    DiaName,
    /// Diamond name list field `diamonds` (`diamond_names_readable`).
    DiaList,
    /// Single-diamond form: one readable name (`transfer_hacd_single_to`).
    DiaSingle,
    /// Numeric field inside the nested `asset` struct.
    AssetNum(&'static str),
    /// String field inside the nested `asset` struct.
    AssetStr(&'static str),
    /// String field inside a nested struct.
    StructStr(&'static str, &'static str),
    /// Optional string field inside a nested struct (absent → `None`).
    StructOptStr(&'static str, &'static str),
    /// Readable (hex-decoded) field inside a nested struct.
    StructReadable(&'static str, &'static str),
}

/// How the JS adapter writes a friendly field back onto the wire shape.
/// `w` is always the wire field name; defaults are JS literals.
#[derive(Debug, Clone, Copy)]
pub enum JsConv {
    /// No conversion, field not touched.
    Noop,
    /// Pass the friendly value through (renamed to `w` when different).
    Rename(&'static str),
    /// Pass through with a default (`a.w = a.friendly ?? d`).
    RenameDef(&'static str, &'static str),
    /// Pass through with a numeric default (`a.w = a.friendly ?? d`).
    RenameDefNum(&'static str, &'static str),
    /// `String(v ?? d)`.
    ToString(&'static str, &'static str),
    /// `(v ?? []).map(Number)`.
    NumList(&'static str),
    /// `toHex(v)` (readable diamond name → hex).
    Hex(&'static str),
    /// `(v ?? []).map(toHex)`.
    HexList(&'static str),
    /// `toHex(v[0])` (single-diamond form).
    HexSingle(&'static str),
    /// `toHexOrKeep(v ?? d)` (hex or readable text).
    HexOrKeep(&'static str, &'static str),
    /// `(v ?? d).replace(/^0x/, "")`.
    Strip0x(&'static str, &'static str),
    /// The field becomes `struct.sub` on the wire (JS sub-value conversion).
    StructField(&'static str, &'static str, JsSubConv),
}

#[derive(Debug, Clone, Copy)]
pub enum JsSubConv {
    Keep(&'static str),
    ToString(&'static str),
    HexOrKeep(&'static str),
    Hex,
}

// ================================ table + generated decoder ================================

macro_rules! rust_conv {
    (none()) => { RustConv::ConstNone };
    (empty()) => { RustConv::ConstEmpty };
    (str($w:literal)) => { RustConv::Str($w) };
    (some_str($w:literal)) => { RustConv::SomeStr($w) };
    (num($w:literal)) => { RustConv::Num($w) };
    (opt($w:literal, $ty:literal)) => { RustConv::Opt($w, $ty) };
    (str_list($w:literal)) => { RustConv::StrList($w) };
    (num_list($w:literal)) => { RustConv::NumList($w) };
    (dia($w:literal)) => { RustConv::Dia($w) };
    (dia_name()) => { RustConv::DiaName };
    (dia_list()) => { RustConv::DiaList };
    (dia_single()) => { RustConv::DiaSingle };
    (asset_num($w:literal)) => { RustConv::AssetNum($w) };
    (asset_str($w:literal)) => { RustConv::AssetStr($w) };
    (struct_str($s:literal, $w:literal)) => { RustConv::StructStr($s, $w) };
    (struct_opt_str($s:literal, $w:literal)) => { RustConv::StructOptStr($s, $w) };
    (struct_readable($s:literal, $w:literal)) => { RustConv::StructReadable($s, $w) };
}

macro_rules! js_conv {
    (noop()) => { JsConv::Noop };
    (rename($w:literal)) => { JsConv::Rename($w) };
    (rename_def($w:literal, $d:literal)) => { JsConv::RenameDef($w, $d) };
    (rename_def_num($w:literal, $d:literal)) => { JsConv::RenameDefNum($w, $d) };
    (to_str($w:literal, $d:literal)) => { JsConv::ToString($w, $d) };
    (num_list($w:literal)) => { JsConv::NumList($w) };
    (hex($w:literal)) => { JsConv::Hex($w) };
    (hex_list($w:literal)) => { JsConv::HexList($w) };
    (hex_single($w:literal)) => { JsConv::HexSingle($w) };
    (hex_or_keep($w:literal, $d:literal)) => { JsConv::HexOrKeep($w, $d) };
    (strip0x($w:literal, $d:literal)) => { JsConv::Strip0x($w, $d) };
    (struct_field($s:literal, $sub:literal, keep($d:literal))) => { JsConv::StructField($s, $sub, JsSubConv::Keep($d)) };
    (struct_field($s:literal, $sub:literal, to_str($d:literal))) => { JsConv::StructField($s, $sub, JsSubConv::ToString($d)) };
    (struct_field($s:literal, $sub:literal, hex_or_keep($d:literal))) => { JsConv::StructField($s, $sub, JsSubConv::HexOrKeep($d)) };
    (struct_field($s:literal, $sub:literal, hex())) => { JsConv::StructField($s, $sub, JsSubConv::Hex) };
}

/// Wire-value extraction expression (Rust decode direction). `fields` is the
/// parameter of the generated `map_action_spec`.
macro_rules! rust_expr {
    ($fields:ident, none()) => { None };
    ($fields:ident, empty()) => { String::new() };
    ($fields:ident, str($w:literal)) => { crate::spec_codec::field_str(&$fields, $w)? };
    ($fields:ident, some_str($w:literal)) => { Some(crate::spec_codec::field_str(&$fields, $w)?) };
    ($fields:ident, num($w:literal)) => { crate::spec_codec::field_num(&$fields, $w)? };
    ($fields:ident, opt($w:literal, $ty:literal)) => { crate::spec_codec::field_opt(&$fields, $w)? };
    ($fields:ident, str_list($w:literal)) => { crate::spec_codec::field_str_list(&$fields, $w)? };
    ($fields:ident, num_list($w:literal)) => { crate::spec_codec::field_num_list(&$fields, $w)? };
    ($fields:ident, dia($w:literal)) => { crate::spec_codec::diamond_field_readable(&$fields, $w)? };
    ($fields:ident, dia_name()) => { crate::spec_codec::diamond_name_readable(&$fields)? };
    ($fields:ident, dia_list()) => { crate::spec_codec::diamond_names_readable(&$fields)? };
    ($fields:ident, dia_single()) => { vec![crate::spec_codec::diamond_name_readable(&$fields)?] };
    ($fields:ident, asset_num($w:literal)) => { crate::spec_codec::field_num(crate::spec_codec::asset_fields(&$fields)?, $w)? };
    ($fields:ident, asset_str($w:literal)) => { crate::spec_codec::field_str(crate::spec_codec::asset_fields(&$fields)?, $w)? };
    ($fields:ident, struct_str($s:literal, $w:literal)) => { crate::spec_codec::struct_field_str(crate::spec_codec::fields_struct(&$fields, $s)?, $w)? };
    ($fields:ident, struct_opt_str($s:literal, $w:literal)) => { crate::spec_codec::struct_field_opt_str(crate::spec_codec::fields_struct(&$fields, $s)?, $w)? };
    ($fields:ident, struct_readable($s:literal, $w:literal)) => { crate::spec_codec::struct_field_readable(crate::spec_codec::fields_struct(&$fields, $s)?, $w)? };
}

/// Declares the friendly action spec table and generates the wire → friendly
/// decoder (`map_action_spec`) from it. The `ACTION_SPECS` const is read by
/// `sdk_codegen` to emit the JS adapter and by the golden-vector/validation
/// tests.
macro_rules! actionspec_table {
    ($fields:ident, $(($kind:literal, $friendly:literal, $variant:ident {
        $($field:ident : $rc:ident($($rargs:tt)*) | $jc:ident($($jargs:tt)*)),+ $(,)?
    })),+ $(,)?) => {
        /// Declarative friendly↔wire mapping (single source; see module docs).
        pub const ACTION_SPECS: &[ActionSpecDef] = &[
            $(ActionSpecDef {
                kind: $kind,
                friendly: $friendly,
                variant: stringify!($variant),
                fields: &[
                    $(FieldDef {
                        friendly: stringify!($field),
                        rust: rust_conv!($rc($($rargs)*)),
                        js: js_conv!($jc($($jargs)*)),
                    }),+
                ],
            }),+
        ];

        /// Wire action fields → friendly `ActionSpec` (design A strings stay
        /// strings). Generated from `ACTION_SPECS` by the macro above: kinds
        /// in the table map to their typed variant, every other registered
        /// kind falls back to `RawAction` (wire-shaped) so the full protocol
        /// action surface is never filtered out by the SDK.
        pub(crate) fn map_action_spec(
            name: &str,
            $fields: Vec<(String, WireValue)>,
        ) -> Result<ActionSpec, SdkError> {
            use ActionSpec::*;
            Ok(match name {
                $($kind => $variant {
                    $($field: rust_expr!($fields, $rc($($rargs)*))),+
                },)+
                other => RawAction {
                    kind: other.to_owned(),
                    fields: $fields,
                },
            })
        }
    };
}

actionspec_table! {
    fields,
    ("transfer_hac_to", "hac_transfer", HacTransfer {
        from: none() | noop(),
        to: str("to") | rename("to"),
        amount: str("hacash") | rename("hacash"),
    }),
    ("transfer_hac_from", "hac_transfer", HacTransfer {
        from: some_str("from") | noop(),
        to: empty() | noop(),
        amount: str("hacash") | rename("hacash"),
    }),
    ("transfer_hac_from_to", "hac_transfer", HacTransfer {
        from: some_str("from") | noop(),
        to: str("to") | rename("to"),
        amount: str("hacash") | rename("hacash"),
    }),
    ("transfer_sat_to", "sat_transfer", SatTransfer {
        from: none() | noop(),
        to: str("to") | rename("to"),
        satoshi: num("satoshi") | rename("satoshi"),
    }),
    ("transfer_sat_from", "sat_transfer", SatTransfer {
        from: some_str("from") | noop(),
        to: empty() | noop(),
        satoshi: num("satoshi") | rename("satoshi"),
    }),
    ("transfer_sat_from_to", "sat_transfer", SatTransfer {
        from: some_str("from") | noop(),
        to: str("to") | rename("to"),
        satoshi: num("satoshi") | rename("satoshi"),
    }),
    ("transfer_hacd_single_to", "hacd_transfer", HacdTransfer {
        from: none() | noop(),
        to: str("to") | rename("to"),
        names: dia_single() | hex_single("diamond"),
    }),
    ("transfer_hacd_to", "hacd_transfer", HacdTransfer {
        from: none() | noop(),
        to: str("to") | rename("to"),
        names: dia_list() | hex_list("diamonds"),
    }),
    ("transfer_hacd_from_to", "hacd_transfer", HacdTransfer {
        from: some_str("from") | noop(),
        to: str("to") | rename("to"),
        names: dia_list() | hex_list("diamonds"),
    }),
    ("transfer_hacd_from", "hacd_transfer", HacdTransfer {
        from: some_str("from") | noop(),
        to: empty() | noop(),
        names: dia_list() | hex_list("diamonds"),
    }),
    ("transfer_asset_to", "asset_transfer", AssetTransfer {
        from: none() | noop(),
        to: str("to") | rename("to"),
        serial: asset_num("serial") | struct_field("asset", "serial", to_str("")),
        amount: asset_str("amount") | struct_field("asset", "amount", to_str("")),
    }),
    ("transfer_asset_from", "asset_transfer", AssetTransfer {
        from: some_str("from") | noop(),
        to: empty() | noop(),
        serial: asset_num("serial") | struct_field("asset", "serial", to_str("")),
        amount: asset_str("amount") | struct_field("asset", "amount", to_str("")),
    }),
    ("transfer_asset_from_to", "asset_transfer", AssetTransfer {
        from: some_str("from") | noop(),
        to: str("to") | rename("to"),
        serial: asset_num("serial") | struct_field("asset", "serial", to_str("")),
        amount: asset_str("amount") | struct_field("asset", "amount", to_str("")),
    }),
    ("height_scope", "height_scope", HeightScope {
        start: num("start") | rename("start"),
        end: num("end") | rename("end"),
    }),
    ("chain_allow", "chain_allow", ChainAllow {
        chains: num_list("chains") | num_list("chains"),
    }),
    ("req_sign_list", "req_sign_list", ReqSignList {
        signers: str_list("signers") | rename("signers"),
    }),
    ("tx_message", "tx_message", TxMessage {
        data: str("data") | rename("data"),
    }),
    ("tx_blob", "tx_blob", TxBlob {
        data: str("data") | rename("data"),
    }),
    ("hacd_insc_push", "insc_push", InscPush {
        diamonds: dia_list() | hex_list("diamonds"),
        protocol_cost: opt("protocol_cost", "str") | rename_def("protocol_cost", "0"),
        engraved_type: opt("engraved_type", "num") | rename_def_num("engraved_type", "0"),
        engraved_content: str("engraved_content") | hex_or_keep("engraved_content", ""),
    }),
    ("hacd_insc_clean", "insc_clean", InscClean {
        diamonds: dia_list() | hex_list("diamonds"),
        protocol_cost: opt("protocol_cost", "str") | rename_def("protocol_cost", "0"),
    }),
    ("hacd_insc_edit", "insc_edit", InscEdit {
        diamond: dia("diamond") | hex("diamond"),
        index: num("index") | rename("index"),
        protocol_cost: opt("protocol_cost", "str") | rename_def("protocol_cost", "0"),
        engraved_type: opt("engraved_type", "num") | rename_def_num("engraved_type", "0"),
        engraved_content: str("engraved_content") | hex_or_keep("engraved_content", ""),
    }),
    ("hacd_insc_move", "insc_move", InscMove {
        from_diamond: dia("from_diamond") | hex("from_diamond"),
        to_diamond: dia("to_diamond") | hex("to_diamond"),
        index: num("index") | rename("index"),
        protocol_cost: opt("protocol_cost", "str") | rename_def("protocol_cost", "0"),
    }),
    ("hacd_insc_drop", "insc_drop", InscDrop {
        diamond: dia("diamond") | hex("diamond"),
        index: num("index") | rename("index"),
        protocol_cost: opt("protocol_cost", "str") | rename_def("protocol_cost", "0"),
    }),
    ("channel_open", "channel_open", ChannelOpen {
        channel_id: str("channel_id") | strip0x("channel_id", ""),
        left_address: struct_str("left_bill", "address") | struct_field("left_bill", "address", keep("")),
        // The native amount is always present on the wire; "0" is the valid
        // zero-amount default ("" is not a parseable amount).
        left_amount: struct_str("left_bill", "amount") | struct_field("left_bill", "amount", to_str("0")),
        right_address: struct_str("right_bill", "address") | struct_field("right_bill", "address", keep("")),
        right_amount: struct_str("right_bill", "amount") | struct_field("right_bill", "amount", to_str("0")),
    }),
    ("channel_close", "channel_close", ChannelClose {
        channel_id: str("channel_id") | strip0x("channel_id", ""),
    }),
    ("asset_create", "asset_create", AssetCreate {
        serial: struct_str("metadata", "serial") | struct_field("metadata", "serial", to_str("")),
        supply: struct_str("metadata", "supply") | struct_field("metadata", "supply", to_str("")),
        decimal: struct_str("metadata", "decimal") | struct_field("metadata", "decimal", keep("0")),
        issuer: struct_str("metadata", "issuer") | struct_field("metadata", "issuer", keep("")),
        ticket: struct_str("metadata", "ticket") | struct_field("metadata", "ticket", hex_or_keep("")),
        name: struct_str("metadata", "name") | struct_field("metadata", "name", hex_or_keep("")),
        protocol_cost: str("protocol_cost") | rename_def("protocol_cost", "0"),
    }),
    ("diamond_mint", "diamond_mint", DiamondMint {
        diamond: struct_readable("d", "diamond") | struct_field("d", "diamond", hex()),
        number: struct_str("d", "number") | struct_field("d", "number", to_str("")),
        prev_hash: struct_str("d", "prev_hash") | struct_field("d", "prev_hash", hex_or_keep("")),
        nonce: struct_str("d", "nonce") | struct_field("d", "nonce", hex_or_keep("")),
        address: struct_str("d", "address") | struct_field("d", "address", keep("")),
        // Optional on the wire (native presence is threshold-conditional):
        // an absent friendly value stays absent through the transport.
        custom_message: struct_opt_str("d", "custom_message") | struct_field("d", "custom_message", hex_or_keep("")),
    }),
}

// ================================ friendly group analysis ================================
//
// Shared by the JS adapter generator (codegen.rs) and the Rust build direction
// (build.rs): the wire-kind selection for one friendly kind is derived from
// the same table data on both sides, so the choice can never drift.

/// One friendly kind: the wire kinds it can produce and the JS-relevant entry.
pub struct FriendlyGroup<'a> {
    pub friendly: &'a str,
    /// Wire entries whose fields include a `to` field (JS can build them).
    pub usable: Vec<&'a ActionSpecDef>,
    pub to_kind: Option<&'a str>,
    pub from_to_kind: Option<&'a str>,
    /// Decode-only from-form kind (e.g. `transfer_hac_from`), selected by the
    /// Rust build when `to` is empty so decoded from-forms rebuild faithfully.
    pub from_only_kind: Option<&'a str>,
    pub single_entry: Option<&'a ActionSpecDef>,
    pub list_entry: Option<&'a ActionSpecDef>,
}

pub fn friendly_groups() -> Vec<FriendlyGroup<'static>> {
    let mut groups: Vec<(&'static str, Vec<&'static ActionSpecDef>)> = Vec::new();
    for def in ACTION_SPECS {
        if let Some(g) = groups.iter_mut().find(|(friendly, _)| *friendly == def.friendly) {
            g.1.push(def);
        } else {
            groups.push((def.friendly, vec![def]));
        }
    }
    groups
        .into_iter()
        .map(|(friendly, entries)| {
            // JS-usable entries carry a real `to` field (the from-only kinds
            // like transfer_hac_from keep `to: empty()` and are decode-only)
            let has_real_to = |d: &&'static ActionSpecDef| {
                d.fields
                    .iter()
                    .any(|f| f.friendly == "to" && !matches!(f.rust, RustConv::ConstEmpty))
            };
            let usable: Vec<&'static ActionSpecDef> = entries
                .iter()
                .copied()
                .filter(|d| has_real_to(d))
                .collect();
            // a real `from` field (some_str); the `none()` placeholder field
            // on the *_to kinds does not count
            let has_real_from = |d: &&'static ActionSpecDef| {
                d.fields
                    .iter()
                    .any(|f| f.friendly == "from" && !matches!(f.rust, RustConv::ConstNone))
            };
            let has_hex_single = |d: &&'static ActionSpecDef| {
                d.fields.iter().any(|f| matches!(f.js, JsConv::HexSingle(_)))
            };
            let to_kind = usable
                .iter()
                .find(|d| !has_real_from(d) && !has_hex_single(d))
                .map(|d| d.kind);
            let from_to_kind = usable
                .iter()
                .find(|d| has_real_from(d))
                .map(|d| d.kind);
            let from_only_kind = entries
                .iter()
                .find(|d| has_real_from(d) && !has_real_to(d))
                .map(|d| d.kind);
            let single_entry = usable
                .iter()
                .find(|d| d.fields.iter().any(|f| matches!(f.js, JsConv::HexSingle(_))))
                .copied();
            let list_entry = usable
                .iter()
                .find(|d| d.fields.iter().any(|f| matches!(f.js, JsConv::HexList(_))))
                .copied();
            FriendlyGroup {
                friendly,
                usable,
                to_kind,
                from_to_kind,
                from_only_kind,
                single_entry,
                list_entry,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::ActionSchema;    fn action_schemas() -> Vec<ActionSchema> {
        crate::codec::standard_codecs()
            .expect("standard codecs assembly")
            .action_schemas()
            .to_vec()
    }

    fn struct_schemas() -> Vec<base::StructSchema> {
        chain_codec::struct_schemas()
    }

    fn nested_struct_wire<'a>(action: &'a ActionSchema, field: &str) -> Option<&'a str> {
        action
            .fields
            .iter()
            .find(|f| f.name == field)
            .and_then(|f| match &f.wire {
                field::FieldWire::Struct(name) => Some(*name),
                _ => None,
            })
    }

    /// Every wire field referenced by the table must exist in the action
    /// schemas (and nested struct fields in the struct schemas); a wire
    /// rename breaks the build of the decoder here, not silently at runtime.
    #[test]
    fn table_wire_fields_exist_in_schemas() {
        let actions = action_schemas();
        let structs = struct_schemas();
        for def in ACTION_SPECS {
            let action = actions
                .iter()
                .find(|s| s.name == def.kind)
                .unwrap_or_else(|| panic!("ACTION_SPECS kind {} has no action schema", def.kind));
            for f in def.fields {
                match f.rust {
                    RustConv::ConstNone | RustConv::ConstEmpty => {}
                    RustConv::Str(w)
                    | RustConv::SomeStr(w)
                    | RustConv::Num(w)
                    | RustConv::Opt(w, _)
                    | RustConv::StrList(w)
                    | RustConv::NumList(w)
                    | RustConv::Dia(w) => assert!(
                        action.fields.iter().any(|sf| sf.name == w),
                        "{} field {} references wire field {w} missing from schema {}",
                        def.kind,
                        f.friendly,
                        def.kind
                    ),
                    RustConv::DiaName | RustConv::DiaSingle => assert!(
                        action.fields.iter().any(|sf| sf.name == "diamond"),
                        "{} references diamond field missing",
                        def.kind
                    ),
                    RustConv::DiaList => assert!(
                        action.fields.iter().any(|sf| sf.name == "diamonds"),
                        "{} references diamonds field missing",
                        def.kind
                    ),
                    RustConv::AssetNum(w) | RustConv::AssetStr(w) => {
                        // `asset` is the dedicated `AssetAmt` wire variant whose
                        // serial/amount sub-fields are intrinsic (no struct schema)
                        assert!(
                            action.fields.iter().any(|sf| {
                                sf.name == "asset"
                                    && matches!(sf.wire, field::FieldWire::AssetAmt)
                            }),
                            "{} has no asset_amt wire field",
                            def.kind
                        );
                        let _ = w;
                    }
                    RustConv::StructStr(s, w) | RustConv::StructReadable(s, w) | RustConv::StructOptStr(s, w) => {
                        let name = nested_struct_wire(action, s)
                            .unwrap_or_else(|| panic!("{} has no nested struct {s}", def.kind));
                        assert!(
                            structs.iter().any(|st| st.name == name
                                && st.fields.iter().any(|sf| sf.name == w)),
                            "{} {s}.{w} missing in nested struct {name}",
                            def.kind
                        );
                    }
                }
            }
        }
    }

    /// Every wire kind the friendly table maps must exist as a schema and every
    /// schema kind referenced by the friendly groups must be in the table.
    #[test]
    fn table_kinds_are_a_subset_of_registered_actions() {
        let actions = action_schemas();
        for def in ACTION_SPECS {
            assert!(
                actions.iter().any(|s| s.name == def.kind),
                "ACTION_SPECS kind {} not registered",
                def.kind
            );
        }
    }

    /// Wire field name a table entry references, or `None` for the
    /// placeholders (`ConstNone`/`ConstEmpty`) that are never wire fields.
    fn referenced_wire_field(conv: RustConv) -> Option<&'static str> {
        match conv {
            RustConv::Str(w)
            | RustConv::SomeStr(w)
            | RustConv::Num(w)
            | RustConv::Opt(w, _)
            | RustConv::StrList(w)
            | RustConv::NumList(w)
            | RustConv::Dia(w) => Some(w),
            RustConv::DiaName | RustConv::DiaSingle => Some("diamond"),
            RustConv::DiaList => Some("diamonds"),
            RustConv::AssetNum(_) | RustConv::AssetStr(_) => Some("asset"),
            RustConv::StructStr(s, _) | RustConv::StructReadable(s, _) | RustConv::StructOptStr(s, _) => {
                Some(s)
            }
            RustConv::ConstNone | RustConv::ConstEmpty => None,
        }
    }

    /// The reverse of `table_wire_fields_exist_in_schemas`: every wire field of
    /// every tabled kind must be covered by the table (top-level fields by
    /// name, nested struct members by member name, `AssetAmt` by serial +
    /// amount). A new schema field on a tabled kind fails here, forcing an
    /// explicit table decision instead of being silently dropped by the typed
    /// decode or failing the typed build.
    #[test]
    fn table_covers_every_schema_field() {
        let actions = action_schemas();
        let structs = struct_schemas();
        for def in ACTION_SPECS {
            let action = actions
                .iter()
                .find(|s| s.name == def.kind)
                .unwrap_or_else(|| panic!("ACTION_SPECS kind {} has no action schema", def.kind));
            for sf in action.fields {
                if sf.name == "kind" {
                    continue;
                }
                match &sf.wire {
                    field::FieldWire::Struct(struct_name) => {
                        let members = structs
                            .iter()
                            .find(|st| st.name == *struct_name)
                            .unwrap_or_else(|| {
                                panic!("{} struct {struct_name} has no struct schema", def.kind)
                            });
                        let referenced: Vec<&str> = def
                            .fields
                            .iter()
                            .filter_map(|f| match f.rust {
                                RustConv::StructStr(s, w)
                                | RustConv::StructReadable(s, w)
                                | RustConv::StructOptStr(s, w)
                                    if s == sf.name =>
                                {
                                    Some(w)
                                }
                                _ => None,
                            })
                            .collect();
                        for member in members.fields {
                            assert!(
                                referenced.contains(&member.name),
                                "{} struct {}.{} not covered by the table",
                                def.kind,
                                struct_name,
                                member.name
                            );
                        }
                    }
                    field::FieldWire::AssetAmt => {
                        let serial = def
                            .fields
                            .iter()
                            .any(|f| matches!(f.rust, RustConv::AssetNum(_)));
                        let amount = def
                            .fields
                            .iter()
                            .any(|f| matches!(f.rust, RustConv::AssetStr(_)));
                        assert!(
                            serial && amount,
                            "{} asset serial/amount not covered by the table",
                            def.kind
                        );
                    }
                    _ => {
                        let covered = def.fields.iter().any(|f| {
                            referenced_wire_field(f.rust) == Some(sf.name)
                        });
                        assert!(
                            covered,
                            "{} wire field {} not covered by the table",
                            def.kind,
                            sf.name
                        );
                    }
                }
            }
        }
    }
}
