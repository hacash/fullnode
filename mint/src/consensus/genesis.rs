use std::sync::{Arc, LazyLock};

use std::any::Any;

use base::{Block, BlockBuild, BlockRef, PowBlock, TxRef};
use field::{Address, Amount, BlockHeight, Encode, Fixed16, Hash, Timestamp, Uint1, Uint4};
use num_bigint::BigUint;
use protocol::block_std::BlockV1;

use crate::tx_coinbase::CoinbaseTx;

pub static GENESIS_BLOCK_HASH: LazyLock<Hash> = LazyLock::new(|| {
    Hash::from(
        hex::decode("000000077790ba2fcdeaef4a4299d9b667135bac577ce204dee8388f1b97f7e6")
            .unwrap()
            .try_into()
            .unwrap(),
    )
});

/// Expected serialized mainnet genesis block bytes (from fullnodedev
/// `mint/src/genesis/block.rs`). Byte-level self-check: any drift in the
/// block/tx codecs or in the genesis construction itself panics at startup
/// instead of silently booting a genesis whose hash no longer matches its
/// bytes.
const GENESIS_BLOCK_BODY_HEX: &str = "010000000000005c57b08c0000000000000000000000000000000000000000000000000000000000000000ad557702fc70afaf70a855e7b8a4400159643cb5a7fc8a89ba2bce6f818a9b0100000001098b344500000000000000000c1aaa4e6007cc58cfb932052ac0ec25ca356183f80101686172646572746f646f62657474657200";

/// Validate the constructed genesis block against the locked mainnet bytes:
/// both the computed block hash and the full serialized body must match.
fn check_genesis_bytes(genesis: &BlockV1) {
    let got_hash = genesis.hash();
    let want_hash = *GENESIS_BLOCK_HASH;
    if got_hash != want_hash {
        panic!(
            "Genesis Block Hash Error: expected {} but got {}",
            want_hash, got_hash
        );
    }
    let got_body = genesis.encode();
    let want_body = hex::decode(GENESIS_BLOCK_BODY_HEX).expect("genesis body hex decode");
    if got_body != want_body {
        panic!(
            "Genesis Block Body Error: expected {} but got {}",
            hex::encode(&want_body),
            hex::encode(got_body)
        );
    }
}

#[derive(Debug)]
struct GenesisBlock {
    inner: BlockV1,
}

impl field::Encode for GenesisBlock {
    fn size(&self) -> usize {
        self.inner.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.inner.encode_to(out);
    }
}

impl Block for GenesisBlock {
    fn version(&self) -> u8 {
        self.inner.version()
    }

    fn height(&self) -> u64 {
        self.inner.height()
    }

    fn hash(&self) -> Hash {
        genesis_block_hash()
    }

    fn prev_hash(&self) -> Hash {
        self.inner.prev_hash()
    }

    fn mrklroot(&self) -> Hash {
        self.inner.mrklroot()
    }

    fn timestamp(&self) -> u64 {
        self.inner.timestamp()
    }

    fn as_pow(&self) -> Option<&dyn PowBlock> {
        Some(self)
    }

    fn transactions(&self) -> &[TxRef] {
        self.inner.transactions()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl PowBlock for GenesisBlock {
    fn nonce(&self) -> u32 {
        self.inner.nonce.uint()
    }

    fn difficulty(&self) -> u32 {
        self.inner.difficulty.uint()
    }
}

static GENESIS_BLOCK: LazyLock<BlockRef> = LazyLock::new(|| {
    Arc::new(GenesisBlock {
        inner: create_genesis_block(),
    })
});

pub fn genesis_block() -> BlockRef {
    GENESIS_BLOCK.clone()
}

pub fn genesis_block_hash() -> Hash {
    *GENESIS_BLOCK_HASH
}

pub fn create_genesis_block() -> BlockV1 {
    let reward_addr = Address::from_readable("1271438866CSDpJUqrnchoJAiGGBFSQhjd").unwrap();
    let mut genesis = BlockV1::new(crate::block_hasher);
    genesis.height = BlockHeight::from(0);
    genesis.timestamp = Timestamp::from(1_549_250_700);
    genesis.nonce = Uint4::from(160_117_829);
    genesis
        .push_transaction(Arc::new(CoinbaseTx {
            ty: Uint1::from(CoinbaseTx::TYPE),
            address: reward_addr,
            reward: Amount::small(1, field::UNIT_MEI),
            message: Fixed16::from(*b"hardertodobetter"),
            extend: Default::default(),
        }))
        .unwrap();
    genesis.update_mrklroot();
    check_genesis_bytes(&genesis);
    genesis
}

pub fn calculate_interest(
    user_distribute_amt: &Amount,
    interest_calc_base_amt: &Amount,
    calc_loop: u64,
    wfzn: u64,
) -> sys::Ret<Amount> {
    let newunit = interest_calc_base_amt.unit() as i32 - 8;
    if newunit < 0 {
        return Ok(user_distribute_amt.clone());
    }
    let zero = BigUint::from(0u64);
    let mut coinnum = BigUint::from_bytes_be(interest_calc_base_amt.byte());
    coinnum *= 1_0000_0000u64;
    for _ in 0..calc_loop {
        coinnum *= 10_000u64 + wfzn;
        coinnum /= 10_000u64;
    }
    let mut unit = newunit as u8;
    loop {
        if unit >= 255 || coinnum.clone() % 10u64 != zero {
            break;
        }
        coinnum /= 10u64;
        unit += 1;
    }
    let realbest = Amount::from_unit_byte(unit, coinnum.to_bytes_be())?
        .sub_mode_u64(interest_calc_base_amt)?;
    realbest.add_mode_u64(user_distribute_amt)
}

pub fn both_interest(
    distribute_type: Uint1,
    amtl: &Amount,
    amtr: &Amount,
    calc_loop: u64,
    wfzn: u64,
) -> sys::Ret<(Amount, Amount)> {
    if field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_DEFAULT == distribute_type {
        let amt1 = calculate_interest(amtl, amtl, calc_loop, wfzn)?;
        let amt2 = calculate_interest(amtr, amtr, calc_loop, wfzn)?;
        return Ok((amt1, amt2));
    }

    let total = amtl.add_mode_u64(amtr)?;
    let mut res = (amtl.clone(), amtr.clone());
    if field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_LEFT == distribute_type {
        res.0 = calculate_interest(amtl, &total, calc_loop, wfzn)?;
    }
    if field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_RIGHT == distribute_type {
        res.1 = calculate_interest(amtr, &total, calc_loop, wfzn)?;
    }
    Ok(res)
}

pub fn calculate_interest_of_height(
    curblkhei: u64,
    chanopenblkhei: u64,
    distribute_type: Uint1,
    amtl: &Amount,
    amtr: &Amount,
) -> sys::Ret<(Amount, Amount)> {
    if curblkhei < chanopenblkhei {
        return sys::errf!("current block height cannot be less than channel open height");
    }
    let calc_loop = (curblkhei - chanopenblkhei) / 10_000;
    let wfzn = 10;
    if calc_loop == 0 {
        return Ok((amtl.clone(), amtr.clone()));
    }
    both_interest(distribute_type, amtl, amtr, calc_loop, wfzn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reaching this proves `create_genesis_block()` is byte-identical to the
    /// locked mainnet genesis (it panics inside on any mismatch).
    #[test]
    fn genesis_construction_matches_mainnet_bytes() {
        let genesis = create_genesis_block();
        assert_eq!(genesis.hash(), *GENESIS_BLOCK_HASH);
        assert_eq!(
            genesis.encode(),
            hex::decode(GENESIS_BLOCK_BODY_HEX).unwrap()
        );
    }

    /// A genesis whose computed hash still matches (fake hasher) but whose
    /// serialized body differs must panic with the body error.
    #[test]
    #[should_panic(expected = "Genesis Block Body Error")]
    fn tampered_genesis_body_panics() {
        let mut genesis = BlockV1::new(|_, _| GENESIS_BLOCK_HASH.0);
        genesis.height = BlockHeight::from(0);
        genesis.timestamp = Timestamp::from(1_549_250_700);
        genesis
            .push_transaction(Arc::new(CoinbaseTx {
                ty: Uint1::from(CoinbaseTx::TYPE),
                address: Address::from_readable("1271438866CSDpJUqrnchoJAiGGBFSQhjd").unwrap(),
                reward: Amount::small(1, field::UNIT_MEI),
                message: Fixed16::from(*b"hardertodobetter"),
                extend: Default::default(),
            }))
            .unwrap();
        genesis.update_mrklroot();
        check_genesis_bytes(&genesis);
    }
}
