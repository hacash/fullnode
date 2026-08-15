use crate::Context;
use field::{
    Address, Amount, AssetAmt, DiamondName, DiamondNameListMax200, DiamondNameListMax60000,
    DiamondNumber, DiamondNumberAuto, DiamondSto, Satoshi, SatoshiAuto, ToJSON, Uint1, Uint8,
    Uint12,
};
use sys::{Ret, errf, revertf};

use super::{BaseTotal, CoreState};

fn check_amount_is_positive(amt: &Amount) -> Ret<()> {
    if !amt.is_positive() {
        return errf!("amount {} value is not positive", amt);
    }
    Ok(())
}

pub const BLACKHOLE_ADDR: Address = Address::from([0u8; 21]);

fn check_transfer_recipient_allowed(to: &Address) -> Ret<()> {
    if to.is_privkey_unknown() && *to != BLACKHOLE_ADDR {
        return errf!(
            "cannot transfer to system address {} (privkey unknown)",
            to.to_json()
        );
    }
    Ok(())
}

fn check_supported_address(addr: &Address) -> Ret<()> {
    if addr.is_supported() {
        return Ok(());
    }
    errf!("address version {} not supported", addr.version())
}

fn u64_to_uint8(v: u64, name: &str) -> Ret<Uint8> {
    Uint8::from_checked(v).ok_or_else(|| sys::Error::fault(format!("{name} overflow")))
}

fn u128_to_uint12(v: u128, name: &str) -> Ret<Uint12> {
    Uint12::from_checked(v).ok_or_else(|| sys::Error::fault(format!("{name} overflow")))
}

pub fn with_base_total<R>(
    state: &mut CoreState,
    f: impl FnOnce(&mut BaseTotal) -> Ret<R>,
) -> Ret<R> {
    let mut total = state.get_base_total()?;
    let res = f(&mut total)?;
    state.set_base_total(&total);
    Ok(res)
}

pub fn total_add_u8(cur: &mut Uint8, add: u64, name: &str) -> Ret<()> {
    let next = cur
        .uint()
        .checked_add(add)
        .ok_or_else(|| sys::Error::fault(format!("{name} overflow")))?;
    *cur = u64_to_uint8(next, name)?;
    Ok(())
}

pub fn total_add_u12(cur: &mut Uint12, add: u128, name: &str) -> Ret<()> {
    let next = cur
        .uint()
        .checked_add(add)
        .ok_or_else(|| sys::Error::fault(format!("{name} overflow")))?;
    *cur = u128_to_uint12(next, name)?;
    Ok(())
}

pub fn total_add_diamond_number(cur: &mut DiamondNumber, add: usize, name: &str) -> Ret<()> {
    let next = (cur.uint() as usize)
        .checked_add(add)
        .ok_or_else(|| sys::Error::fault(format!("{name} overflow")))?;
    *cur = DiamondNumber::from_usize(next)?;
    Ok(())
}

pub fn total_sub_u8(cur: &mut Uint8, sub: u64, name: &str) -> Ret<()> {
    let next = cur
        .uint()
        .checked_sub(sub)
        .ok_or_else(|| sys::Error::fault(format!("{name} underflow")))?;
    *cur = u64_to_uint8(next, name)?;
    Ok(())
}

pub fn total_sub_u12(cur: &mut Uint12, sub: u128, name: &str) -> Ret<()> {
    let next = cur
        .uint()
        .checked_sub(sub)
        .ok_or_else(|| sys::Error::fault(format!("{name} underflow")))?;
    *cur = u128_to_uint12(next, name)?;
    Ok(())
}

pub fn total_add_amount_238(cur: &mut Uint12, amt: &Amount, name: &str) -> Ret<()> {
    total_add_u12(cur, amt.to_238_u64()? as u128, name)
}

pub fn total_record_blackhole_hac(state: &mut CoreState, amt: &Amount) -> Ret<()> {
    if !amt.is_positive() {
        return Ok(());
    }
    with_base_total(state, |total| {
        total_add_amount_238(
            &mut total.blackhole_hac_burn_238,
            amt,
            "blackhole_hac_burn_238",
        )
    })
}

pub fn total_record_blackhole_sat(state: &mut CoreState, sat: &Satoshi) -> Ret<()> {
    if sat.uint() == 0 {
        return Ok(());
    }
    with_base_total(state, |total| {
        total_add_u8(
            &mut total.blackhole_sat_burn,
            sat.uint(),
            "blackhole_sat_burn",
        )
    })
}

pub fn total_record_blackhole_asset(state: &mut CoreState) -> Ret<()> {
    with_base_total(state, |total| {
        total_add_u8(
            &mut total.blackhole_asset_burn_count,
            1,
            "blackhole_asset_burn_count",
        )
    })
}

pub fn total_record_blackhole_hacd(state: &mut CoreState) -> Ret<()> {
    with_base_total(state, |total| {
        total_add_u8(
            &mut total.blackhole_hacd_burn_count,
            1,
            "blackhole_hacd_burn_count",
        )
    })
}

pub fn blackhole_engulf(state: &mut CoreState, addr: &Address) -> Ret<bool> {
    if *addr != BLACKHOLE_ADDR {
        return Ok(false);
    }
    state.balance_set(addr, &field::Balance::default());
    if state.diamond_owned_exist(addr)? {
        state.diamond_owned_del(addr);
    }
    Ok(true)
}

fn do_hac_add(state: &mut CoreState, addr: &Address, amt: &Amount) -> Ret<Amount> {
    check_supported_address(addr)?;
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let new_hac = balance.hacash.add_mode_u128(amt)?;
    new_hac.check_store_long()?;
    balance.hacash = new_hac.clone();
    state.balance_set(addr, &balance);
    Ok(new_hac)
}

fn do_hac_sub(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Amount> {
    check_supported_address(addr)?;
    let mut state = CoreState::wrap(ctx.layer());
    let mut balance = state.balance(addr)?.unwrap_or_default();
    if balance.hacash < *amt {
        return revertf!(
            "address {} balance {} is insufficient, at least {}",
            addr.to_json(),
            balance.hacash,
            amt
        );
    }
    let new_hac = balance.hacash.sub_mode_u128(amt)?;
    new_hac.check_store_long()?;
    balance.hacash = new_hac.clone();
    state.balance_set(addr, &balance);
    Ok(new_hac)
}

pub fn hac_check(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Amount> {
    check_amount_is_positive(amt)?;
    check_supported_address(addr)?;
    let state = CoreState::wrap(ctx.layer());
    if let Some(balance) = state.balance(addr)? {
        if balance.hacash >= *amt {
            return Ok(balance.hacash);
        }
    }
    revertf!(
        "address {} balance is insufficient, at least {}",
        addr.to_json(),
        amt
    )
}

pub fn hac_add(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Vec<u8>> {
    hac_add_state(ctx.layer(), addr, amt)
}

/// Add HAC directly to a state layer when no transaction Context exists.
///
/// Block-level fee settlement uses this after all transactions have executed.
/// It intentionally shares the normal `hac_add` validation and blackhole
/// accounting semantics.
pub fn hac_add_state(
    layer: &mut dyn crate::StateLayer,
    addr: &Address,
    amt: &Amount,
) -> Ret<Vec<u8>> {
    check_amount_is_positive(amt)?;
    let mut state = CoreState::wrap(layer);
    do_hac_add(&mut state, addr, amt)?;
    if blackhole_engulf(&mut state, addr)? {
        total_record_blackhole_hac(&mut state, amt)?;
    }
    Ok(vec![])
}

pub fn hac_sub(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Vec<u8>> {
    check_amount_is_positive(amt)?;
    do_hac_sub(ctx, addr, amt)?;
    Ok(vec![])
}

pub fn hac_transfer(
    ctx: &mut dyn Context,
    from: &Address,
    to: &Address,
    amt: &Amount,
) -> Ret<Vec<u8>> {
    check_transfer_recipient_allowed(to)?;
    if from == to {
        if !from.is_privkey() {
            return errf!("non-privkey address cannot transfer HAC to self");
        }
        // Preserve the historical pre-200,000 self-transfer fast path. After
        // that height the balance check is consensus-visible.
        if ctx.env().block.height >= 200_000 {
            hac_check(ctx, from, amt)?;
        }
        return Ok(vec![]);
    }
    check_amount_is_positive(amt)?;
    do_hac_sub(ctx, from, amt)?;
    hac_add(ctx, to, amt)?;
    Ok(vec![])
}

fn check_satoshi_nonzero(sat: &Satoshi, what: &str) -> Ret<()> {
    if sat.uint() == 0 {
        return errf!("satoshi {} amount cannot be empty", what);
    }
    SatoshiAuto::check_satoshi(sat)
}

pub fn sat_add(ctx: &mut dyn Context, addr: &Address, sat: &Satoshi) -> Ret<Satoshi> {
    check_satoshi_nonzero(sat, "add")?;
    check_supported_address(addr)?;
    let mut state = CoreState::wrap(ctx.layer());
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let old = balance.satoshi.to_satoshi();
    let sum = old
        .uint()
        .checked_add(sat.uint())
        .ok_or_else(|| sys::Error::fault("satoshi add overflow"))?;
    let next = Satoshi::from(sum);
    balance.satoshi = SatoshiAuto::from_satoshi(&next)?;
    state.balance_set(addr, &balance);
    if blackhole_engulf(&mut state, addr)? {
        total_record_blackhole_sat(&mut state, sat)?;
    }
    Ok(next)
}

pub fn sat_sub(ctx: &mut dyn Context, addr: &Address, sat: &Satoshi) -> Ret<Satoshi> {
    check_satoshi_nonzero(sat, "sub")?;
    check_supported_address(addr)?;
    let mut state = CoreState::wrap(ctx.layer());
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let old = balance.satoshi.to_satoshi();
    if old < *sat {
        return revertf!(
            "address {} satoshi {} is insufficient, at least {}",
            addr.to_json(),
            old.uint(),
            sat.uint()
        );
    }
    let next = Satoshi::from(old.uint() - sat.uint());
    balance.satoshi = SatoshiAuto::from_satoshi(&next)?;
    state.balance_set(addr, &balance);
    Ok(next)
}

pub fn sat_check(ctx: &mut dyn Context, addr: &Address, sat: &Satoshi) -> Ret<Satoshi> {
    check_satoshi_nonzero(sat, "check")?;
    check_supported_address(addr)?;
    let state = CoreState::wrap(ctx.layer());
    if let Some(balance) = state.balance(addr)? {
        let old = balance.satoshi.to_satoshi();
        if old >= *sat {
            return Ok(old);
        }
    }
    revertf!("address {} satoshi is insufficient", addr.to_json())
}

pub fn sat_transfer(
    ctx: &mut dyn Context,
    from: &Address,
    to: &Address,
    sat: &Satoshi,
) -> Ret<Vec<u8>> {
    check_transfer_recipient_allowed(to)?;
    if from == to {
        return errf!("cannot transfer to self");
    }
    sat_sub(ctx, from, sat)?;
    sat_add(ctx, to, sat)?;
    Ok(vec![])
}

fn check_asset_nonzero(asset: &AssetAmt, what: &str) -> Ret<()> {
    if asset.amount.is_zero() {
        return errf!("asset {} amount cannot be empty", what);
    }
    Ok(())
}

pub fn asset_add(state: &mut CoreState, addr: &Address, asset: &AssetAmt) -> Ret<AssetAmt> {
    check_asset_nonzero(asset, "add")?;
    check_supported_address(addr)?;
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let old = balance.asset_must(asset.serial)?;
    let next = old.checked_add(asset)?;
    balance.asset_set(next.clone())?;
    state.balance_set(addr, &balance);
    if blackhole_engulf(state, addr)? {
        total_record_blackhole_asset(state)?;
    }
    Ok(next)
}

pub fn asset_sub(state: &mut CoreState, addr: &Address, asset: &AssetAmt) -> Ret<AssetAmt> {
    check_asset_nonzero(asset, "sub")?;
    check_supported_address(addr)?;
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let old = balance.asset_must(asset.serial)?;
    if old < *asset {
        return revertf!(
            "address {} asset is insufficient, at least {}",
            addr.to_json(),
            asset.amount.uint()
        );
    }
    let next = old.checked_sub(asset)?;
    balance.asset_set(next.clone())?;
    state.balance_set(addr, &balance);
    Ok(next)
}

pub fn asset_check(ctx: &mut dyn Context, addr: &Address, asset: &AssetAmt) -> Ret<AssetAmt> {
    check_asset_nonzero(asset, "check")?;
    check_supported_address(addr)?;
    let state = CoreState::wrap(ctx.layer());
    if let Some(balance) = state.balance(addr)? {
        if let Some(old) = balance.asset(asset.serial) {
            if old >= *asset {
                return Ok(old);
            }
        }
    }
    revertf!("address {} asset is insufficient", addr.to_json())
}

pub fn asset_transfer(
    ctx: &mut dyn Context,
    from: &Address,
    to: &Address,
    asset: &AssetAmt,
) -> Ret<Vec<u8>> {
    check_transfer_recipient_allowed(to)?;
    if from == to {
        return errf!("cannot transfer to self");
    }
    let mut state = CoreState::wrap(ctx.layer());
    asset_sub(&mut state, from, asset)?;
    asset_add(&mut state, to, asset)?;
    Ok(vec![])
}

pub const DIAMOND_STATUS_NORMAL: Uint1 = Uint1::from(1);

pub fn hacd_add(state: &mut CoreState, addr: &Address, hacd: &DiamondNumber) -> Ret<DiamondNumber> {
    check_supported_address(addr)?;
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let old = balance.diamond.to_diamond()?;
    let sum = old
        .uint()
        .checked_add(hacd.uint())
        .ok_or_else(|| sys::Error::fault("diamond add overflow"))?;
    let next = DiamondNumber::from(sum);
    balance.diamond = DiamondNumberAuto::from_diamond(&next);
    state.balance_set(addr, &balance);
    if blackhole_engulf(state, addr)? {
        total_record_blackhole_hacd(state)?;
    }
    Ok(next)
}

pub fn hacd_sub(state: &mut CoreState, addr: &Address, hacd: &DiamondNumber) -> Ret<DiamondNumber> {
    check_supported_address(addr)?;
    let mut balance = state.balance(addr)?.unwrap_or_default();
    let old = balance.diamond.to_diamond()?;
    if old < *hacd {
        return revertf!(
            "address {} diamond {} is insufficient, at least {}",
            addr.to_json(),
            old.uint(),
            hacd.uint()
        );
    }
    let next = DiamondNumber::from(old.uint() - hacd.uint());
    balance.diamond = DiamondNumberAuto::from_diamond(&next);
    state.balance_set(addr, &balance);
    Ok(next)
}

pub fn hacd_transfer(
    state: &mut CoreState,
    from: &Address,
    to: &Address,
    hacd: &DiamondNumber,
    _dlist: &DiamondNameListMax200,
) -> Ret<Vec<u8>> {
    if from == to {
        return errf!("cannot transfer to self");
    }
    hacd_sub(state, from, hacd)?;
    hacd_add(state, to, hacd)?;
    Ok(vec![])
}

pub fn check_diamond_status(
    state: &mut CoreState,
    addr_from: &Address,
    hacd_name: &DiamondName,
) -> Ret<DiamondSto> {
    check_supported_address(addr_from)?;
    let Some(diaitem) = state.diamond(hacd_name)? else {
        return errf!(
            "diamond status {} not found",
            String::from_utf8_lossy(hacd_name.as_ref())
        );
    };
    if diaitem.status != DIAMOND_STATUS_NORMAL {
        return revertf!(
            "diamond {} has been mortgaged and cannot be transferred",
            String::from_utf8_lossy(hacd_name.as_ref())
        );
    }
    if *addr_from != diaitem.address {
        return revertf!(
            "diamond {} does not belong to address {}",
            String::from_utf8_lossy(hacd_name.as_ref()),
            addr_from.to_json()
        );
    }
    Ok(diaitem)
}

pub fn hacd_move_one_diamond(
    state: &mut CoreState,
    addr_from: &Address,
    addr_to: &Address,
    hacd_name: &DiamondName,
) -> Ret<()> {
    check_supported_address(addr_from)?;
    check_supported_address(addr_to)?;
    if addr_from == addr_to {
        return errf!("cannot transfer to self");
    }
    let mut diaitem = check_diamond_status(state, addr_from, hacd_name)?;
    diaitem.address = *addr_to;
    state.diamond_set(hacd_name, &diaitem);
    Ok(())
}

pub fn diamond_owned_push_one(
    state: &mut CoreState,
    address: &Address,
    name: &DiamondName,
) -> Ret<()> {
    check_supported_address(address)?;
    let mut owned = state.diamond_owned(address)?.unwrap_or_default();
    owned.push_one(name)?;
    state.diamond_owned_set(address, &owned);
    Ok(())
}

pub fn diamond_owned_append(
    state: &mut CoreState,
    address: &Address,
    list: DiamondNameListMax60000,
) -> Ret<()> {
    check_supported_address(address)?;
    let mut owned = state.diamond_owned(address)?.unwrap_or_default();
    for name in list.as_list() {
        owned.push_one(name)?;
    }
    state.diamond_owned_set(address, &owned);
    Ok(())
}

pub fn diamond_owned_move(
    state: &mut CoreState,
    from: &Address,
    to: &Address,
    list: &DiamondNameListMax200,
) -> Ret<()> {
    check_supported_address(from)?;
    check_supported_address(to)?;
    if from == to {
        return errf!("cannot transfer to self");
    }
    let Some(mut from_owned) = state.diamond_owned(from)? else {
        return errf!("diamond owner record not found");
    };
    from_owned.drop(list)?;
    if from_owned.names.length() > 0 {
        state.diamond_owned_set(from, &from_owned);
    } else {
        state.diamond_owned_del(from);
    }

    let mut to_owned = state.diamond_owned(to)?.unwrap_or_default();
    to_owned.push(list)?;
    state.diamond_owned_set(to, &to_owned);
    Ok(())
}
