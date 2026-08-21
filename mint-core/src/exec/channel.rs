//! Channel open/close execute bodies.

use base::{
    Context, hac_add, hac_sub, sat_add, total_add_u8, total_add_u12, total_sub_u8, total_sub_u12,
};
use field::{
    AddrBalance, Balance, ChannelId, ChannelSto, ClosedDistributionData,
    ClosedDistributionDataOptional, Encode, Uint2, Uint4,
};
use sys::{Rerr, Ret, errf};

use crate::action::channel::{ChannelClose, ChannelOpen};
use crate::state::{MintState, with_mint_total};

fn channel_open(this: &ChannelOpen, ctx: &mut dyn Context) -> Rerr {
    check_channel_id(&this.channel_id)?;
    let left_addr = &this.left_bill.address;
    let right_addr = &this.right_bill.address;
    let left_amt = &this.left_bill.amount;
    let right_amt = &this.right_bill.amount;
    if !left_addr.is_privkey() || !right_addr.is_privkey() {
        return errf!("channel address must be PRIVAKEY type");
    }
    if left_addr == right_addr {
        return errf!("left address cannot be equal to right address");
    }
    if left_amt.size() > 6 {
        return errf!("left amount bytes too long");
    }
    if right_amt.size() > 6 {
        return errf!("right amount bytes too long");
    }
    if left_amt.is_negative()
        || right_amt.is_negative()
        || (left_amt.is_zero() && right_amt.is_zero())
    {
        return errf!("left or right amount must be positive, or both are empty");
    }
    if left_amt.not_zero() {
        hac_sub(ctx, left_addr, left_amt)?;
    }
    if right_amt.not_zero() {
        hac_sub(ctx, right_addr, right_amt)?;
    }
    let lock_total = left_amt.add_mode_u64(right_amt)?;
    let lock_total_238 = lock_total.to_238_u64()? as u128;

    let mut reuse_version = Uint4::from(1);
    {
        let state = MintState::wrap(ctx.layer());
        if let Some(chan) = state.channel(&this.channel_id)? {
            let samebothaddr =
                *left_addr == chan.left_bill.address && *right_addr == chan.right_bill.address;
            if !samebothaddr || chan.status != field::CHANNEL_STATUS_AGREEMENT_CLOSED {
                return errf!("channel is opening or cannot be reused");
            }
            let Some(next_version) = chan.reuse_version.uint().checked_add(1) else {
                return errf!("channel reuse_version overflow");
            };
            reuse_version = Uint4::from(next_version);
        }
    }

    let channel = ChannelSto {
        status: field::CHANNEL_STATUS_OPENING,
        reuse_version,
        open_height: field::BlockHeight::from(ctx.env().block.height),
        close_height: field::BlockHeight::from(0),
        arbitration_lock_block: Uint2::from(5000),
        interest_attribution: field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_DEFAULT,
        left_bill: AddrBalance {
            address: *left_addr,
            balance: Balance::hac(left_amt.clone()),
        },
        right_bill: AddrBalance {
            address: *right_addr,
            balance: Balance::hac(right_amt.clone()),
        },
        if_challenging: Default::default(),
        if_distribution: Default::default(),
    };
    MintState::wrap(ctx.layer()).channel_set(&this.channel_id, &channel);
    with_mint_total(&mut MintState::wrap(ctx.layer()), |ttcount| {
        total_add_u8(&mut ttcount.opening_channel, 1, "opening_channel")?;
        total_add_u12(
            &mut ttcount.channel_deposit_238,
            lock_total_238,
            "channel_deposit_238",
        )?;
        total_add_u8(&mut ttcount.channel_open_total, 1, "channel_open_total")?;
        Ok(())
    })?;
    Ok(())
}

fn channel_close(this: &ChannelClose, ctx: &mut dyn Context) -> Ret<Vec<u8>> {
    check_channel_id(&this.channel_id)?;
    let pending_height = ctx.env().block.height;
    let Some(chan) = MintState::wrap(ctx.layer()).channel(&this.channel_id)? else {
        return errf!("channel not found");
    };
    if !chan.left_bill.address.is_privkey() || !chan.right_bill.address.is_privkey() {
        return errf!("channel address must be PRIVAKEY type");
    }
    ctx.check_sign(&chan.left_bill.address)?;
    ctx.check_sign(&chan.right_bill.address)?;
    close_channel_default(pending_height, ctx, &this.channel_id, &chan)
}

fn check_channel_id(id: &ChannelId) -> Rerr {
    let key = id.as_ref();
    if key.len() != ChannelId::SIZE || key[0] == 0 || key[ChannelId::SIZE - 1] == 0 {
        return errf!("channel check key {} format failed", id);
    }
    Ok(())
}

fn close_channel_default(
    pending_height: u64,
    ctx: &mut dyn Context,
    channel_id: &ChannelId,
    channel: &ChannelSto,
) -> Ret<Vec<u8>> {
    close_channel_with_distribution(
        pending_height,
        ctx,
        channel_id,
        channel,
        &channel.left_bill.balance,
        &channel.right_bill.balance,
        false,
    )
}

fn close_channel_with_distribution(
    pending_height: u64,
    ctx: &mut dyn Context,
    channel_id: &ChannelId,
    channel: &ChannelSto,
    left_bls: &Balance,
    right_bls: &Balance,
    final_closed: bool,
) -> Ret<Vec<u8>> {
    if channel.status != field::CHANNEL_STATUS_OPENING {
        return errf!("channel is not open");
    }
    let left_addr = &channel.left_bill.address;
    let right_addr = &channel.right_bill.address;
    let left_amt = &left_bls.hacash;
    let right_amt = &right_bls.hacash;
    if left_amt.is_negative() || right_amt.is_negative() {
        return errf!("channel distribution amount cannot be negative");
    }
    let locked_hac = channel
        .left_bill
        .balance
        .hacash
        .add_mode_u64(&channel.right_bill.balance.hacash)?;
    if left_amt.add_mode_u64(right_amt)? != locked_hac {
        return errf!("HAC distribution amount must match lock-in");
    }
    let locked_hac_238 = locked_hac.to_238_u64()? as u128;
    let locked_sat = channel
        .left_bill
        .balance
        .satoshi
        .uint()
        .checked_add(channel.right_bill.balance.satoshi.uint())
        .ok_or_else(|| sys::Error::fault("channel satoshi overflow"))?;
    let dist_sat = left_bls
        .satoshi
        .uint()
        .checked_add(right_bls.satoshi.uint())
        .ok_or_else(|| sys::Error::fault("channel satoshi overflow"))?;
    if dist_sat != locked_sat {
        return errf!("BTC distribution amount must match lock-in");
    }

    let mut interest_add_238 = 0u64;
    let mut deposit_sub_238 = 0u128;
    let mut closed_hac_volume_add_238 = 0u128;
    let mut deposit_sat_sub = 0u64;

    if locked_hac.is_positive() {
        let (new_left, new_right) = crate::interest::calculate_interest_of_height(
            pending_height,
            channel.open_height.uint(),
            channel.interest_attribution,
            left_amt,
            right_amt,
        )?;
        let new_total = new_left.add_mode_u64(&new_right)?;
        if new_total < locked_hac {
            return errf!("interest calculation failed");
        }
        let interest = new_total.sub_mode_u64(&locked_hac)?;
        interest_add_238 = interest.to_238_u64()?;
        deposit_sub_238 = locked_hac_238;
        closed_hac_volume_add_238 = locked_hac_238;
        if new_left.is_positive() {
            hac_add(ctx, left_addr, &new_left)?;
        }
        if new_right.is_positive() {
            hac_add(ctx, right_addr, &new_right)?;
        }
    }
    if locked_sat > 0 {
        deposit_sat_sub = locked_sat;
        if left_bls.satoshi.uint() > 0 {
            sat_add(ctx, left_addr, &left_bls.satoshi.to_satoshi())?;
        }
        if right_bls.satoshi.uint() > 0 {
            sat_add(ctx, right_addr, &right_bls.satoshi.to_satoshi())?;
        }
    }

    let mut save = channel.clone();
    save.status = if final_closed {
        field::CHANNEL_STATUS_FINAL_ARBITRATION_CLOSED
    } else {
        field::CHANNEL_STATUS_AGREEMENT_CLOSED
    };
    save.close_height = field::BlockHeight::from(pending_height);
    save.if_distribution = ClosedDistributionDataOptional::must(ClosedDistributionData {
        left_bill: Balance {
            hacash: left_amt.clone(),
            satoshi: left_bls.satoshi,
            ..Default::default()
        },
        right_bill: Balance {
            hacash: right_amt.clone(),
            satoshi: right_bls.satoshi,
            ..Default::default()
        },
    });
    MintState::wrap(ctx.layer()).channel_set(channel_id, &save);
    with_mint_total(&mut MintState::wrap(ctx.layer()), |ttcount| {
        total_sub_u8(&mut ttcount.opening_channel, 1, "opening_channel")?;
        total_add_u8(&mut ttcount.channel_close_total, 1, "channel_close_total")?;
        total_add_u8(
            &mut ttcount.channel_interest_238,
            interest_add_238,
            "channel_interest_238",
        )?;
        total_sub_u12(
            &mut ttcount.channel_deposit_238,
            deposit_sub_238,
            "channel_deposit_238",
        )?;
        total_add_u12(
            &mut ttcount.channel_closed_hac_volume_238,
            closed_hac_volume_add_238,
            "channel_closed_hac_volume_238",
        )?;
        total_sub_u8(
            &mut ttcount.channel_deposit_sat,
            deposit_sat_sub,
            "channel_deposit_sat",
        )?;
        Ok(())
    })?;
    Ok(vec![])
}

base::impl_action_execute! {
    ChannelOpen {
        (self, ctx) {
            channel_open(self, ctx)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    ChannelClose {
        (self, ctx) {
            channel_close(self, ctx)
        }
    }
}
