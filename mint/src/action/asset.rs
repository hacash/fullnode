use std::sync::Arc;

use base::{ActionRef, Context, CoreState, hac_sub, total_add_amount_238, total_add_u8};
use field::{Amount, AssetAmt, AssetSmelt, Decode, Encode, Uint2};
use sys::{Rerr, Ret, errf};

use crate::{
    minter::block_reward_number,
    state::{MintState, with_mint_total},
};

pub const ASSET_ALIVE_HEIGHT: u64 = 765_432;

#[derive(Debug, Clone, base::ActionCodec)]
pub struct AssetCreate {
    pub kind: Uint2,
    pub metadata: AssetSmelt,
    pub protocol_cost: Amount,
}

impl AssetCreate {
    pub const KIND: u16 = 16;

    pub fn new(metadata: AssetSmelt, protocol_cost: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            metadata,
            protocol_cost,
        }
    }
}

base::impl_action! {
    AssetCreate {
        name: "asset_create",
        scope: base::ActScope::TOP_ONLY,
        min_tx_type: 2,
        description: |this: &AssetCreate| format!("Register asset <{}>", this.metadata.ticket.to_readable_or_hex()),
        execute: (self, ctx) {
        execute_asset_create(ctx, &self.metadata, &self.protocol_cost)?;
        Ok(vec![])
        }
    }
}

fn check_alive_blk_hei(ctx: &mut dyn Context) -> (u64, u64) {
    let is_mainnet = ctx.env().chain.id.is_mainnet();
    if is_mainnet {
        (ASSET_ALIVE_HEIGHT, 1025)
    } else {
        (0, 5)
    }
}

fn execute_asset_create(
    ctx: &mut dyn Context,
    metadata: &AssetSmelt,
    protocol_cost: &Amount,
) -> Rerr {
    let amd = metadata.clone();
    let fast_sync = ctx.env().chain.fast_sync;
    let serial = amd.serial.uint();
    let chei = ctx.env().block.height;
    if !fast_sync {
        let (alive_hei, minsri) = check_alive_blk_hei(ctx);
        if alive_hei > chei {
            return errf!("The asset issuance has not yet begun");
        }
        if serial < minsri {
            return errf!("serial cannot be less than {}", minsri);
        }
        if serial > minsri + (chei - alive_hei) {
            return errf!("asset serial overflow");
        }
        if !amd.issuer.is_supported() {
            return errf!(
                "issuer address version {} is not supported",
                amd.issuer.version()
            );
        }
        if amd.issuer.is_privkey_unknown() {
            return errf!(
                "issuer cannot be system address {:?} (privakey unknown)",
                amd.issuer
            );
        }
        let tl = amd.ticket.length();
        let nl = amd.name.length();
        if tl < 1 || tl > 8 {
            return errf!("ticket length must be 1 ~ 8");
        }
        if nl < 1 || nl > 32 {
            return errf!("name length must be 1 ~ 32");
        }
        if !sys::check_readable_string(amd.ticket.as_ref()) {
            return errf!("ticket must be ascii2 readable string");
        }
        if !sys::check_readable_string(amd.name.as_ref()) {
            return errf!("name must be ascii2 readable string");
        }
        if amd.decimal.uint() > 16 {
            return errf!("decimal cannot exceed 16");
        }
        if amd.supply.is_zero() {
            return errf!("supply must be greater than zero");
        }
        let required_fee = Amount::small(block_reward_number(chei), field::UNIT_MEI);
        if protocol_cost != &required_fee {
            return errf!(
                "Protocol fee must be {:?} but got {:?}",
                required_fee,
                protocol_cost
            );
        }
    }

    let main_addr = ctx.env().tx.main;
    hac_sub(ctx, &main_addr, protocol_cost)?;

    let mut state = CoreState::wrap(ctx.layer());
    if !fast_sync && state.asset(&amd.serial)?.is_some() {
        return errf!("Asset serial {} already exists", serial);
    }
    state.asset_set(&amd.serial, &amd);
    with_mint_total(&mut MintState::wrap(&mut *state.0), |ttcount| {
        total_add_u8(&mut ttcount.created_asset, 1, "created_asset")?;
        total_add_amount_238(
            &mut ttcount.asset_issue_burn_238,
            protocol_cost,
            "asset_issue_burn_238",
        )?;
        Ok(())
    })?;

    let asset_obj = AssetAmt {
        serial: amd.serial,
        amount: amd.supply,
    }
    .checked()?;
    let mut bls = state.balance(&amd.issuer)?.unwrap_or_default();
    bls.asset_set(asset_obj)?;
    state.balance_set(&amd.issuer, &bls);
    Ok(())
}

pub fn create_asset_create(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = AssetCreate::decode(buf)?;
    Ok((Arc::new(action), used))
}
