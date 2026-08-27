use std::collections::HashSet;

use sys::{Rerr, Ret, errf, normalf};

use crate::codec::{Decode, Encode};
use crate::types::fixed::{Fixed6, Fixed10};
use crate::types::fold64::Fold64;
use crate::types::list::{ListW1, ListW2};
use crate::types::uint::Uint3;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiamondName(Fixed6);
pub type DiamondNumber = Uint3;
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiamondNumberAuto(Fold64);
pub type DiamondVisualGene = Fixed10;
pub type DiamondLifeGene = Fixed10;

/// Semantic cap of `DiamondNameListMax200` (u8-counted wire form, so a semantic rule).
/// Single source — this module's checks and the SDK's codec profile both read it.
pub const DIAMOND_LIST_MAX: usize = 200;

impl DiamondNumberAuto {
    pub fn uint(&self) -> u64 {
        self.0.uint()
    }

    pub fn from_diamond(diamond: &DiamondNumber) -> Self {
        Self(Fold64::from(diamond.uint() as u64).expect("DiamondNumber fits Fold64"))
    }

    pub fn to_diamond(&self) -> Ret<DiamondNumber> {
        if self.0.uint() > DiamondNumber::MAX as u64 {
            return errf!("diamond number {} exceeds max", self.0.uint());
        }
        Ok(DiamondNumber::from(self.0.uint() as u32))
    }
}

impl Encode for DiamondNumberAuto {
    fn size(&self) -> usize {
        self.0.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.0.encode_to(out);
    }
}

impl Decode for DiamondNumberAuto {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (value, used) = Fold64::decode(buf)?;
        let auto = Self(value);
        auto.to_diamond()?;
        Ok((auto, used))
    }
}

impl DiamondName {
    pub const SIZE: usize = Fixed6::SIZE;

    pub const fn from(value: [u8; Self::SIZE]) -> Self {
        Self(Fixed6::from(value))
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn is_valid(stuff: &[u8]) -> bool {
        const DIAMOND_NAME_VALID_CHARS: [u8; 16] = *b"WTYUIAHXVMEKBSZN";
        stuff.len() == Self::SIZE && stuff.iter().all(|&x| DIAMOND_NAME_VALID_CHARS.contains(&x))
    }

    pub fn check_bytes(stuff: &[u8]) -> Rerr {
        if Self::is_valid(stuff) {
            return Ok(());
        }
        errf!(
            "diamond name {} is not valid",
            String::from_utf8_lossy(stuff)
        )
    }

    pub fn from_readable(stuff: impl AsRef<[u8]>) -> Ret<Self> {
        let raw = stuff.as_ref();
        Self::check_bytes(raw)?;
        let mut out = [0u8; Self::SIZE];
        out.copy_from_slice(raw);
        Ok(Self::from(out))
    }

    pub fn to_readable(&self) -> String {
        String::from_utf8_lossy(self.as_ref()).to_string()
    }
}

impl From<[u8; DiamondName::SIZE]> for DiamondName {
    fn from(value: [u8; DiamondName::SIZE]) -> Self {
        Self::from(value)
    }
}

impl From<Fixed6> for DiamondName {
    fn from(value: Fixed6) -> Self {
        Self(value)
    }
}

impl AsRef<[u8]> for DiamondName {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Encode for DiamondName {
    fn size(&self) -> usize {
        self.0.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.0.encode_to(out);
    }
}

impl Decode for DiamondName {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (fixed, used) = Fixed6::decode(buf)?;
        let name = Self(fixed);
        if !Self::is_valid(name.as_ref()) {
            return normalf!("diamond name {} is not valid", name.to_readable());
        }
        Ok((name, used))
    }
}

/// Diamond name list with the legacy CSV-string JSON contract and the full
/// format-checking/formatting surface (`define_diamond_name_list!` in the
/// legacy codebase), wrapping the generic wire list so the binary layout
/// stays `W1/W2 count + 7n`. `check` runs at wire decode time too — a
/// manually crafted duplicate/invalid payload never enters the graph.
macro_rules! define_diamond_name_list {
    ($class:ident, $inner:ident, $max:expr) => {
        #[derive(Clone, Default, PartialEq, Eq)]
        pub struct $class($inner<DiamondName>);

        impl std::fmt::Debug for $class {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "[list {}]", self.length())
            }
        }

        impl<'a> IntoIterator for &'a $class {
            type Item = &'a DiamondName;
            type IntoIter = std::slice::Iter<'a, DiamondName>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }

        impl Encode for $class {
            fn size(&self) -> usize {
                self.0.size()
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                self.0.encode_to(out);
            }
        }

        impl Decode for $class {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let (list, used) = <$inner<DiamondName>>::decode(buf)?;
                let value = Self(list);
                value.check()?;
                Ok((value, used))
            }
        }

        impl $class {
            pub fn from(v: Vec<DiamondName>) -> Ret<Self> {
                let value = Self($inner::from(v)?);
                value.check()?;
                Ok(value)
            }

            pub fn one(dia: DiamondName) -> Ret<Self> {
                Self::from(vec![dia])
            }

            pub fn length(&self) -> usize {
                self.0.len()
            }

            pub fn as_list(&self) -> &Vec<DiamondName> {
                &self.0.0
            }

            pub fn into_vec(self) -> Vec<DiamondName> {
                self.0.0
            }

            pub fn push(&mut self, v: DiamondName) -> Rerr {
                DiamondName::check_bytes(v.as_ref())?;
                if self.contains(v.as_ref()) {
                    return errf!("diamond name {} is duplicated", v.to_readable());
                }
                if self.length() + 1 > $max {
                    return errf!("diamond list max {} overflow", $max);
                }
                self.0.push(v)
            }

            pub fn drop(&mut self, i: usize) -> Rerr {
                self.0.drop(i).map(|_| ())
            }

            pub fn replace(&mut self, i: usize, v: DiamondName) -> Rerr {
                if i >= self.length() {
                    return errf!("list index overflow");
                }
                DiamondName::check_bytes(v.as_ref())?;
                if self
                    .0
                    .0
                    .iter()
                    .enumerate()
                    .any(|(j, name)| j != i && name.as_ref() == v.as_ref())
                {
                    return errf!("diamond name {} is duplicated", v.to_readable());
                }
                self.0.0[i] = v;
                Ok(())
            }

            pub fn append(&mut self, dias: Vec<DiamondName>) -> Rerr {
                if self.length() + dias.len() > $max {
                    return errf!("diamond list max {} overflow", $max);
                }
                let mut seen = HashSet::with_capacity(self.length() + dias.len());
                for name in &self.0.0 {
                    seen.insert(*name);
                }
                for d in &dias {
                    DiamondName::check_bytes(d.as_ref())?;
                    if !seen.insert(*d) {
                        return errf!("diamond name {} is duplicated", d.to_readable());
                    }
                }
                self.0.0.extend(dias);
                Ok(())
            }

            pub fn pop(&mut self) -> Option<DiamondName> {
                if self.length() == 0 {
                    return None;
                }
                self.0.0.pop()
            }

            pub fn fetch_list(&mut self, n: usize) -> Ret<Vec<DiamondName>> {
                let m = self.length();
                if n > m {
                    return errf!("list data length is {} but will fetch {}", m, n);
                }
                Ok(self.0.0.split_off(m - n))
            }

            pub fn fetch_head_list(&mut self, n: usize) -> Ret<Vec<DiamondName>> {
                let m = self.length();
                if n > m {
                    return errf!("list data length is {} but will fetch {}", m, n);
                }
                if n == 0 {
                    return Ok(vec![]);
                }
                if n == m {
                    return Ok(std::mem::take(&mut self.0.0));
                }
                // Preserve FIFO extraction without repeated front-removes.
                let mut head = std::mem::take(&mut self.0.0);
                let tail = head.split_off(n);
                self.0.0 = tail;
                Ok(head)
            }

            pub fn check(&self) -> Ret<usize> {
                // The legacy list kept a stored count checked against `len()`;
                // here the inner `ListW1` derives its length from the vec, so
                // that invariant is structurally guaranteed.
                let reallen = self.length();
                if reallen == 0 {
                    return errf!("diamonds quantity cannot be zero");
                }
                if reallen > $max {
                    return errf!("diamonds quantity cannot exceed {}", $max);
                }
                let mut seen = HashSet::with_capacity(reallen);
                for name in &self.0.0 {
                    if !DiamondName::is_valid(name.as_ref()) {
                        return errf!("diamond name {} is not valid", name.to_readable());
                    }
                    if !seen.insert(*name) {
                        return errf!("diamond name {} is duplicated", name.to_readable());
                    }
                }
                Ok(reallen)
            }

            pub fn contains(&self, x: &[u8]) -> bool {
                self.0.0.iter().any(|name| x == name.as_ref())
            }

            pub fn splitstr(&self) -> String {
                self.0
                    .iter()
                    .map(|name| name.to_readable())
                    .collect::<Vec<_>>()
                    .join(",")
            }

            pub fn readable(&self) -> String {
                self.0
                    .iter()
                    .map(|name| name.to_readable())
                    .collect::<Vec<_>>()
                    .concat()
            }

            pub fn form(&self) -> Vec<u8> {
                self.0.0.iter().flat_map(|name| name.encode()).collect()
            }

            pub fn hashset(&self) -> HashSet<DiamondName> {
                self.0.0.iter().copied().collect()
            }

            pub fn from_readable(stuff: &str) -> Ret<Self> {
                let s = stuff
                    .replace(' ', "")
                    .replace('\n', "")
                    .replace('|', "")
                    .replace(',', "");
                if s.is_empty() {
                    return errf!("diamond list is empty");
                }
                if s.len() % DiamondName::SIZE != 0 {
                    return errf!("diamond list format invalid");
                }
                let num = s.len() / DiamondName::SIZE;
                if num > $max {
                    return errf!("diamond list max {} overflow", $max);
                }
                let mut obj = Self::default();
                let bs = s.as_bytes();
                for i in 0..num {
                    let x = i * DiamondName::SIZE;
                    obj.push(DiamondName::from_readable(&bs[x..x + DiamondName::SIZE])?)?;
                }
                obj.check()?;
                Ok(obj)
            }
        }
    };
}

define_diamond_name_list!(DiamondNameListMax200, ListW1, DIAMOND_LIST_MAX);
define_diamond_name_list!(DiamondNameListMax60000, ListW2, 60_000);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diamond_list_rejects_duplicates() {
        // from_readable rejects duplicate diamond names
        assert!(DiamondNameListMax200::from_readable("WTYUIA,WTYUIA").is_err());
        assert!(DiamondNameListMax200::from_readable("WTYUIA,HYXYHY,WTYUIA").is_err());
        // valid: no duplicates
        assert!(DiamondNameListMax200::from_readable("WTYUIA,HYXYHY").is_ok());
        // from() rejects duplicates
        let dup = vec![DiamondName::from(*b"WTYUIA"), DiamondName::from(*b"WTYUIA")];
        assert!(DiamondNameListMax200::from(dup).is_err());

        // check() also rejects duplicates even when the list comes from raw
        // bytes (defense against manually crafted binary payloads).
        let mut raw200 = vec![2u8];
        raw200.extend_from_slice(b"WTYUIA");
        raw200.extend_from_slice(b"WTYUIA");
        assert!(DiamondNameListMax200::decode(&raw200).is_err());

        let mut raw60000 = vec![0u8, 2u8];
        raw60000.extend_from_slice(b"WTYUIA");
        raw60000.extend_from_slice(b"WTYUIA");
        assert!(DiamondNameListMax60000::decode(&raw60000).is_err());
    }

    #[test]
    fn diamond_list_fetch_head_keeps_fifo_order() {
        let mut list =
            DiamondNameListMax60000::from_readable("WTYUIA,HYXYHY,UETWNK,WYUKKZ").unwrap();
        let head = DiamondNameListMax200::from(list.fetch_head_list(2).unwrap()).unwrap();
        assert_eq!(head.readable(), "WTYUIAHYXYHY");
        assert_eq!(list.readable(), "UETWNKWYUKKZ");
    }

    #[test]
    fn diamond_list_from_readable_accepts_separators() {
        // comma-separated, space-separated and concatenated forms are equal
        let a = DiamondNameListMax200::from_readable("WTYUIA,HYXYHY").unwrap();
        let b = DiamondNameListMax200::from_readable("WTYUIA HYXYHY").unwrap();
        let c = DiamondNameListMax200::from_readable("WTYUIA|HYXYHY").unwrap();
        let d = DiamondNameListMax200::from_readable("WTYUIAHYXYHY").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a, d);
        assert!(DiamondNameListMax200::from_readable("").is_err());
        assert!(DiamondNameListMax200::from_readable("WTYUI").is_err());
    }

    #[test]
    fn diamond_list_wire_roundtrip() {
        let list = DiamondNameListMax200::from_readable("WTYUIA,HYXYHY,UETWNK").unwrap();
        let mut buf = Vec::new();
        list.encode_to(&mut buf);
        let (back, used) = DiamondNameListMax200::decode(&buf).unwrap();
        assert_eq!(used, buf.len());
        assert_eq!(back, list);
        assert_eq!(back.splitstr(), "WTYUIA,HYXYHY,UETWNK");
    }

    #[test]
    fn diamond_list_one_and_push_validate() {
        let one = DiamondNameListMax200::one(DiamondName::from(*b"WTYUIA")).unwrap();
        assert_eq!(one.length(), 1);
        assert!(one.check().is_ok());
        assert!(DiamondNameListMax200::one(DiamondName::from(*b"xxxxxx")).is_err());

        let mut list = DiamondNameListMax200::one(DiamondName::from(*b"WTYUIA")).unwrap();
        assert!(list.push(DiamondName::from(*b"WTYUIA")).is_err());
        assert!(list.push(DiamondName::from(*b"xxxxxx")).is_err());
        assert!(list.push(DiamondName::from(*b"HYXYHY")).is_ok());
        assert_eq!(list.length(), 2);
    }

    #[test]
    fn diamond_list_replace_and_append_validate() {
        let mut list = DiamondNameListMax200::one(DiamondName::from(*b"WTYUIA")).unwrap();
        assert!(list.replace(0, DiamondName::from(*b"xxxxxx")).is_err());
        assert!(list.replace(0, DiamondName::from(*b"HYXYHY")).is_ok());
        assert_eq!(list.readable(), "HYXYHY");
        assert!(list.append(vec![DiamondName::from(*b"HYXYHY")]).is_err());
        assert!(list.append(vec![DiamondName::from(*b"WTYUIA")]).is_ok());
        assert!(list.replace(0, DiamondName::from(*b"WTYUIA")).is_err());
        assert!(list.replace(1, DiamondName::from(*b"WTYUIA")).is_ok());
        assert!(
            list.append(vec![
                DiamondName::from(*b"UETWNK"),
                DiamondName::from(*b"UETWNK"),
            ])
            .is_err()
        );
        assert_eq!(list.length(), 2);
    }

    #[test]
    fn diamond_list_drain_may_become_empty() {
        let mut list = DiamondNameListMax200::one(DiamondName::from(*b"WTYUIA")).unwrap();
        assert!(list.pop().is_some());
        assert_eq!(list.length(), 0);
        assert!(list.check().is_err());
        let mut list = DiamondNameListMax200::from_readable("WTYUIA,HYXYHY").unwrap();
        assert_eq!(list.fetch_list(2).unwrap().len(), 2);
        assert_eq!(list.length(), 0);
    }
}
