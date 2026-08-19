//! `codec-schema-gen` — generates the TypeScript codec from Rust action/struct schemas.
//!
//! Single source of truth: every action's wire schema comes from the `ActionCodec`
//! derive or a handwritten `ActionSchemaProvider`/`StructSchemaProvider`. This tool
//! assembles the same registration graph as the fullnode through the shared
//! `chain-codec::register_standard` entry (protocol + vm + mint-core, same as
//! `app::standard_registry`'s action part), then:
//! 1. validates kind/name uniqueness and complete nested closures;
//! 2. computes a deterministic schema hash;
//! 3. generates `sdk/js/generated/codec.ts` (action metadata table + TransactionSpec
//!    binary payload codec; "algorithmic" fields such as amounts/addresses travel as
//!    strings per design A).
//!
//! `sdk/pack.sh` regenerates these on every build; `sdk/check-schema.sh`
//! verifies the checked-out copy matches the Rust schema (never hand-edit).

use base::{ActionSchema, StructSchema, validate_schema_set};

/// Collects all action schemas using the same assembly as the fullnode
/// (`chain-codec::register_standard`; CoinbaseTx is a tx, not an action).
fn collect_actions() -> Vec<ActionSchema> {
    chain_codec::collect_action_schemas()
}

fn collect_structs() -> Vec<StructSchema> {
    chain_codec::struct_schemas()
}

fn main() {
    let actions = collect_actions();
    let structs = collect_structs();

    if let Err(err) = validate_schema_set(&actions, &structs) {
        eprintln!("codec-schema-gen: schema validation failed: {err}");
        std::process::exit(1);
    }

    let hash = base::schema_set_hash(&actions, &structs);
    let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();

    let artifact = render_codec(&actions, &structs, &hash_hex);

    let gen_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("sdk/js/generated");
    std::fs::create_dir_all(&gen_dir).expect("create sdk/js/generated");
    std::fs::write(gen_dir.join("codec.ts"), artifact.ts).expect("write codec.ts");
    std::fs::write(gen_dir.join("codec.mjs"), artifact.mjs).expect("write codec.mjs");
    println!(
        "codec-schema-gen: wrote codec.ts/codec.mjs ({} actions, {} structs, schema hash {})",
        actions.len(),
        structs.len(),
        hash_hex
    );
}

// ================================ TS rendering ================================

fn wire_tag(wire: &base::FieldWire) -> String {
    match wire {
        base::FieldWire::U1 => "u1",
        base::FieldWire::U2 => "u2",
        base::FieldWire::U4 => "u4",
        base::FieldWire::U5 => "u5",
        base::FieldWire::U8 => "u8",
        base::FieldWire::Fixed(n) => return format!("fixed:{n}"),
        base::FieldWire::Amount => "amount",
        base::FieldWire::WireAmount => "wire_amount",
        base::FieldWire::Address => "address",
        base::FieldWire::AddrOrPtr => "addr_or_ptr",
        base::FieldWire::AddrOrList => "addr_or_list",
        base::FieldWire::BytesW1 => "bytes_w1",
        base::FieldWire::BytesW2 => "bytes_w2",
        base::FieldWire::Satoshi => "satoshi",
        base::FieldWire::Fold64 => "fold64",
        base::FieldWire::Timestamp => "timestamp",
        base::FieldWire::DiamondName => "diamond_name",
        base::FieldWire::DiamondNumber => "diamond_number",
        base::FieldWire::DiamondNameList => "diamond_name_list",
        base::FieldWire::AssetAmt => "asset_amt",
        base::FieldWire::AssetAmtW1 => "asset_amt_w1",
        base::FieldWire::ChainIDList => "chain_id_list",
        base::FieldWire::ContractAddrListW1 => "contract_addr_list_w1",
        base::FieldWire::SignW2 => "sign_w2",
        base::FieldWire::ListW1(name) => return format!("list_w1:{name}"),
        base::FieldWire::ListW2(name) => return format!("list_w2:{name}"),
        base::FieldWire::Struct(name) => return format!("struct:{name}"),
        base::FieldWire::ActionList => "action_list",
        base::FieldWire::ActionListW1 => "action_list_w1",
    }
    .to_owned()
}

/// Renders the `ListW1/ListW2` element resolution map from the Rust
/// `base::BUILTIN_LEAVES` table (name -> wire tag via `wire_tag`). The TS
/// runtime reads this generated object instead of a second hand-written table,
/// so adding a leaf type only touches Rust.
fn render_builtin_leaf_table() -> String {
    let entries: Vec<String> = base::BUILTIN_LEAVES
        .iter()
        .map(|(name, wire)| format!("    {:?}: {:?}", name, wire_tag(wire)))
        .collect();
    format!("{{\n{}\n  }}", entries.join(",\n"))
}

/// Design-A JS handler kind for a plain (non-prefixed) wire tag; `None` for
/// the parameterized tags (`fixed:`/`list_w1:`/`list_w2:`/`struct:`) that the
/// runtime resolves in its default branch. This match is the single source of
/// the tag → handler mapping: the JS `WIRE_HANDLER` object is rendered from
/// it, so a new `FieldWire` variant either maps onto an existing handler
/// (handled automatically) or fails to compile here.
fn wire_handler(wire: &base::FieldWire) -> Option<&'static str> {
    use base::FieldWire::*;
    Some(match wire {
        U1 | U8 => "raw_u8",
        U2 => "raw_u16",
        U4 => "raw_u32",
        U5 | Amount | WireAmount | Address | AddrOrPtr | AddrOrList | Satoshi | Fold64
        | Timestamp | DiamondNumber => "decimal_str",
        BytesW1 | BytesW2 | DiamondName | SignW2 | AssetAmtW1 => "hex_w2",
        AssetAmt => "asset_amt",
        DiamondNameList | ChainIDList | ContractAddrListW1 => "hex_list",
        ActionList => "action_list",
        ActionListW1 => "action_list_w1",
        Fixed(_) | ListW1(_) | ListW2(_) | Struct(_) => return None,
    })
}

/// Renders the `WIRE_HANDLER` object (plain wire tag -> handler kind) from
/// `wire_handler`. The parameterized tags live in the runtime's default
/// branch, so they never appear here.
fn render_wire_handler_table() -> String {
    use base::FieldWire::*;
    let variants: &[base::FieldWire] = &[
        U1,
        U8,
        U2,
        U4,
        U5,
        Amount,
        WireAmount,
        Address,
        AddrOrPtr,
        AddrOrList,
        BytesW1,
        BytesW2,
        DiamondName,
        DiamondNumber,
        Satoshi,
        Fold64,
        Timestamp,
        DiamondNameList,
        AssetAmt,
        AssetAmtW1,
        ChainIDList,
        ContractAddrListW1,
        SignW2,
        ActionList,
        ActionListW1,
    ];
    let entries: Vec<String> = variants
        .iter()
        .map(|wire| format!("    {:?}: {:?},", wire_tag(wire), wire_handler(wire).expect("plain tag")))
        .collect();
    format!("{{\n{}\n  }}", entries.join("\n"))
}

/// Renders `codec.ts` (with type declarations, for TS toolchains) and `codec.mjs`
/// (pure JS runtime for direct use in Node/browser — releases never reference .ts).
struct CodecArtifact {
    ts: String,
    mjs: String,
}

fn render_codec(
    actions: &[ActionSchema],
    structs: &[StructSchema],
    hash_hex: &str,
) -> CodecArtifact {
    let header = format!(
        "// GENERATED by `codec-schema-gen` (cargo run -p codec-schema-gen). DO NOT EDIT.\n\
         // Single source of truth: Rust `ActionSchemaProvider`/`StructSchemaProvider`.\n\
         // Design A: \"algorithmic\" fields (amounts/addresses/hex) travel as strings in the payload; parsing stays in Rust.\n\n\
         export const SCHEMA_HASH = \"{hash_hex}\";\n\n"
    );
    // .ts-only type declarations (excluded from .mjs)
    let types_ts = "export interface FieldMeta { name: string; wire: string; optional?: boolean; }\n\
                    export interface ActionMeta { kind: number; name: string; fields: FieldMeta[]; }\n\
                    export interface TxSpecAction { kind: string; [field: string]: unknown; }\n\
                    export interface TransactionSpec {\n\
                      schema: string;\n\
                      tx_type: number;\n\
                      main: string;\n\
                      fee: string;\n\
                      timestamp?: number;\n\
                      gas_max?: number;\n\
                      actions: TxSpecAction[];\n\
                    }\n\n";

    let mut meta = String::new();
    meta.push_str("export const ACTION_METADATA = [\n");
    for action in actions {
        meta.push_str(&format!(
            "  {{ kind: {}, name: {:?}, fields: [\n",
            action.kind, action.name
        ));
        for field in action.fields {
            let optional = if field.optional { ", optional: true" } else { "" };
            meta.push_str(&format!(
                "    {{ name: {:?}, wire: {:?}{} }},\n",
                field.name,
                wire_tag(&field.wire),
                optional
            ));
        }
        meta.push_str("  ] },\n");
    }
    meta.push_str("];\n\n");

    meta.push_str("export const STRUCT_METADATA = {\n");
    for s in structs {
        meta.push_str(&format!("  {:?}: [\n", s.name));
        for field in s.fields {
            let optional = if field.optional { ", optional: true" } else { "" };
            meta.push_str(&format!(
                "    {{ name: {:?}, wire: {:?}{} }},\n",
                field.name,
                wire_tag(&field.wire),
                optional
            ));
        }
        meta.push_str("  ],\n");
    }
    meta.push_str("};\n\n");

    let meta_ts = meta.replace(
        "export const ACTION_METADATA = [",
        "export const ACTION_METADATA: ActionMeta[] = [",
    );
    let meta_ts = meta_ts.replace(
        "export const STRUCT_METADATA = {",
        "export const STRUCT_METADATA: Record<string, FieldMeta[]> = {",
    );

    // The built-in leaf map is rendered from `base::BUILTIN_LEAVES` (single
    // source, Rust); `fieldWireOf` reads it instead of a second table. The
    // wire-tag → handler map is rendered from `wire_handler` (single source,
    // Rust); the runtime dispatch reads it instead of a second switch.
    let runtime = TS_RUNTIME
        .replace("__BUILTIN_LEAF_WIRE__", &render_builtin_leaf_table())
        .replace("__WIRE_HANDLER__", &render_wire_handler_table());

    CodecArtifact {
        ts: format!("{header}{types_ts}{meta_ts}{runtime}"),
        mjs: format!("{header}{meta}{runtime}"),
    }
}

const TS_RUNTIME: &str = r##"
export const ACTION_BY_KIND = new Map(
  ACTION_METADATA.map((m) => [m.kind, m]),
);
export const ACTION_BY_NAME = new Map(
  ACTION_METADATA.map((m) => [m.name, m]),
);

// ---- TransactionSpec binary payload v1 (design A) ----
// Layout: u8 tx_type | W2 main (base58 string) | W2 fee (decimal string)
//       | u64 timestamp | u8 gas_max | u16 action_count
//       | per action: u16 kind + fields per schema (amounts/addresses/hex as W2 strings)
// Numeric fields: u1/u2/u4/fixed as raw big-endian bytes; u5/u8/satoshi/timestamp as decimal strings.
// All numeric/hex inputs are validated first; silent truncation or rewriting is rejected.

export function checkUint(name, v, max) {
  if (!Number.isInteger(v) || v < 0 || v > max) {
    throw new Error(`${name} must be an integer in [0, ${max}], got ${v}`);
  }
}
export function pushU16(out, v) {
  checkUint("u16", v, 0xffff);
  out.push((v >> 8) & 0xff, v & 0xff);
}
export function pushU32(out, v) {
  checkUint("u32", v, 0xffffffff);
  out.push((v >>> 24) & 0xff, (v >>> 16) & 0xff, (v >>> 8) & 0xff, v & 0xff);
}
export function pushU64(out, v) {
  checkUint("u64", v, Number.MAX_SAFE_INTEGER);
  const hi = Math.floor(v / 0x100000000);
  const lo = v % 0x100000000;
  out.push((hi >>> 24) & 0xff, (hi >>> 16) & 0xff, (hi >>> 8) & 0xff, hi & 0xff);
  out.push((lo >>> 24) & 0xff, (lo >>> 16) & 0xff, (lo >>> 8) & 0xff, lo & 0xff);
}
export function pushStrW2(out, s) {
  const bytes = Array.from(new TextEncoder().encode(s));
  if (bytes.length > 0xffff) throw new Error(`string too long: ${bytes.length} bytes`);
  pushU16(out, bytes.length);
  out.push(...bytes);
}
const HEX_RE = /^[0-9a-fA-F]*$/;
export function pushHexW2(out, hex) {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (clean.length % 2 !== 0) throw new Error(`hex field must have even length: ${hex}`);
  if (!HEX_RE.test(clean)) throw new Error(`hex field has invalid characters: ${hex}`);
  pushU16(out, clean.length / 2);
  for (let i = 0; i < clean.length; i += 2) {
    out.push(parseInt(clean.slice(i, i + 2), 16));
  }
}
function readU16(buf, pos) {
  const v = (buf[pos.p] << 8) | buf[pos.p + 1];
  pos.p += 2;
  return v;
}
function readU32(buf, pos) {
  const v =
    ((buf[pos.p] << 24) | (buf[pos.p + 1] << 16) | (buf[pos.p + 2] << 8) | buf[pos.p + 3]) >>> 0;
  pos.p += 4;
  return v;
}
function readU64(buf, pos) {
  const hi =
    ((buf[pos.p] << 24) | (buf[pos.p + 1] << 16) | (buf[pos.p + 2] << 8) | buf[pos.p + 3]) >>> 0;
  const lo =
    ((buf[pos.p + 4] << 24) | (buf[pos.p + 5] << 16) | (buf[pos.p + 6] << 8) | buf[pos.p + 7]) >>> 0;
  pos.p += 8;
  return hi * 0x100000000 + lo;
}
function readStrW2(buf, pos) {
  const len = readU16(buf, pos);
  const s = new TextDecoder().decode(buf.subarray(pos.p, pos.p + len));
  pos.p += len;
  return s;
}
function readHexW2(buf, pos) {
  const len = readU16(buf, pos);
  let hex = "";
  for (let i = 0; i < len; i++) hex += buf[pos.p + i].toString(16).padStart(2, "0");
  pos.p += len;
  return hex;
}

// Plain wire tag -> handler kind, rendered from `wire_handler` (single
// source, Rust); parameterized tags (`fixed:`/`list_w1:`/`list_w2:`/`struct:`)
// are resolved in the switch's `undefined` branch.
const WIRE_HANDLER = __WIRE_HANDLER__;

function encodeFieldValue(out, value, wire) {
  switch (WIRE_HANDLER[wire]) {
    case "raw_u8":
      checkUint("u8", Number(value), 0xff);
      out.push(Number(value));
      break;
    case "raw_u16":
      pushU16(out, Number(value));
      break;
    case "raw_u32":
      pushU32(out, Number(value));
      break;
    case "decimal_str":
      // Design A: strings on the wire, parsed in Rust
      pushStrW2(out, String(value));
      break;
    case "hex_w2":
      pushHexW2(out, String(value));
      break;
    case "asset_amt": {
      // Design A: serial/amount as decimal strings (Fold64 compressed encoding stays in Rust)
      pushStrW2(out, String(value.serial));
      pushStrW2(out, String(value.amount));
      break;
    }
    case "hex_list": {
      const items = value;
      pushU16(out, items.length);
      for (const item of items) pushHexW2(out, String(item));
      break;
    }
    case "action_list": {
      const items = value;
      pushU16(out, items.length);
      for (const item of items) encodeAction(out, item);
      break;
    }
    case "action_list_w1": {
      const items = value;
      if (items.length > 0xff) throw new Error(`action list exceeds 255 items: ${items.length}`);
      out.push(items.length);
      for (const item of items) encodeAction(out, item);
      break;
    }
    case undefined:
      if (wire.startsWith("fixed:")) {
        const n = Number(wire.slice(6));
        const hex = String(value).replace(/^0x/, "");
        if (hex.length !== n * 2) throw new Error(`fixed(${n}) field must be ${n * 2} hex chars`);
        if (!HEX_RE.test(hex)) throw new Error(`fixed(${n}) field has invalid characters: ${hex}`);
        for (let i = 0; i < hex.length; i += 2) out.push(parseInt(hex.slice(i, i + 2), 16));
        break;
      }
      if (wire.startsWith("list_w1:") || wire.startsWith("list_w2:")) {
        const items = value;
        pushU16(out, items.length);
        const elemWire = fieldWireOf(wire.slice(8));
        for (const item of items) encodeFieldValue(out, item, elemWire);
        break;
      }
      if (wire.startsWith("struct:")) {
        const name = wire.slice(7);
        // Nested references check STRUCT_METADATA first, then fall back to action metadata
        // (action names like `ast_select` used as nested struct references).
        const meta = STRUCT_METADATA[name] ?? ACTION_METADATA.find((m) => m.name === name);
        if (!meta) throw new Error(`unknown struct schema: ${wire}`);
        encodeStruct(out, value, meta);
        break;
      }
      throw new Error(`unsupported wire shape: ${wire}`);
  }
}

// Generated from `base::BUILTIN_LEAVES` (see render_builtin_leaf_table); the
// `ListW1/ListW2` element resolution map exists only in Rust.
const BUILTIN_LEAF_WIRE = __BUILTIN_LEAF_WIRE__;

function fieldWireOf(name) {
  for (const meta of ACTION_METADATA) {
    if (meta.name === name) return "struct:" + name;
  }
  // Built-in leaves and nested structs come from STRUCT_METADATA
  if (STRUCT_METADATA[name]) return "struct:" + name;
  const w = BUILTIN_LEAF_WIRE[name];
  if (w) return w;
  throw new Error(`unknown list element name: ${name}`);
}

function encodeStruct(out, obj, meta) {
  if (meta.length === 0) {
    // Placeholder empty schemas (e.g. TexCell/FuncArgvTypes) can't be encoded: error instead of silently writing zero bytes
    throw new Error(`struct schema has no fields (not yet supported)`);
  }
  // Unknown fields are rejected: the binary equivalent of deny_unknown_fields
  const known = new Set(meta.map((f) => f.name));
  for (const key of Object.keys(obj)) {
    if (!known.has(key)) throw new Error(`unknown field ${key}`);
  }
  for (const field of meta) {
    if (field.name === "kind") continue;
    if (!(field.name in obj) && !field.optional) throw new Error(`missing field ${field.name}`);
    if (field.optional) {
      // Optional fields travel as W2 length + value (length 0 = absent), so
      // presence is never ambiguous with the following data.
      const value = obj[field.name];
      if (value === undefined || value === null || value === "") {
        pushU16(out, 0);
      } else {
        const tmp = [];
        encodeFieldValue(tmp, value, field.wire);
        pushU16(out, tmp.length);
        out.push(...tmp);
      }
    } else {
      encodeFieldValue(out, obj[field.name], field.wire);
    }
  }
}

function encodeAction(out, action) {
  const meta = ACTION_BY_NAME.get(action.kind);
  if (!meta) throw new Error(`unknown action name: ${action.kind}`);
  pushU16(out, meta.kind);
  encodeStruct(out, action, meta.fields);
}

export function encodeTransactionSpec(spec) {
  checkUint("tx_type", spec.tx_type, 0xff);
  checkUint("gas_max", spec.gas_max ?? 0, 0xff);
  const out = [];
  out.push(spec.tx_type & 0xff);
  pushStrW2(out, spec.main);
  pushStrW2(out, spec.fee);
  pushU64(out, spec.timestamp ?? 0);
  out.push(spec.gas_max ?? 0);
  pushU16(out, spec.actions.length);
  for (const action of spec.actions) encodeAction(out, action);
  return new Uint8Array(out);
}

function decodeFieldValue(buf, pos, wire) {
  switch (WIRE_HANDLER[wire]) {
    case "raw_u8":
      return buf[pos.p++];
    case "raw_u16":
      return readU16(buf, pos);
    case "raw_u32":
      return readU32(buf, pos);
    case "decimal_str":
      return readStrW2(buf, pos);
    case "hex_w2":
      return readHexW2(buf, pos);
    case "asset_amt": {
      return { serial: readStrW2(buf, pos), amount: readStrW2(buf, pos) };
    }
    case "hex_list": {
      const n = readU16(buf, pos);
      const items = [];
      for (let i = 0; i < n; i++) items.push(readHexW2(buf, pos));
      return items;
    }
    case "action_list": {
      const n = readU16(buf, pos);
      const items = [];
      for (let i = 0; i < n; i++) items.push(decodeAction(buf, pos));
      return items;
    }
    case "action_list_w1": {
      const n = buf[pos.p++];
      const items = [];
      for (let i = 0; i < n; i++) items.push(decodeAction(buf, pos));
      return items;
    }
    case undefined:
      if (wire.startsWith("fixed:")) {
        const n = Number(wire.slice(6));
        let hex = "";
        for (let i = 0; i < n; i++) hex += buf[pos.p + i].toString(16).padStart(2, "0");
        pos.p += n;
        return hex;
      }
      if (wire.startsWith("list_w1:") || wire.startsWith("list_w2:")) {
        const n = readU16(buf, pos);
        const elemWire = fieldWireOf(wire.slice(8));
        const items = [];
        for (let i = 0; i < n; i++) items.push(decodeFieldValue(buf, pos, elemWire));
        return items;
      }
      if (wire.startsWith("struct:")) {
        const name = wire.slice(7);
        const meta = STRUCT_METADATA[name] ?? ACTION_METADATA.find((m) => m.name === name);
        if (!meta) throw new Error(`unknown struct schema: ${wire}`);
        return decodeStruct(buf, pos, meta);
      }
      throw new Error(`unsupported wire shape: ${wire}`);
  }
}

function decodeStruct(buf, pos, meta) {
  if (meta.length === 0) {
    throw new Error(`struct schema has no fields (not yet supported)`);
  }
  const obj = {};
  for (const field of meta) {
    if (field.name === "kind") continue;
    if (field.optional) {
      // W2 length prefix; length 0 = absent.
      const len = readU16(buf, pos);
      if (len === 0) continue;
      const inner = buf.subarray(pos.p, pos.p + len);
      pos.p += len;
      obj[field.name] = decodeFieldValue(inner, { p: 0 }, field.wire);
    } else {
      obj[field.name] = decodeFieldValue(buf, pos, field.wire);
    }
  }
  return obj;
}

function decodeAction(buf, pos) {
  const kind = readU16(buf, pos);
  const meta = ACTION_BY_KIND.get(kind);
  if (!meta) throw new Error(`unknown action kind: ${kind}`);
  const obj = { kind: meta.name };
  for (const field of meta.fields) {
    if (field.name === "kind") continue;
    obj[field.name] = decodeFieldValue(buf, pos, field.wire);
  }
  return obj;
}

export function decodeTransactionSpec(buf) {
  const pos = { p: 0 };
  const tx_type = buf[pos.p++];
  const main = readStrW2(buf, pos);
  const fee = readStrW2(buf, pos);
  const timestamp = readU64(buf, pos);
  const gas_max = buf[pos.p++];
  const count = readU16(buf, pos);
  const actions = [];
  for (let i = 0; i < count; i++) actions.push(decodeAction(buf, pos));
  if (pos.p !== buf.length) throw new Error(`trailing bytes in TransactionSpec payload`);
  return {
    schema: "hacash.sdk/transaction-spec@1",
    tx_type,
    main,
    fee,
    timestamp: timestamp === 0 ? undefined : timestamp,
    gas_max,
    actions,
  };
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Every wire tag reachable from the schema set must be handled by the JS
    /// runtime: plain tags must appear in the rendered `WIRE_HANDLER`, the
    /// parameterized tags (`fixed:`/`list_w1:`/`list_w2:`/`struct:`) in the
    /// default branch. Adding a `FieldWire` variant that a schema uses fails
    /// here — never silently at encode time.
    #[test]
    fn every_schema_used_wire_tag_is_handled() {
        let actions = collect_actions();
        let structs = collect_structs();
        let struct_map: HashMap<&str, &base::StructSchema> =
            structs.iter().map(|s| (s.name, s)).collect();
        let action_map: HashMap<&str, &base::ActionSchema> =
            actions.iter().map(|a| (a.name, a)).collect();
        let mut seen: HashSet<String> = HashSet::new();

        fn walk(
            wire: &base::FieldWire,
            struct_map: &HashMap<&str, &base::StructSchema>,
            action_map: &HashMap<&str, &base::ActionSchema>,
            seen: &mut HashSet<String>,
        ) {
            seen.insert(wire_tag(wire));
            match wire {
                base::FieldWire::ListW1(n) | base::FieldWire::ListW2(n) => {
                    let fields = struct_map
                        .get(n)
                        .map(|s| s.fields.as_ref())
                        .or_else(|| action_map.get(n).map(|a| a.fields.as_ref()));
                    match fields {
                        Some(fields) => {
                            for f in fields {
                                walk(&f.wire, struct_map, action_map, seen);
                            }
                        }
                        None => {
                            if let Some(w) = base::builtin_leaf_wire(n) {
                                walk(&w, struct_map, action_map, seen);
                            }
                        }
                    }
                }
                base::FieldWire::Struct(n) => {
                    let fields = struct_map
                        .get(n)
                        .map(|s| s.fields.as_ref())
                        .or_else(|| action_map.get(n).map(|a| a.fields.as_ref()));
                    if let Some(fields) = fields {
                        for f in fields {
                            walk(&f.wire, struct_map, action_map, seen);
                        }
                    }
                }
                _ => {}
            }
        }

        for action in &actions {
            for f in action.fields {
                walk(&f.wire, &struct_map, &action_map, &mut seen);
            }
        }
        for s in &structs {
            for f in s.fields {
                walk(&f.wire, &struct_map, &action_map, &mut seen);
            }
        }

        let rendered = render_wire_handler_table();
        for tag in seen {
            if tag.starts_with("fixed:")
                || tag.starts_with("list_w1:")
                || tag.starts_with("list_w2:")
                || tag.starts_with("struct:")
            {
                continue;
            }
            assert!(
                rendered.contains(&format!("    {tag:?}:")),
                "wire tag {tag} is used by a schema but missing from the generated WIRE_HANDLER"
            );
        }
    }
}
