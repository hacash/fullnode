//! Coinbase / prelude

use std::any::Any;
use std::sync::Arc;

use base::{Context, Transaction, TransactionBuild, hac_add};
use field::{Address, Amount, Bool, Encode, Fixed16, Hash, Reader, Sign, Uint1};
use sys::{Rerr, Ret};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoinbaseExtendDataV1 {
    pub miner_nonce: Hash,
    pub witness_count: Uint1,
}

impl Encode for CoinbaseExtendDataV1 {
    fn size(&self) -> usize {
        self.miner_nonce.size() + self.witness_count.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.miner_nonce.encode_to(out);
        self.witness_count.encode_to(out);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoinbaseExtend {
    pub exist: Bool,
    pub extend: Option<CoinbaseExtendDataV1>,
}

impl CoinbaseExtend {
    pub fn must(extend: CoinbaseExtendDataV1) -> Self {
        Self {
            exist: Bool::new(true),
            extend: Some(extend),
        }
    }

    fn is_exist(&self) -> bool {
        self.exist.is_true()
    }
}

impl Encode for CoinbaseExtend {
    fn size(&self) -> usize {
        self.exist.size()
            + if self.is_exist() {
                self.extend.as_ref().unwrap().size()
            } else {
                0
            }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.exist.encode_to(out);
        if self.is_exist() {
            self.extend.as_ref().unwrap().encode_to(out);
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoinbaseTx {
    pub ty: Uint1,
    pub address: Address,
    pub reward: Amount,
    pub message: Fixed16,
    pub extend: CoinbaseExtend,
}

impl Default for CoinbaseTx {
    fn default() -> Self {
        Self {
            ty: Uint1::from(Self::TYPE),
            address: Address::default(),
            reward: Amount::zero(),
            message: Fixed16::default(),
            extend: CoinbaseExtend::default(),
        }
    }
}

impl CoinbaseTx {
    pub const TYPE: u8 = 0;
}

impl Encode for CoinbaseTx {
    fn size(&self) -> usize {
        self.ty.size()
            + self.address.size()
            + self.reward.size()
            + self.message.size()
            + self.extend.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.ty.encode_to(out);
        self.address.encode_to(out);
        self.reward.encode_to(out);
        self.message.encode_to(out);
        self.extend.encode_to(out);
    }
}

impl Transaction for CoinbaseTx {
    fn ty(&self) -> u8 {
        Self::TYPE
    }
    fn main(&self) -> Address {
        self.address
    }
    fn addrs(&self) -> Vec<Address> {
        vec![self.address]
    }
    fn fee(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn fee_pay(&self) -> Amount {
        Amount::zero()
    }
    fn fee_got(&self) -> Amount {
        Amount::zero()
    }
    fn author(&self) -> Option<Address> {
        Some(self.address)
    }
    fn block_reward(&self) -> Option<&Amount> {
        Some(&self.reward)
    }
    fn block_message(&self) -> Option<&Fixed16> {
        Some(&self.message)
    }
    fn fee_receiver(&self) -> Option<Address> {
        Some(self.address)
    }
    fn mempool_policy(&self) -> base::MempoolPolicy {
        base::MempoolPolicy::Forbidden
    }
    fn is_block_prelude(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl base::TransactionSign for CoinbaseTx {
    fn hash(&self) -> Hash {
        Hash::from(sys::calculate_hash(self.encode()))
    }
    fn req_sign(&self) -> Ret<Vec<Address>> {
        Ok(vec![])
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }

    fn as_execute(&self) -> Option<&dyn base::TransactionExecute> {
        Some(self)
    }
}

impl base::TransactionExecute for CoinbaseTx {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        hac_add(ctx, &self.address, &self.reward)?;
        Ok(())
    }
}

impl TransactionBuild for CoinbaseTx {
    fn set_mining_nonce(&mut self, nonce: Hash) {
        if let Some(extend) = &mut self.extend.extend {
            extend.miner_nonce = nonce;
        }
    }
    fn fill_sign(&mut self, _acc_addr: &Address) -> Ret<Sign> {
        sys::errf!("coinbase does not sign")
    }
}

pub fn create_coinbase(_reg: &dyn base::BinaryCodecs, buf: &[u8]) -> Ret<(base::TxRef, usize)> {
    let mut r = Reader::new(buf);
    let ty: Uint1 = r.read()?;
    let address: Address = r.read()?;
    let reward: Amount = r.read()?;
    let message: Fixed16 = r.read()?;
    let exist: Bool = r.read()?;
    let extend = if exist.is_true() {
        Some(CoinbaseExtendDataV1 {
            miner_nonce: r.read()?,
            witness_count: r.read()?,
        })
    } else {
        None
    };
    Ok((
        Arc::new(CoinbaseTx {
            ty,
            address,
            reward,
            message,
            extend: CoinbaseExtend { exist, extend },
        }),
        r.used(),
    ))
}
