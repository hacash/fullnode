use std::collections::{HashMap, HashSet};

use hex;

/// Source map for human-readable decompilation: maps bytecode/IR indices back
/// to fitsh source names (libraries, function selectors, local slots, const
/// literals, parameter prelude). Optional on every decompile path — absent
/// maps degrade to structural names, present maps maximize readability.
///
/// JSON wire format (shared with the SDK boundary; hand-written `field::json_*`
/// engine, serde is only a test oracle):
/// ```json
/// {
///   "libs":   [{"idx": 1, "name": "lib.a", "address": "1MzN..."}],
///   "funcs":  [{"sig": "aabbccdd", "name": "my_func"}],
///   "slots":  ["x", "", "y"],
///   "lets":   [0],
///   "vars":   [2],
///   "params": ["a", "b"],
///   "param_prelude_count": 2,
///   "consts": [{"name": "MAX", "value": "100"}]
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Param,
    Var,
    Let,
}

#[derive(Debug, Clone)]
pub struct LibInfo {
    pub name: String,
    pub address: Option<Address>,
}

#[derive(Debug, Clone)]
pub struct SourceMap {
    libs: HashMap<u8, LibInfo>,
    funcs: HashMap<FnSign, String>,
    slots: HashMap<u8, String>,
    params: Vec<String>,
    param_prelude_count: Option<u8>,
    lets: HashSet<u8>,
    vars: HashSet<u8>,
    const_val_to_name: HashMap<String, String>,
    const_name_to_val: HashMap<String, String>,
}

impl Default for SourceMap {
    fn default() -> Self {
        Self {
            libs: HashMap::new(),
            funcs: HashMap::new(),
            slots: HashMap::new(),
            params: Vec::new(),
            param_prelude_count: None,
            lets: HashSet::new(),
            vars: HashSet::new(),
            const_val_to_name: HashMap::new(),
            const_name_to_val: HashMap::new(),
        }
    }
}

impl SourceMap {
    /// Keep only function labels for security-sensitive review output. Slot names,
    /// declaration classes, parameters, library aliases, and constants can all
    /// change the apparent structure or values of ordinary decompilation.
    pub fn review_labels(&self) -> (Self, Vec<String>, Vec<String>) {
        let mut safe = SourceMap::default();
        let mut warnings = Vec::new();
        let mut labels = Vec::new();
        collect_validated_funcs_slots(self, &mut safe, &mut labels, &mut warnings);
        if !self.const_name_to_val.is_empty() { warnings.push("constant labels ignored in review mode; numeric literals are preserved".to_owned()); }
        if !self.libs.is_empty() { warnings.push("library aliases ignored in review mode; ext(index) is preserved".to_owned()); }
        if !self.params.is_empty() || self.param_prelude_count.is_some() { warnings.push("parameter annotations ignored in review mode".to_owned()); }
        if !self.lets.is_empty() || !self.vars.is_empty() { warnings.push("declaration annotations ignored in review mode".to_owned()); }
        (safe, labels, warnings)
    }

    /// Validated labels for the named (non-review) decompilation of UNTRUSTED
    /// input: function labels and slot names that pass the `review_labels`
    /// checks, plus library aliases. Alias names must be legal identifiers and
    /// their addresses are dropped — a dapp must not imprint address bindings
    /// the transaction's own address table does not carry. Parameter names are
    /// carried too (same identifier validation; the formatter cross-checks the
    /// count against the IR's canonical prelude shape, so a hostile count only
    /// degrades to the raw prelude) — without them the `param { ... }` binding
    /// line is lost and body references to accepted slot names would not
    /// parse. Declaration classes and constants stay excluded: they can change
    /// the apparent structure or values of ordinary decompilation.
    pub fn display_labels(&self) -> (Self, Vec<String>, Vec<String>) {
        let mut safe = SourceMap::default();
        let mut warnings = Vec::new();
        let mut labels = Vec::new();
        collect_validated_funcs_slots(self, &mut safe, &mut labels, &mut warnings);
        for (idx, info) in self.lib_entries() {
            if review_identifier(&info.name) {
                let _ = safe.register_lib(idx, info.name.clone(), None);
                // Compilable form (`lib AMM = 1`), matching the prelude the
                // named decompile emits.
                labels.push(format!("lib {} = {}", info.name, idx));
            } else {
                warnings.push(format!("library alias for ext({}) ignored: invalid identifier", idx));
            }
        }
        // Param names must be pairwise distinct, identifier-legal, and agree
        // with `param_prelude_count`. They also override the accepted slot
        // labels for the covered slots: the `param { ... }` line is the
        // binding the emitted body must reference, so slot and param names
        // may not disagree. A later slot reusing a param name would print a
        // second declaration of an already-bound name — drop it instead.
        if !self.params.is_empty() || self.param_prelude_count.is_some() {
            let names = &self.params;
            let distinct = names.iter().collect::<HashSet<_>>().len() == names.len();
            let count_ok = self.param_prelude_count.is_some_and(|c| c as usize == names.len());
            if count_ok && distinct && names.iter().all(|n| review_identifier(n)) {
                safe.params = names.clone();
                safe.param_prelude_count = self.param_prelude_count;
                for (i, name) in names.iter().enumerate() {
                    safe.slots.insert(i as u8, name.clone());
                }
                let collisions: Vec<u8> = safe
                    .slots
                    .iter()
                    .filter_map(|(&slot, name)| {
                        (slot >= names.len() as u8 && names.iter().any(|n| n == name))
                            .then_some(slot)
                    })
                    .collect();
                for slot in collisions {
                    if let Some(name) = safe.slots.remove(&slot) {
                        warnings.push(format!(
                            "slot ${} label ignored: collides with parameter name {}",
                            slot, name
                        ));
                    }
                }
            } else {
                warnings.push(
                    "parameter annotations ignored: invalid, duplicated, or count-mismatched names"
                        .to_owned(),
                );
            }
        }
        (safe, labels, warnings)
    }
    pub fn register_lib(&mut self, idx: u8, name: String, address: Option<Address>) -> Rerr {
        if self.libs.contains_key(&idx) {
            return errf!("lib index {} already in source map", idx);
        }
        self.libs.insert(idx, LibInfo { name, address });
        Ok(())
    }

    pub fn register_func(&mut self, sig: [u8; 4], name: String) -> Rerr {
        self.funcs.insert(sig, name);
        Ok(())
    }

    /// Register a local slot name for decompilation. Slots start pessimistically as `let` and
    /// are promoted to `var` only when `mark_slot_mutated` is observed during parsing.
    pub fn register_slot(&mut self, slot: u8, name: String) -> Rerr {
        self.slots.insert(slot, name);
        self.vars.remove(&slot);
        self.lets.insert(slot);
        Ok(())
    }

    pub fn register_const(&mut self, name: String, value: String) -> Rerr {
        self.const_val_to_name.insert(value.clone(), name.clone());
        self.const_name_to_val.insert(name, value);
        Ok(())
    }

    pub fn get_const_name(&self, value: &str) -> Option<&String> {
        self.const_val_to_name.get(value)
    }

    pub fn get_const_value(&self, name: &str) -> Option<&String> {
        self.const_name_to_val.get(name)
    }

    pub fn register_param_names(&mut self, names: Vec<String>) -> Rerr {
        self.params = names;
        Ok(())
    }

    pub fn register_param_prelude_count(&mut self, count: u8) -> Rerr {
        self.param_prelude_count = Some(count);
        Ok(())
    }

    pub fn param_prelude_count(&self) -> Option<u8> {
        self.param_prelude_count
    }

    pub fn param_names(&self) -> Option<&Vec<String>> {
        if self.params.is_empty() {
            None
        } else {
            Some(&self.params)
        }
    }

    pub fn lib(&self, idx: u8) -> Option<&LibInfo> {
        self.libs.get(&idx)
    }

    pub fn func(&self, sig: &[u8; 4]) -> Option<&String> {
        self.funcs.get(sig)
    }

    pub fn slot(&self, slot: u8) -> Option<&String> {
        self.slots.get(&slot)
    }

    pub fn lib_entries(&self) -> Vec<(u8, LibInfo)> {
        let mut libs: Vec<(u8, LibInfo)> = self
            .libs
            .iter()
            .map(|(&idx, info)| (idx, info.clone()))
            .collect();
        libs.sort_by_key(|(idx, _)| *idx);
        libs
    }

    pub fn mark_slot_mutated(&mut self, slot: u8) {
        if self.vars.contains(&slot) {
            return;
        }
        self.lets.remove(&slot);
        self.vars.insert(slot);
    }

    pub fn slot_is_var(&self, slot: u8) -> bool {
        self.vars.contains(&slot)
    }

    pub fn slot_is_let(&self, slot: u8) -> bool {
        self.lets.contains(&slot)
    }

    /// Serialize to the SDK boundary JSON format (hand-written builder).
    ///
    /// Empty sections are omitted entirely (`{}` when the map is empty):
    /// a typical maincall map carries far more `lets:[]`/`vars:[]`/`params:[]`/
    /// `consts:[]`/`param_prelude_count:null` padding than payload. This is
    /// wire-compatible — `from_json` (the only machine consumer; the wallet
    /// passes the object through opaquely) treats missing keys as absent.
    pub fn to_json(&self) -> Ret<String> {
        let mut sections: Vec<String> = Vec::with_capacity(8);

        if !self.libs.is_empty() {
            let mut libs: Vec<(u8, &LibInfo)> = self
                .libs
                .iter()
                .map(|(&idx, info)| (idx, info))
                .collect();
            libs.sort_by_key(|(idx, _)| *idx);
            let mut buf = String::from("\"libs\":[");
            for (i, (idx, info)) in libs.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push_str(&format!(
                    "{{\"idx\":{},\"name\":{},\"address\":",
                    idx,
                    field::json_escape(&info.name)
                ));
                match &info.address {
                    Some(addr) => buf.push_str(&field::json_escape(&addr.to_readable())),
                    None => buf.push_str("null"),
                }
                buf.push('}');
            }
            buf.push(']');
            sections.push(buf);
        }

        if !self.funcs.is_empty() {
            let mut funcs: Vec<([u8; 4], &String)> =
                self.funcs.iter().map(|(sig, name)| (*sig, name)).collect();
            funcs.sort_by(|a, b| a.0.cmp(&b.0));
            let mut buf = String::from("\"funcs\":[");
            for (i, (sig, name)) in funcs.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push_str(&format!(
                    "{{\"sig\":\"{}\",\"name\":{}}}",
                    hex::encode(sig),
                    field::json_escape(name)
                ));
            }
            buf.push(']');
            sections.push(buf);
        }

        // Slots travel as an index-addressed array ('' = unnamed gap).
        if !self.slots.is_empty() {
            let max_slot = self.slots.keys().max().copied().unwrap_or(0);
            let mut buf = String::from("\"slots\":[");
            for i in 0..=max_slot {
                if i > 0 {
                    buf.push(',');
                }
                match self.slots.get(&i) {
                    Some(name) => buf.push_str(&field::json_escape(name)),
                    None => buf.push_str("\"\""),
                }
            }
            buf.push(']');
            sections.push(buf);
        }

        let mut lets: Vec<u8> = self.lets.iter().copied().collect();
        lets.sort_unstable();
        if !lets.is_empty() {
            let joined = lets
                .iter()
                .map(|slot| slot.to_string())
                .collect::<Vec<_>>()
                .join(",");
            sections.push(format!("\"lets\":[{}]", joined));
        }

        let mut vars: Vec<u8> = self.vars.iter().copied().collect();
        vars.sort_unstable();
        if !vars.is_empty() {
            let joined = vars
                .iter()
                .map(|slot| slot.to_string())
                .collect::<Vec<_>>()
                .join(",");
            sections.push(format!("\"vars\":[{}]", joined));
        }

        if !self.params.is_empty() {
            let joined = self
                .params
                .iter()
                .map(|name| field::json_escape(name))
                .collect::<Vec<_>>()
                .join(",");
            sections.push(format!("\"params\":[{}]", joined));
        }

        if let Some(n) = self.param_prelude_count {
            sections.push(format!("\"param_prelude_count\":{}", n));
        }

        if !self.const_name_to_val.is_empty() {
            let mut consts: Vec<(&String, &String)> = self
                .const_name_to_val
                .iter()
                .map(|(name, value)| (name, value))
                .collect();
            consts.sort_by(|a, b| a.0.cmp(b.0));
            let mut buf = String::from("\"consts\":[");
            for (i, (name, value)) in consts.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push_str(&format!(
                    "{{\"name\":{},\"value\":{}}}",
                    field::json_escape(name),
                    field::json_escape(value)
                ));
            }
            buf.push(']');
            sections.push(buf);
        }

        Ok(format!("{{{}}}", sections.join(",")))
    }

    /// Parse the SDK boundary JSON format (hand-written `field::json_*` engine).
    pub fn from_json(text: &str) -> Ret<Self> {
        let pairs = field::json_split_object(text)
            .map_err(|e| sys::Error::normal(format!("source map json invalid: {e}")))?;
        let mut map = SourceMap::default();

        for (key, value) in pairs {
            match key {
                "libs" => {
                    for item in field::json_split_array(value).map_err(json_field_err("libs"))? {
                        let entries = field::json_split_object(item)
                            .map_err(json_field_err("libs item"))?;
                        let mut idx = None;
                        let mut name = None;
                        let mut address = None;
                        for (k, v) in entries {
                            match k {
                                "idx" => {
                                    idx = Some(
                                        json_num::<u8>(v).map_err(json_field_err("libs idx"))?,
                                    );
                                }
                                "name" => {
                                    name = Some(json_str(v).map_err(json_field_err("libs name"))?);
                                }
                                "address" => {
                                    if v.trim() != "null" {
                                        address = Some(
                                            json_str(v).map_err(json_field_err("libs address"))?,
                                        );
                                    }
                                }
                                _ => {}
                            }
                        }
                        let idx = idx.ok_or_else(|| field_err("source map libs item missing idx"))?;
                        let name = name.ok_or_else(|| field_err("source map libs item missing name"))?;
                        let address = match address {
                            Some(addr) => Some(
                                Address::from_readable(&addr)
                                    .map_err(|_| field_err("source map address parse failed"))?,
                            ),
                            None => None,
                        };
                        map.register_lib(idx, name, address)?;
                    }
                }
                "funcs" => {
                    for item in field::json_split_array(value).map_err(json_field_err("funcs"))? {
                        let entries = field::json_split_object(item)
                            .map_err(json_field_err("funcs item"))?;
                        let mut sig = None;
                        let mut name = None;
                        for (k, v) in entries {
                            match k {
                                "sig" => {
                                    sig = Some(json_str(v).map_err(json_field_err("funcs sig"))?);
                                }
                                "name" => {
                                    name = Some(json_str(v).map_err(json_field_err("funcs name"))?);
                                }
                                _ => {}
                            }
                        }
                        let sig = sig.ok_or_else(|| field_err("source map funcs item missing sig"))?;
                        let name = name
                            .ok_or_else(|| field_err("source map funcs item missing name"))?;
                        let bytes = hex::decode(&sig)
                            .map_err(|_| field_err("source map function signature decode failed"))?;
                        if bytes.len() != 4 {
                            return errf!("source map function signature length invalid");
                        }
                        let mut sig_bytes = [0u8; 4];
                        sig_bytes.copy_from_slice(&bytes);
                        map.register_func(sig_bytes, name)?;
                    }
                }
                "slots" => {
                    for (i, item) in field::json_split_array(value)
                        .map_err(json_field_err("slots"))?
                        .into_iter()
                        .enumerate()
                    {
                        let name = json_str(item).map_err(json_field_err("slots item"))?;
                        if name.is_empty() {
                            continue;
                        }
                        let slot = u8::try_from(i)
                            .map_err(|_| field_err("source map slot index out of range"))?;
                        map.slots.insert(slot, name);
                    }
                }
                "lets" => {
                    for item in field::json_split_array(value).map_err(json_field_err("lets"))? {
                        map.lets
                            .insert(json_num::<u8>(item).map_err(json_field_err("lets item"))?);
                    }
                }
                "vars" => {
                    for item in field::json_split_array(value).map_err(json_field_err("vars"))? {
                        let slot = json_num::<u8>(item).map_err(json_field_err("vars item"))?;
                        map.vars.insert(slot);
                        map.lets.remove(&slot);
                    }
                }
                "params" => {
                    let mut params = Vec::new();
                    for item in field::json_split_array(value)
                        .map_err(json_field_err("params"))?
                    {
                        params.push(json_str(item).map_err(json_field_err("params item"))?);
                    }
                    map.params = params;
                }
                "param_prelude_count" => {
                    if value.trim() != "null" {
                        map.param_prelude_count = Some(
                            json_num::<u8>(value).map_err(json_field_err("param_prelude_count"))?,
                        );
                    }
                }
                "consts" => {
                    for item in field::json_split_array(value)
                        .map_err(json_field_err("consts"))?
                    {
                        let entries = field::json_split_object(item)
                            .map_err(json_field_err("consts item"))?;
                        let mut name = None;
                        let mut value = None;
                        for (k, v) in entries {
                            match k {
                                "name" => {
                                    name =
                                        Some(json_str(v).map_err(json_field_err("consts name"))?);
                                }
                                "value" => {
                                    value =
                                        Some(json_str(v).map_err(json_field_err("consts value"))?);
                                }
                                _ => {}
                            }
                        }
                        let name = name
                            .ok_or_else(|| field_err("source map consts item missing name"))?;
                        let value = value
                            .ok_or_else(|| field_err("source map consts item missing value"))?;
                        map.register_const(name, value)?;
                    }
                }
                _ => return errf!("source map json field {} is unknown", key),
            }
        }
        Ok(map)
    }
}

/// Shared validation for function and slot labels (used by both the review
/// and the named-display channels): a function label must be a legal
/// identifier whose own signature hashes back to the selector it labels, a
/// slot label must be a legal identifier and distinct from every other
/// accepted slot label (a duplicate would emit a second declaration of an
/// already-bound name).
fn collect_validated_funcs_slots(
    map: &SourceMap,
    safe: &mut SourceMap,
    labels: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    for (sig, name) in &map.funcs {
        if !review_identifier(name) {
            warnings.push(format!("function label for {} ignored: invalid identifier", hex::encode(sig)));
        } else if crate::rt::calc_func_sign(name) != *sig {
            warnings.push(format!("function label for {} ignored: name does not hash to this selector", hex::encode(sig)));
        } else {
            safe.funcs.insert(*sig, name.clone());
            labels.push(format!("selector 0x{} = {}", hex::encode(sig), name));
        }
    }
    let mut seen_slot_names: HashSet<String> = HashSet::new();
    for (slot, name) in &map.slots {
        if !review_identifier(name) {
            warnings.push(format!("slot ${} label ignored: invalid identifier", slot));
        } else if !seen_slot_names.insert(name.clone()) {
            warnings.push(format!("slot ${} label ignored: duplicates an earlier slot name", slot));
        } else {
            safe.slots.insert(*slot, name.clone());
            labels.push(format!("slot ${} = {}", slot, name));
        }
    }
}

fn review_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else { return false; };
    value.len() <= 64
        && (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn json_field_err<'a>(field: &'a str) -> impl Fn(sys::Error) -> sys::Error + 'a {
    move |_| sys::Error::normal(format!("source map {field} invalid"))
}

/// `errf!` yields a `Result`, so error-carrier combinators need a direct value.
fn field_err(msg: &str) -> sys::Error {
    sys::Error::normal(msg.to_string())
}

/// Raw string value that must be a quoted JSON string (escape-decoded).
fn json_str(raw: &str) -> Ret<String> {
    field::json_expect_quoted_decoded(raw)
        .map_err(|e| sys::Error::normal(format!("source map json string invalid: {e}")))
}

/// Raw unquoted numeric value.
fn json_num<T: std::str::FromStr>(raw: &str) -> Ret<T> {
    let text = field::json_expect_unquoted(raw)
        .map_err(|_| sys::Error::normal("source map json value must be unquoted"))?;
    text.trim()
        .parse::<T>()
        .map_err(|_| sys::Error::normal("source map json value is not a number"))
}

#[cfg(test)]
mod sourcemap_tests {
    use super::*;

    #[test]
    fn register_slot_defaults_to_let_until_mutated() {
        let mut map = SourceMap::default();
        map.register_slot(7, "x".to_string()).unwrap();
        assert!(map.slot_is_let(7));
        assert!(!map.slot_is_var(7));
        map.mark_slot_mutated(7);
        assert!(map.slot_is_var(7));
        assert!(!map.slot_is_let(7));
    }

    #[test]
    fn json_roundtrip_preserves_all_sections() {
        let mut map = SourceMap::default();
        map.register_lib(
            1,
            "lib.a".to_string(),
            Some(Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap()),
        )
        .unwrap();
        map.register_lib(2, "lib.b".to_string(), None).unwrap();
        map.register_func([0x01, 0x02, 0x03, 0x04], "my_func".to_string())
            .unwrap();
        map.register_slot(0, "x".to_string()).unwrap();
        map.register_slot(2, "y".to_string()).unwrap();
        map.mark_slot_mutated(2);
        map.register_param_names(vec!["a".to_string(), "b".to_string()])
            .unwrap();
        map.register_param_prelude_count(2).unwrap();
        map.register_const("MAX".to_string(), "100".to_string())
            .unwrap();

        let json = map.to_json().unwrap();
        let parsed = SourceMap::from_json(&json).unwrap();

        assert_eq!(parsed.libs.len(), 2);
        assert_eq!(parsed.lib(1).unwrap().name, "lib.a");
        assert!(parsed.lib(1).unwrap().address.is_some());
        assert_eq!(parsed.lib(2).unwrap().name, "lib.b");
        assert!(parsed.lib(2).unwrap().address.is_none());
        assert_eq!(parsed.func(&[0x01, 0x02, 0x03, 0x04]).unwrap(), "my_func");
        assert_eq!(parsed.slot(0).unwrap(), "x");
        assert_eq!(parsed.slot(2).unwrap(), "y");
        assert!(parsed.slot_is_let(0));
        assert!(parsed.slot_is_var(2));
        assert_eq!(
            parsed.param_names().unwrap(),
            &vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(parsed.param_prelude_count(), Some(2));
        assert_eq!(parsed.get_const_value("MAX").unwrap(), "100");
        assert_eq!(parsed.get_const_name("100").unwrap(), "MAX");
    }

    #[test]
    fn empty_map_roundtrips() {
        let map = SourceMap::default();
        let json = map.to_json().unwrap();
        assert_eq!(json, "{}", "empty map must serialize without padding");
        let parsed = SourceMap::from_json(&json).unwrap();
        assert_eq!(parsed.to_json().unwrap(), json);
    }

    #[test]
    fn to_json_omits_empty_sections() {
        // Typical maincall shape: one lib, one func, one slot, no
        // lets/vars/params/prelude/consts — the omitted padding used to
        // dominate the payload.
        let mut map = SourceMap::default();
        map.register_lib(1, "AMM".to_string(), None).unwrap();
        map.register_func([0xaa, 0xbb, 0xcc, 0xdd], "swap".to_string())
            .unwrap();
        map.register_slot(0, "out".to_string()).unwrap();
        let json = map.to_json().unwrap();
        // register_slot defaults the slot to `let`, so lets travels with it.
        assert_eq!(
            json,
            "{\"libs\":[{\"idx\":1,\"name\":\"AMM\",\"address\":null}],\
             \"funcs\":[{\"sig\":\"aabbccdd\",\"name\":\"swap\"}],\
             \"slots\":[\"out\"],\"lets\":[0]}"
        );
        for absent in ["vars", "params", "param_prelude_count", "consts"] {
            assert!(!json.contains(absent), "{absent} must be omitted: {json}");
        }
        // Roundtrip through the compact form loses nothing.
        let parsed = SourceMap::from_json(&json).unwrap();
        assert_eq!(parsed.to_json().unwrap(), json);
        assert_eq!(parsed.lib(1).unwrap().name, "AMM");
        assert_eq!(parsed.slot(0).unwrap(), "out");
    }

    #[test]
    fn from_json_accepts_legacy_full_shape() {
        // Pre-optimization producers always emitted every section, including
        // the empty ones; those payloads must keep parsing identically.
        let legacy = r#"{"libs":[],"funcs":[],"slots":[],"lets":[],"vars":[],"params":[],"param_prelude_count":null,"consts":[]}"#;
        let parsed = SourceMap::from_json(legacy).unwrap();
        assert_eq!(parsed.to_json().unwrap(), "{}");

        let legacy_partial = r#"{"libs":[{"idx":1,"name":"AMM","address":null}],"funcs":[],"slots":["out"],"lets":[],"vars":[0],"params":[],"param_prelude_count":null,"consts":[]}"#;
        let parsed = SourceMap::from_json(legacy_partial).unwrap();
        assert_eq!(parsed.slot(0).unwrap(), "out");
        assert!(parsed.slot_is_var(0));
        assert_eq!(
            parsed.to_json().unwrap(),
            "{\"libs\":[{\"idx\":1,\"name\":\"AMM\",\"address\":null}],\"slots\":[\"out\"],\"vars\":[0]}"
        );
    }

    #[test]
    fn from_json_rejects_hostile_payloads() {
        // Unknown field.
        assert!(SourceMap::from_json(r#"{"evil":1}"#).is_err());
        // Truncated / garbage.
        assert!(SourceMap::from_json("{").is_err());
        assert!(SourceMap::from_json("").is_err());
        assert!(SourceMap::from_json("null").is_err());
        // Deep nesting is rejected by the shared JSON engine depth cap, not by
        // recursion.
        let deep = format!("{}{}", "[[".repeat(200), "]]".repeat(200));
        assert!(SourceMap::from_json(&format!("{{\"slots\":{}}}", deep)).is_err());
        // Duplicate lib index inside one array is a hard error (no silent override).
        assert!(SourceMap::from_json(
            r#"{"libs":[{"idx":1,"name":"a","address":null},{"idx":1,"name":"b","address":null}]}"#
        )
        .is_err());
        // Bad selector width / non-hex.
        assert!(SourceMap::from_json(r#"{"funcs":[{"sig":"zz","name":"f"}]}"#).is_err());
        assert!(SourceMap::from_json(r#"{"funcs":[{"sig":"aabbccddee","name":"f"}]}"#).is_err());
        // Slot index beyond u8.
        assert!(SourceMap::from_json(
            &format!("{{\"slots\":[{}]}}", vec!["\"x\""; 300].join(","))
        )
        .is_err());
        // lets item not a number / out of range.
        assert!(SourceMap::from_json(r#"{"lets":[300]}"#).is_err());
        assert!(SourceMap::from_json(r#"{"lets":["a"]}"#).is_err());
        // param_prelude_count out of range.
        assert!(SourceMap::from_json(r#"{"param_prelude_count":256}"#).is_err());
    }

    #[test]
    fn from_json_uppercase_selector_sig_normalizes() {
        // Wire consumers may send uppercase hex sigs; hex::decode accepts both
        // and to_json always re-emits lowercase, so the canonical form is stable.
        let parsed = SourceMap::from_json(r#"{"funcs":[{"sig":"AABBCCDD","name":"f"}]}"#).unwrap();
        let json = parsed.to_json().unwrap();
        assert!(json.contains("\"sig\":\"aabbccdd\""), "{json}");
        assert_eq!(parsed.func(&[0xaa, 0xbb, 0xcc, 0xdd]).unwrap(), "f");
    }
}
