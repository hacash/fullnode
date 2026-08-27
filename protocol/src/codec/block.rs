use std::any::Any;
use std::sync::Arc;

use base::{BinaryCodecs, Block, BlockBuild, BlockHasherFn, PowBlock, PowBlockBuild, TxRef};
use field::{BlockHeight, Encode, Fixed2, Hash, Reader, Timestamp, Uint1, Uint4};
use sys::{Rerr, Ret};

#[derive(Debug, Clone)]
pub struct BlockV1 {
    pub version: Uint1,
    pub height: BlockHeight,
    pub timestamp: Timestamp,
    pub prevhash: Hash,
    pub mrklroot: Hash,
    transaction_count: Uint4,
    pub nonce: Uint4,
    pub difficulty: Uint4,
    pub witness_stage: Fixed2,
    pub transactions: Vec<TxRef>,
    hasher: BlockHasherFn,
}

pub type StdBlock = BlockV1;

impl BlockV1 {
    pub const VERSION: u8 = 1;
    /// Fixed header size: version + height + timestamp + prevhash + mrklroot
    /// + transaction_count + nonce + difficulty + witness_stage.
    pub const INTRO_SIZE: usize = Uint1::SIZE
        + BlockHeight::SIZE
        + Timestamp::SIZE
        + Hash::SIZE
        + Hash::SIZE
        + Uint4::SIZE
        + Uint4::SIZE
        + Uint4::SIZE
        + Fixed2::SIZE;

    pub fn new(hasher: BlockHasherFn) -> Self {
        Self {
            version: Uint1::from(Self::VERSION),
            height: BlockHeight::from(0),
            timestamp: Timestamp::default(),
            prevhash: Hash::default(),
            mrklroot: Hash::default(),
            transaction_count: Uint4::from(0),
            nonce: Uint4::from(0),
            difficulty: Uint4::from(0),
            witness_stage: Fixed2::default(),
            transactions: Vec::new(),
            hasher,
        }
    }

    /// Intro-header transaction count (wire field). Distinct from
    /// [`Block::transaction_count`](base::Block::transaction_count), which is
    /// `transactions().len()`.
    pub fn header_transaction_count(&self) -> Uint4 {
        self.transaction_count
    }

    pub fn genesis(hasher: BlockHasherFn) -> Self {
        let mut blk = Self::new(hasher);
        blk.difficulty = Uint4::from(1);
        blk
    }

    fn intro_size(&self) -> usize {
        Self::INTRO_SIZE
    }

    fn encode_intro_to(&self, out: &mut Vec<u8>) {
        self.version.encode_to(out);
        self.height.encode_to(out);
        self.timestamp.encode_to(out);
        self.prevhash.encode_to(out);
        self.mrklroot.encode_to(out);
        self.transaction_count.encode_to(out);
        self.nonce.encode_to(out);
        self.difficulty.encode_to(out);
        self.witness_stage.encode_to(out);
    }

    pub fn encode_intro(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.intro_size());
        self.encode_intro_to(&mut out);
        out
    }

    fn read_intro(
        buf: &[u8],
    ) -> Ret<(
        Uint1,
        BlockHeight,
        Timestamp,
        Hash,
        Hash,
        Uint4,
        Uint4,
        Uint4,
        Fixed2,
        usize,
    )> {
        let mut r = Reader::new(buf);
        let version: Uint1 = r.read()?;
        if version.uint() != Self::VERSION {
            return sys::normalf!("block version {} not supported", version.uint());
        }
        let height: BlockHeight = r.read()?;
        let timestamp: Timestamp = r.read()?;
        let prevhash: Hash = r.read()?;
        let mrklroot: Hash = r.read()?;
        let transaction_count: Uint4 = r.read()?;
        let nonce: Uint4 = r.read()?;
        let difficulty: Uint4 = r.read()?;
        let witness_stage: Fixed2 = r.read()?;
        Ok((
            version,
            height,
            timestamp,
            prevhash,
            mrklroot,
            transaction_count,
            nonce,
            difficulty,
            witness_stage,
            r.used(),
        ))
    }

    pub fn decode_intro(hasher: BlockHasherFn, buf: &[u8]) -> Ret<Self> {
        let (
            version,
            height,
            timestamp,
            prevhash,
            mrklroot,
            transaction_count,
            nonce,
            difficulty,
            witness_stage,
            used,
        ) = Self::read_intro(buf)?;
        if used != buf.len() {
            return sys::normalf!(
                "block intro length mismatch: consumed {} but payload length is {}",
                used,
                buf.len()
            );
        }
        Ok(Self {
            version,
            height,
            timestamp,
            prevhash,
            mrklroot,
            transaction_count,
            nonce,
            difficulty,
            witness_stage,
            transactions: Vec::new(),
            hasher,
        })
    }

    fn sync_transaction_count(&mut self) -> Rerr {
        self.transaction_count = Uint4::from_usize(self.transactions.len())?;
        Ok(())
    }
}

impl Encode for BlockV1 {
    fn size(&self) -> usize {
        self.intro_size() + self.transactions.iter().map(|t| t.size()).sum::<usize>()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        let actual =
            Uint4::from_usize(self.transactions.len()).expect("block transaction count overflow");
        assert_eq!(
            self.header_transaction_count(),
            actual,
            "block transaction_count {} != transactions.len() {}",
            self.header_transaction_count().uint(),
            self.transactions.len()
        );
        self.encode_intro_to(out);
        for tx in &self.transactions {
            tx.encode_to(out);
        }
    }
}

impl Block for BlockV1 {
    fn version(&self) -> u8 {
        self.version.uint()
    }

    fn height(&self) -> u64 {
        self.height.uint()
    }

    fn hash(&self) -> Hash {
        let mut intro = Vec::with_capacity(self.intro_size());
        self.encode_intro_to(&mut intro);
        Hash::from((self.hasher)(self.height(), &intro))
    }

    fn prev_hash(&self) -> Hash {
        self.prevhash
    }

    fn mrklroot(&self) -> Hash {
        self.mrklroot
    }

    fn timestamp(&self) -> u64 {
        self.timestamp.value()
    }

    fn as_pow(&self) -> Option<&dyn PowBlock> {
        Some(self)
    }

    fn transactions(&self) -> &[TxRef] {
        &self.transactions
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BlockBuild for BlockV1 {
    fn update_mrklroot(&mut self) {
        let hashes = self.transaction_hash_list(true);
        self.mrklroot = calculate_mrklroot(&hashes);
    }

    fn set_mrklroot(&mut self, root: Hash) {
        self.mrklroot = root;
    }

    fn replace_transaction(&mut self, idx: usize, tx: TxRef) -> Rerr {
        if idx >= self.transactions.len() {
            return sys::errf!("transaction index {} out of range", idx);
        }
        self.transactions[idx] = tx;
        self.update_mrklroot();
        Ok(())
    }

    fn push_transaction(&mut self, tx: TxRef) -> Rerr {
        self.transactions.push(tx);
        self.sync_transaction_count()?;
        Ok(())
    }
}

impl PowBlock for BlockV1 {
    fn nonce(&self) -> u32 {
        self.nonce.uint()
    }

    fn difficulty(&self) -> u32 {
        self.difficulty.uint()
    }
}

impl PowBlockBuild for BlockV1 {
    fn set_nonce(&mut self, nonce: u32) {
        self.nonce = Uint4::from(nonce);
    }
}

fn mrkl_merge(list: &[Hash]) -> Vec<Hash> {
    let mut res = Vec::with_capacity((list.len() + 1) / 2);
    let mut i = 0usize;
    while i < list.len() {
        let lh = &list[i];
        let rh = if i + 1 < list.len() { &list[i + 1] } else { lh };
        let mut pair = Vec::with_capacity(lh.size() + rh.size());
        pair.extend_from_slice(lh.as_ref());
        pair.extend_from_slice(rh.as_ref());
        res.push(Hash::from(sys::calculate_hash(pair)));
        i += 2;
    }
    res
}

pub fn calculate_mrklroot(list: &[Hash]) -> Hash {
    if list.is_empty() {
        return Hash::default();
    }
    let mut layer = list.to_vec();
    while layer.len() > 1 {
        layer = mrkl_merge(&layer);
    }
    layer[0]
}

pub fn calculate_mrkl_prelude_modify(list: &[Hash]) -> Vec<Hash> {
    assert!(!list.is_empty(), "merkle prelude list is empty");
    if list.len() == 1 {
        return Vec::new();
    }
    if list.len() == 2 {
        return vec![list[1]];
    }
    let mut out = Vec::new();
    let mut layer = list.to_vec();
    while layer.len() > 1 {
        if layer.len() >= 2 {
            out.push(layer[1]);
        }
        layer = mrkl_merge(&layer);
    }
    out
}

pub fn calculate_mrkl_prelude_update(cbhx: Hash, list: &[Hash]) -> Hash {
    let mut res = cbhx;
    for hx in list {
        let mut pair = Vec::with_capacity(res.size() + hx.size());
        pair.extend_from_slice(res.as_ref());
        pair.extend_from_slice(hx.as_ref());
        res = Hash::from(sys::calculate_hash(pair));
    }
    res
}

pub fn create_std_block(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(base::BlockRef, usize)> {
    let (
        version,
        height,
        timestamp,
        prevhash,
        mrklroot,
        transaction_count,
        nonce,
        difficulty,
        witness_stage,
        used,
    ) = BlockV1::read_intro(buf)?;
    let mut r = Reader::new(buf);
    r.skip(used)?;

    // Do not preallocate from the untrusted wire count: a malformed `u32::MAX` count
    // must fail during bounded decoding, not allocate before the consistency check.
    let mut transactions = Vec::new();
    for _ in 0..transaction_count.uint() {
        let tx = r.read_with(|rest| reg.decode_transaction(rest))?;
        transactions.push(tx);
    }

    Ok((
        Arc::new(BlockV1 {
            version,
            height,
            timestamp,
            prevhash,
            mrklroot,
            transaction_count,
            nonce,
            difficulty,
            witness_stage,
            transactions,
            hasher: reg.block_hasher_fn(),
        }),
        r.used(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::action::TransferHacTo;
    use crate::codec::test_reg::{TestRegistry, hasher};
    use crate::codec::tx::StdTransaction;
    use base::{BlockBuild, TransactionBuild};
    use field::{Address, Amount, Encode};
    use std::sync::Arc;

    #[test]
    fn intro_size_is_fixed_and_decode_paths_share_read_intro() {
        assert_eq!(BlockV1::INTRO_SIZE, 89);
        let block = BlockV1::new(hasher());
        let intro = block.encode_intro();
        assert_eq!(intro.len(), BlockV1::INTRO_SIZE);
        assert_eq!(block.size(), intro.len());

        let decoded = BlockV1::decode_intro(hasher(), &intro).unwrap();
        assert_eq!(decoded.header_transaction_count().uint(), 0);
        assert!(decoded.transactions.is_empty());
        assert_eq!(decoded.encode_intro(), intro);

        let mut longer = intro.clone();
        longer.extend_from_slice(&[0xaa]);
        assert!(BlockV1::decode_intro(hasher(), &longer).is_err());
        let (fields_used, _) = {
            let (_, _, _, _, _, _, _, _, _, used) = BlockV1::read_intro(&longer).unwrap();
            (used, ())
        };
        assert_eq!(fields_used, BlockV1::INTRO_SIZE);

        let mut wrong = intro.clone();
        wrong[0] = 2;
        assert!(BlockV1::decode_intro(hasher(), &wrong).is_err());
        let reg = TestRegistry::protocol().unwrap();
        assert!(create_std_block(&reg, &wrong).is_err());
    }

    #[test]
    fn full_block_roundtrip_size_matches_encode() {
        let reg = TestRegistry::protocol().unwrap();
        let mut block = BlockV1::new(hasher());
        let mut tx =
            StdTransaction::new(hacash_params::TX_TYPE_2, Address::default(), Amount::mei(1));
        tx.push_action(Arc::new(TransferHacTo::new(
            Address::default(),
            Amount::mei(1),
        )))
        .unwrap();
        block.push_transaction(Arc::new(tx)).unwrap();
        assert_eq!(block.size(), block.encode().len());
        let wire = block.encode();
        let (decoded, used) = create_std_block(&reg, &wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);
        assert_eq!(decoded.transactions().len(), 1);
    }
}
