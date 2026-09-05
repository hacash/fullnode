//! VM syscall action execute bodies.

use base::CoreState;
use field::{Address, DiamondName, Encode};
use sys::errf;

use crate::codec::action::{
    BalanceAsset, BalanceCoin, BlockAuthorAddr, CheckSignature, EnvHeight, HacdInscGet,
    HacdInscNum, HacdNameList, HacdOwnerAddrs, TxBlob, TxBlobNum, TxBlobSize, TxMainAddr,
    TxMessage, TxMessageNum,
};

/// Temporary upgrade gate for the tx message/blob read syscalls (0x0615/0x0616/
/// 0x0617/0x0704/0x0705), which take effect at height 784_000. Hand-written on
/// purpose — remove the const, this helper and the five call sites together with
/// the syscalls in the next release.
const TX_MSG_BLOB_ENABLE_HEIGHT: u64 = 784_000;

fn tx_message_blob_gate(ctx: &dyn base::Context) -> sys::Rerr {
    // Mainnet activates at height 784_000. Non-mainnet (hacash-testnet chain_id=1)
    // enables immediately so deposit hooks can ViewMessage on a local chain.
    if ctx.env().chain.id.is_mainnet() && ctx.env().block.height < TX_MSG_BLOB_ENABLE_HEIGHT {
        return errf!(
            "tx message/blob syscall not enabled until height {}",
            TX_MSG_BLOB_ENABLE_HEIGHT
        );
    }
    Ok(())
}

base::impl_action_execute! {
    TxMessage {
        (self, ctx) {
            tx_message_blob_gate(ctx)?;
            let mut n = 0u8;
            for action in ctx.tx().actions() {
                if let Some(msg) = action.as_any().downcast_ref::<crate::codec::action::Message>() {
                    if n == self.idx.uint() { return Ok(msg.data.as_ref().to_vec()); }
                    n = n.saturating_add(1);
                }
            }
            errf!("transaction message index {} out of range", self.idx.uint())
        }
    }
}

base::impl_action_execute! {
    TxBlob {
        (self, ctx) {
            tx_message_blob_gate(ctx)?;
            let mut n = 0u8;
            for action in ctx.tx().actions() {
                if let Some(blob) = action.as_any().downcast_ref::<crate::codec::action::Blob>() {
                    if n == self.idx.uint() {
                        let start = self.start.uint() as usize;
                        let end = self.end.uint() as usize;
                        let data = blob.data.as_ref();
                        if start > end || end > data.len() {
                            return errf!("blob range [{}..{}] out of range for size {}", start, end, data.len());
                        }
                        return Ok(data[start..end].to_vec());
                    }
                    n = n.saturating_add(1);
                }
            }
            errf!("transaction blob index {} out of range", self.idx.uint())
        }
    }
}

base::impl_action_execute! {
    TxMessageNum { (self, ctx) {
        tx_message_blob_gate(ctx)?;
        let n = ctx.tx().actions().iter().filter(|a| a.as_any().is::<crate::codec::action::Message>()).count();
        if n > u8::MAX as usize { return errf!("message count exceeds u8"); }
        Ok(vec![n as u8])
    } }
}
base::impl_action_execute! {
    TxBlobNum { (self, ctx) {
        tx_message_blob_gate(ctx)?;
        let n = ctx.tx().actions().iter().filter(|a| a.as_any().is::<crate::codec::action::Blob>()).count();
        if n > u8::MAX as usize { return errf!("blob count exceeds u8"); }
        Ok(vec![n as u8])
    } }
}
base::impl_action_execute! {
    TxBlobSize { (self, ctx) {
        tx_message_blob_gate(ctx)?;
        let mut n = 0u8;
        for action in ctx.tx().actions() {
            if let Some(blob) = action.as_any().downcast_ref::<crate::codec::action::Blob>() {
                if n == self.idx.uint() {
                    let len = blob.data.as_ref().len();
                    if len > u16::MAX as usize {
                        return errf!("transaction blob {} size {} exceeds u16::MAX", self.idx.uint(), len);
                    }
                    return Ok((len as u16).to_be_bytes().to_vec());
                }
                n = n.saturating_add(1);
            }
        }
        errf!("transaction blob index {} out of range", self.idx.uint())
    } }
}
base::impl_action_execute! {
    EnvHeight {
        (self, ctx) {
            Ok(ctx.env().block.height.to_be_bytes().to_vec())
        }
    }
}

base::impl_action_execute! {
    TxMainAddr {
        (self, ctx) {
            Ok(ctx.env().tx.main.as_ref().to_vec())
        }
    }
}

base::impl_action_execute! {
    BlockAuthorAddr {
        (self, ctx) {
            Ok(ctx.env().block.author.as_ref().to_vec())
        }
    }
}

base::impl_action_execute! {
    BalanceCoin {
        (self, ctx) {
            let bls = CoreState::wrap(ctx.layer())
                .balance(&self.addr)?
                .unwrap_or_default();
            let dia = bls.diamond.uint();
            if dia > u32::MAX as u64 {
                return errf!(
                    "address {} diamond count {} exceeds u32::MAX",
                    self.addr.to_readable(),
                    dia
                );
            }
            let hac = bls.hacash.encode();
            let mut res = Vec::with_capacity(12 + hac.len());
            res.extend_from_slice(&(dia as u32).to_be_bytes());
            res.extend_from_slice(&bls.satoshi.uint().to_be_bytes());
            res.extend_from_slice(&hac);
            Ok(res)
        }
    }
}

base::impl_action_execute! {
    BalanceAsset {
        (self, ctx) {
            let serial = self.serial.uint();
            if serial == 0 {
                return errf!("asset serial cannot be zero");
            }
            let bls = CoreState::wrap(ctx.layer())
                .balance(&self.addr)?
                .unwrap_or_default();
            let amt = bls
                .assets
                .as_list()
                .iter()
                .find(|a| a.serial.uint() == serial)
                .map(|a| a.amount.uint())
                .unwrap_or(0);
            Ok(amt.to_be_bytes().to_vec())
        }
    }
}

base::impl_action_execute! {
    CheckSignature {
        (self, ctx) {
            let ok = match ctx.check_sign(&self.addr) {
                Ok(()) => 1u8,
                Err(_) => 0u8,
            };
            Ok(vec![ok])
        }
    }
}

base::impl_action_execute! {
    HacdInscNum {
        (self, ctx) {
            let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond)? else {
                return errf!("diamond {} not found", self.diamond.to_readable());
            };
            let num = diaobj.inscripts.length();
            if num > u8::MAX as usize {
                return errf!(
                    "diamond {} inscripts number invalid",
                    self.diamond.to_readable()
                );
            }
            Ok(vec![num as u8])
        }
    }
}

base::impl_action_execute! {
    HacdInscGet {
        (self, ctx) {
            let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond)? else {
                return errf!("diamond {} not found", self.diamond.to_readable());
            };
            let num = diaobj.inscripts.length();
            let idx = self.inscidx.uint() as usize;
            if idx >= num {
                return errf!(
                    "diamond {} inscripts number overflow",
                    self.diamond.to_readable()
                );
            }
            Ok(diaobj.inscripts.as_list()[idx].content.to_vec())
        }
    }
}

base::impl_action_execute! {
    HacdNameList {
        (self, ctx) {
            const DNM_SZ: usize = DiamondName::SIZE;
            let owned = CoreState::wrap(ctx.layer())
                .diamond_owned(&self.addr)?
                .unwrap_or_default();
            let names = owned.names.as_ref();
            if names.len() % DNM_SZ != 0 {
                return errf!(
                    "address {} diamond names length {} invalid",
                    self.addr.to_readable(),
                    names.len()
                );
            }
            let limit = self.limit.uint() as usize;
            if limit > 200 {
                return errf!("limit {} cannot exceed 200", limit);
            }
            if limit == 0 {
                return Ok(vec![]);
            }
            let page = self.page.uint() as usize;
            let unit = limit * DNM_SZ;
            let start = page.saturating_mul(unit);
            if start >= names.len() {
                return Ok(vec![]);
            }
            let end = start.saturating_add(unit).min(names.len());
            Ok(names[start..end].to_vec())
        }
    }
}

base::impl_action_execute! {
    HacdOwnerAddrs {
        (self, ctx) {
            let num = self.diamonds.check()?;
            if num > 50 {
                return errf!("diamond list length {} cannot exceed 50", num);
            }
            let state = CoreState::wrap(ctx.layer());
            let mut res = Vec::with_capacity(num * Address::SIZE);
            for dian in self.diamonds.as_list() {
                let Some(diaobj) = state.diamond(dian)? else {
                    return errf!("diamond {} not found", dian.to_readable());
                };
                res.extend_from_slice(diaobj.address.as_ref());
            }
            Ok(res)
        }
    }
}
