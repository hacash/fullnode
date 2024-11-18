
// block
pub type BlockHeight = Uint5;
pub type Timestamp = Uint5;


// balance
pub type Satoshi = Uint8;


// diamond
pub type DiamondNumber = Uint3;
pub type DiamondVisualGene = Fixed10;
pub type DiamondLifeGene   = Fixed32;


// key
pub type ChannelId = Fixed16;

// common
pub type Hash = Fixed32;
pub type HashHalf = Fixed16;
pub type HashNonce = Fixed8;
pub type HashCheck = Fixed4;
pub type HashMark = Fixed2;

impl Hash {

    pub fn half(&self) -> HashHalf {
        let pt: [u8; HashHalf::SIZE] = self.bytes[0..HashHalf::SIZE].try_into().unwrap();
        HashHalf::must(&pt)
    }

    pub fn nonce(&self) -> HashNonce {
        let pt: [u8; HashNonce::SIZE] = self.bytes[0..HashNonce::SIZE].try_into().unwrap();
        HashNonce::must(&pt)
    }

    pub fn check(&self) -> HashCheck {
        let pt: [u8; HashCheck::SIZE] = self.bytes[0..HashCheck::SIZE].try_into().unwrap();
        HashCheck::must(&pt)
    }

    pub fn mark(&self) -> HashMark {
        let pt: [u8; HashMark::SIZE] = self.bytes[0..HashMark::SIZE].try_into().unwrap();
        HashMark::must(&pt)
    }

    // value +1 bigend
    pub fn increase(&mut self) {
        const S128: usize = 16;
        let right: [u8; S128] = self.bytes[S128..].try_into().unwrap();
        let mut rnum: u128 = u128::from_be_bytes(right);
        if rnum == u128::MAX {
            let left: [u8; S128] = self.bytes[0..S128].try_into().unwrap();
            let mut lnum: u128 = u128::from_be_bytes(left);
            lnum += 1;
            self.bytes[0..S128].copy_from_slice(&lnum.to_be_bytes());
        }
        // yes
        rnum += 1;
        self.bytes[S128..].copy_from_slice(&rnum.to_be_bytes());
    }

}
