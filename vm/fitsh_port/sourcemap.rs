use std::collections::{HashMap, HashSet};

use hex;
use serde::{Deserialize, Serialize};
use serde_json;


#[derive(Serialize, Deserialize)]
struct SourceMapJson {
    libs: Vec<LibJson>,
    funcs: Vec<FuncJson>,
    slots: Vec<String>,
    lets: Vec<u8>,
    vars: Vec<u8>,
    params: Vec<String>,
    #[serde(default)]
    param_prelude_count: Option<u8>,
    #[serde(default)]
    consts: Vec<ConstJson>,
}

#[derive(Serialize, Deserialize)]
struct ConstJson {
    name: String,
    value: String,
}

#[derive(Serialize, Deserialize)]
struct LibJson {
    idx: u8,
    name: String,
    address: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct FuncJson {
    sig: String,
    name: String,
}

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

    /// Register a local slot name for decompilation.
    /// Slots are pessimistically marked as `let` and promoted to `var`
    /// only when `mark_slot_mutated` is observed during parsing.
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
        let mut libs: Vec<(u8, LibInfo)> = self.libs.iter().map(|(&idx, info)| (idx, info.clone())).collect();
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

    pub fn to_json(&self) -> Ret<String> {
        let mut libs: Vec<LibJson> = self.libs.iter().map(|(&idx, info)| LibJson {
            idx,
            name: info.name.clone(),
            address: info.address.as_ref().map(|a| a.to_readable()),
        }).collect();
        libs.sort_by_key(|entry| entry.idx);

        let mut funcs: Vec<FuncJson> = self.funcs.iter().map(|(sig, name)| FuncJson {
            sig: hex::encode(sig),
            name: name.clone(),
        }).collect();
        funcs.sort_by(|a, b| a.sig.cmp(&b.sig));

        // slots
        let max_slot = self.slots.keys().max().copied().unwrap_or(0);
        let mut slots = Vec::new();
        if !self.slots.is_empty() {
             for i in 0..=max_slot {
                if let Some(name) = self.slots.get(&i) {
                    slots.push(name.clone());
                } else {
                    slots.push(String::new());
                }
             }
        }

        let mut lets: Vec<u8> = self.lets.iter().copied().collect();
        lets.sort_unstable();
        let mut vars: Vec<u8> = self.vars.iter().copied().collect();
        vars.sort_unstable();

        let mut consts: Vec<ConstJson> = self.const_name_to_val.iter().map(|(name, value)| ConstJson {
            name: name.clone(),
            value: value.clone(),
        }).collect();
        consts.sort_by(|a, b| a.name.cmp(&b.name));

        let doc = SourceMapJson {
            libs,
            funcs,
            slots,
            lets,
            vars,
            params: self.params.clone(),
            param_prelude_count: self.param_prelude_count,
            consts,
        };
        serde_json::to_string(&doc).map_err(|_| s!("source map serialize failed"))
    }

    pub fn from_json(text: &str) -> Ret<Self> {
        let doc: SourceMapJson = serde_json::from_str(text).map_err(|_| s!("source map deserialize failed"))?;
        let mut map = SourceMap::default();
        for lib in doc.libs {
            let address = match lib.address {
                Some(addr) => Some(Address::from_readable(&addr).map_err(|_| s!("address parse failed"))?),
                None => None,
            };
            map.register_lib(lib.idx, lib.name, address)?;
        }
        for func in doc.funcs {
            let bytes = hex::decode(func.sig).map_err(|_| s!("function signature decode failed"))?;
            if bytes.len() != 4 {
                return errf!("function signature length invalid")
            }
            let mut sig = [0u8; 4];
            sig.copy_from_slice(&bytes);
            map.register_func(sig, func.name)?;
        }
        for cnst in doc.consts {
            map.register_const(cnst.name, cnst.value)?;
        }

        map.lets.clear();
        map.vars.clear();
        for slot in doc.lets {
            map.lets.insert(slot);
        }
        for slot in doc.vars {
            map.vars.insert(slot);
            map.lets.remove(&slot);
        }

        for (i, name) in doc.slots.into_iter().enumerate() {
            if name.is_empty() { continue; }
            let slot = i as u8;
            map.slots.insert(slot, name);
        }
        
        map.params = doc.params;
        map.param_prelude_count = doc.param_prelude_count;
        Ok(map)
    }
}

