use base58check::{FromBase58Check, ToBase58Check};
use sys::{Ret, decodef, errf};

use crate::codec::{Decode, Encode};
use crate::types::fixed::Fixed21;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(Fixed21);

impl Default for Address {
    fn default() -> Self {
        Self(Fixed21::DEFAULT)
    }
}

impl Address {
    pub const SIZE: usize = Fixed21::SIZE;
    pub const VERSION_PRIVAKEY: u8 = 0;
    pub const VERSION_CONTRACT: u8 = 1;
    pub const VERSION_SCRIPTMH: u8 = 5;
    const UNKNOWN_PRIVKEY_TAIL_SIZE: usize = std::mem::size_of::<u32>();
    const UNKNOWN_PRIVKEY_PREFIX_SIZE: usize = Self::SIZE - Self::UNKNOWN_PRIVKEY_TAIL_SIZE;

    pub const fn from(v: [u8; Self::SIZE]) -> Self {
        Self(Fixed21::from(v))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn as_array(&self) -> &[u8; Fixed21::SIZE] {
        self.0.as_array()
    }

    pub fn version(&self) -> u8 {
        self.0.as_array()[0]
    }

    /// Whether this address uses a version supported by the current protocol.
    pub fn is_supported(&self) -> bool {
        matches!(
            self.version(),
            Self::VERSION_PRIVAKEY | Self::VERSION_CONTRACT | Self::VERSION_SCRIPTMH
        )
    }

    pub fn is_privkey(&self) -> bool {
        self.version() == Self::VERSION_PRIVAKEY
    }

    /// System PRIVAKEY address whose 21-byte big-endian value is `< u32::MAX`
    /// (unknown private key; e.g. TEX settlement `ADDRESS_ONEX`).
    pub fn is_privkey_unknown(&self) -> bool {
        if !self.is_privkey() {
            return false;
        }
        let b = self.as_bytes();
        b[..Self::UNKNOWN_PRIVKEY_PREFIX_SIZE]
            .iter()
            .all(|&x| x == 0)
            && u32::from_be_bytes(
                b[Self::UNKNOWN_PRIVKEY_PREFIX_SIZE..]
                    .try_into()
                    .expect("Address private-key tail size is fixed"),
            ) < u32::MAX
    }

    pub fn must_privkey(&self) -> Ret<()> {
        sys::maybe!(
            self.is_privkey(),
            Ok(()),
            errf!("address {} must be PRIVAKEY type", self.to_readable())
        )
    }

    pub fn must_scriptmh(&self) -> Ret<()> {
        sys::maybe!(
            self.is_scriptmh(),
            Ok(()),
            errf!("address {} must be SCRIPTMH type", self.to_readable())
        )
    }

    pub fn is_contract(&self) -> bool {
        self.version() == Self::VERSION_CONTRACT
    }

    pub fn is_scriptmh(&self) -> bool {
        self.version() == Self::VERSION_SCRIPTMH
    }

    pub fn from_readable(addr: &str) -> Ret<Self> {
        let (version, body) = addr
            .from_base58check()
            .map_err(|e| sys::Error::fault(format!("base58check failed: {:?}", e)))?;
        if body.len() != Self::SIZE - 1 {
            return errf!(
                "address body length {} invalid, expected {}",
                body.len(),
                Self::SIZE - 1
            );
        }
        let mut bytes = [0u8; Self::SIZE];
        bytes[0] = version;
        bytes[1..].copy_from_slice(&body);
        let address = Self::from(bytes);
        if !address.is_supported() {
            return errf!("address version {} not supported", address.version());
        }
        Ok(address)
    }

    pub fn to_readable(&self) -> String {
        self.0.as_array()[1..].to_base58check(self.version())
    }
}

impl From<[u8; Address::SIZE]> for Address {
    fn from(v: [u8; Address::SIZE]) -> Self {
        Self::from(v)
    }
}

impl From<Fixed21> for Address {
    fn from(v: Fixed21) -> Self {
        Self(v)
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Encode for Address {
    fn size(&self) -> usize {
        self.0.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.0.encode_to(out);
    }
}

impl Decode for Address {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        if buf.len() < Self::SIZE {
            return decodef!("buffer too short for Address");
        }
        let (fixed, used) = Fixed21::decode(buf)?;
        let address = Self(fixed);
        if !address.is_supported() {
            return decodef!("address version {} invalid", address.version());
        }
        Ok((address, used))
    }
}
