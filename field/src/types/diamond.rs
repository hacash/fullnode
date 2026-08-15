use sys::{Rerr, Ret, normalf, errf};

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
pub type DiamondNameListMax200 = ListW1<DiamondName>;
pub type DiamondNameListMax60000 = ListW2<DiamondName>;

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

impl DiamondNameListMax200 {
    pub fn one(dia: DiamondName) -> Self {
        Self(vec![dia])
    }

    pub fn check(&self) -> Ret<usize> {
        if self.0.is_empty() {
            return errf!("diamonds quantity cannot be zero");
        }
        if self.0.len() > 200 {
            return errf!("diamonds quantity cannot exceed 200");
        }
        let mut seen = std::collections::HashSet::with_capacity(self.0.len());
        for name in &self.0 {
            DiamondName::check_bytes(name.as_ref())?;
            if !seen.insert(*name) {
                return errf!(
                    "diamond name {} is duplicated",
                    String::from_utf8_lossy(name.as_ref())
                );
            }
        }
        Ok(self.0.len())
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

    pub fn from_readable(stuff: &str) -> Ret<Self> {
        let list = Self::from(parse_diamond_name_list(stuff, 200)?)?;
        list.check()?;
        Ok(list)
    }
}

impl DiamondNameListMax60000 {
    pub fn check(&self) -> Ret<usize> {
        if self.0.is_empty() {
            return errf!("diamonds quantity cannot be zero");
        }
        if self.0.len() > 60_000 {
            return errf!("diamonds quantity cannot exceed 60000");
        }
        let mut seen = std::collections::HashSet::with_capacity(self.0.len());
        for name in &self.0 {
            DiamondName::check_bytes(name.as_ref())?;
            if !seen.insert(*name) {
                return errf!("diamond name {} is duplicated", name.to_readable());
            }
        }
        Ok(self.0.len())
    }

    pub fn from_readable(stuff: &str) -> Ret<Self> {
        let list = Self::from(parse_diamond_name_list(stuff, 60_000)?)?;
        list.check()?;
        Ok(list)
    }
}

fn parse_diamond_name_list(stuff: &str, max: usize) -> Ret<Vec<DiamondName>> {
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
    if num > max {
        return errf!("diamond list max {} overflow", max);
    }
    let mut out = Vec::with_capacity(num);
    let mut seen = std::collections::HashSet::with_capacity(num);
    for chunk in s.as_bytes().chunks_exact(DiamondName::SIZE) {
        let dia = DiamondName::from_readable(chunk)?;
        if !seen.insert(dia) {
            return errf!("diamond name {} is duplicated", dia.to_readable());
        }
        out.push(dia);
    }
    Ok(out)
}
