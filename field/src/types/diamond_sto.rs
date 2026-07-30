use sys::{Ret, errf};

use crate::codec::{Decode, Encode, Reader};
use crate::types::address::Address;
use crate::types::amount::Amount;
use crate::types::bytes_w::{BytesW1, BytesW4};
use crate::types::diamond::{DiamondName, DiamondNameListMax200, DiamondNumber};
use crate::types::fixed::{Fixed8, Hash};
use crate::types::list::ListW1;
use crate::types::uint::{BlockHeight, Uint1, Uint2};

codec_struct!(DiamondInscript {
    engraved_type: Uint1,
    content: BytesW1,
});

impl DiamondInscript {
    pub fn to_readable_or_hex(&self) -> String {
        self.content.to_readable_or_hex()
    }
}

pub type Inscripts = ListW1<DiamondInscript>;

impl Inscripts {
    pub fn array(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|item| item.to_readable_or_hex())
            .collect()
    }
}

codec_struct!(DiamondSto {
    status: Uint1,
    address: Address,
    prev_engraved_height: BlockHeight,
    inscripts: Inscripts,
});

codec_struct!(DiamondSmelt {
    diamond: DiamondName,
    number: DiamondNumber,
    born_height: BlockHeight,
    born_hash: Hash,
    prev_hash: Hash,
    miner_address: Address,
    bid_fee: Amount,
    nonce: Fixed8,
    average_bid_burn: Uint2,
    life_gene: Hash,
});

codec_struct!(DiamondOwnedForm { names: BytesW4 });

impl DiamondOwnedForm {
    fn contains_diamond(&self, dian: &DiamondName) -> bool {
        let names = self.names.as_ref();
        names.len() % DiamondName::SIZE == 0
            && names
                .chunks_exact(DiamondName::SIZE)
                .any(|name| name == dian.as_ref())
    }

    pub fn readable(&self) -> String {
        String::from_utf8_lossy(self.names.as_ref()).to_string()
    }

    pub fn push_one(&mut self, dian: &DiamondName) -> Ret<()> {
        if self.contains_diamond(dian) {
            return Ok(());
        }
        let mut bytes = self.names.to_vec();
        bytes.extend_from_slice(dian.as_ref());
        self.names = BytesW4::from(bytes)?;
        Ok(())
    }

    pub fn push(&mut self, dian: &DiamondNameListMax200) -> Ret<()> {
        for name in dian.as_list() {
            self.push_one(name)?;
        }
        Ok(())
    }

    pub fn drop_one(&mut self, dian: &DiamondName) -> Ret<usize> {
        let list = DiamondNameListMax200::from(vec![*dian])?;
        self.drop(&list)
    }

    pub fn drop(&mut self, dian: &DiamondNameListMax200) -> Ret<usize> {
        let mut form = std::mem::take(&mut self.names).to_vec();
        let form_len = form.len();
        if form_len % DiamondName::SIZE != 0 {
            self.names = BytesW4::from(form)?;
            return errf!("DiamondOwnedForm names length invalid");
        }
        let mut length = form.len() / DiamondName::SIZE;
        let mut index = 0;
        let mut removed = 0;
        let mut pending: std::collections::HashSet<_> = dian.as_list().iter().copied().collect();
        while index < length {
            let offset = index * DiamondName::SIZE;
            let name = DiamondName::from(
                form[offset..offset + DiamondName::SIZE]
                    .try_into()
                    .expect("diamond name has fixed width"),
            );
            if pending.remove(&name) {
                removed += 1;
                length -= 1;
                if index < length {
                    form.copy_within(
                        length * DiamondName::SIZE..(length + 1) * DiamondName::SIZE,
                        offset,
                    );
                }
                if pending.is_empty() {
                    break;
                }
            } else {
                index += 1;
            }
        }
        if !pending.is_empty() {
            self.names = BytesW4::from(form)?;
            return errf!("diamond names not found in owned form");
        }
        form.truncate(length * DiamondName::SIZE);
        self.names = BytesW4::from(form)?;
        Ok(removed)
    }
}
