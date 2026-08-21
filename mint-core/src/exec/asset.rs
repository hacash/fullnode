//! AssetCreate execute body.

use base::{Context, CoreState, hac_sub, total_add_amount_238, total_add_u8};
use field::{Amount, AssetAmt, AssetSmelt};
use sys::{Rerr, errf};

use crate::action::asset::AssetCreate;
use crate::state::{MintState, with_mint_total};

fn check_alive_blk_hei(ctx: &dyn Context, rules: hacash_params::MintRules) -> (u64, u64) {
    let is_mainnet = ctx.env().chain.id.is_mainnet();
    if is_mainnet {
        (rules.asset_alive_height, rules.asset_mainnet_min_serial)
    } else {
        (
            rules.asset_non_mainnet_alive_height,
            rules.asset_non_mainnet_min_serial,
        )
    }
}

fn execute_asset_create(
    ctx: &mut dyn Context,
    metadata: &AssetSmelt,
    protocol_cost: &Amount,
) -> Rerr {
    let profile = ctx.services().execution_profile()?;
    let params = hacash_params::as_hacash_params(profile)
        .ok_or_else(|| sys::Error::fault("standard Hacash params not registered"))?;
    let amd = metadata.clone();
    let fast_sync = ctx.env().chain.fast_sync;
    let serial = amd.serial.uint();
    let chei = ctx.env().block.height;
    if !fast_sync {
        let (alive_hei, minsri) = check_alive_blk_hei(ctx, params.mint_rules);
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
        let required_fee =
            Amount::small(params.mint_rules.block_reward_number(chei), field::UNIT_MEI);
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

base::impl_action_execute! {
    AssetCreate {
        (self, ctx) {
            execute_asset_create(ctx, &self.metadata, &self.protocol_cost)?;
            Ok(vec![])
        }
    }
}
