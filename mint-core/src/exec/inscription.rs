//! Inscription action execute bodies.

use base::{
    Context, CoreState, DIAMOND_STATUS_NORMAL, check_diamond_status, hac_sub, total_add_amount_238,
    total_add_u8,
};
use field::{
    Address, Amount, BlockHeight, BytesW1, DiamondInscript, DiamondName, DiamondSto, Inscripts,
    Uint1,
};
use sys::{Rerr, Ret, errf};

use crate::inscription::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush,
    check_inscription_content_with_rules, check_protocol_cost,
};
use crate::state::{MintState, MintTotal, with_mint_total};

fn mint_rules(ctx: &dyn Context) -> Ret<hacash_params::InscriptionRules> {
    let profile = ctx.services().execution_profile()?;
    let params = hacash_params::as_hacash_params(profile)
        .ok_or_else(|| sys::Error::fault("standard Hacash params not registered"))?;
    Ok(params.mint_rules.inscription)
}

fn create_diamond_inscript(engraved_type: u8, content: &BytesW1) -> DiamondInscript {
    DiamondInscript {
        engraved_type: Uint1::from(engraved_type),
        content: content.clone(),
    }
}

fn diamond_readable(diamond: &DiamondName) -> String {
    String::from_utf8_lossy(diamond.as_ref()).to_string()
}

fn check_inscription_owner_privakey(owner: &Address, diamond: &DiamondName) -> Rerr {
    if !owner.is_privkey() {
        return errf!(
            "diamond {} owner {:?} must be privakey address",
            diamond_readable(diamond),
            owner
        );
    }
    Ok(())
}

fn check_diamond_status_for_inscription(
    state: &mut CoreState,
    owner: &Address,
    diamond: &DiamondName,
) -> Ret<DiamondSto> {
    check_inscription_owner_privakey(owner, diamond)?;
    check_diamond_status(state, owner, diamond)
}

fn load_diamond_for_inscription(state: &mut CoreState, diamond: &DiamondName) -> Ret<DiamondSto> {
    let Some(diasto) = state.diamond(diamond)? else {
        return errf!("diamond status {} not found", diamond_readable(diamond));
    };
    check_inscription_owner_privakey(&diasto.address, diamond)?;
    if diasto.status != DIAMOND_STATUS_NORMAL {
        return errf!(
            "diamond {} has been mortgaged and cannot operate inscription",
            diamond_readable(diamond)
        );
    }
    Ok(diasto)
}

fn check_inscription_cooldown(
    rules: hacash_params::InscriptionRules,
    prev_engraved_height: u64,
    pending_height: u64,
    diamond: &DiamondName,
) -> Rerr {
    let next_height = prev_engraved_height.saturating_add(rules.cooldown_blocks);
    if next_height > pending_height {
        return errf!(
            "HACD {} inscription cooldown not met, need {} blocks",
            diamond_readable(diamond),
            rules.cooldown_blocks
        );
    }
    Ok(())
}

fn load_diamond_average_bid_burn_mei(state: &mut CoreState, diamond: &DiamondName) -> Ret<u16> {
    let Some(diaslt) = state.diamond_smelt(diamond)? else {
        return errf!("diamond {} not found", diamond_readable(diamond));
    };
    Ok(diaslt.average_bid_burn.uint())
}

fn check_inscription_index(
    diamond: &DiamondName,
    idx: usize,
    insc_len: usize,
    role_prefix: &str,
) -> Rerr {
    if insc_len == 0 {
        if role_prefix.is_empty() {
            return errf!("no inscriptions in diamond {}", diamond_readable(diamond));
        }
        return errf!(
            "no inscriptions in {} HACD {}",
            role_prefix,
            diamond_readable(diamond)
        );
    }
    if idx >= insc_len {
        return errf!(
            "inscription index {} out of range, HACD {} has {} inscriptions",
            idx,
            diamond_readable(diamond),
            insc_len
        );
    }
    Ok(())
}

fn load_diamond_owner_for_inscription_index(
    rules: hacash_params::InscriptionRules,
    state: &mut CoreState,
    diamond: &DiamondName,
    idx: usize,
    pending_height: u64,
) -> Ret<(DiamondSto, Address)> {
    let diasto = load_diamond_for_inscription(state, diamond)?;
    let owner = diasto.address;
    let insc_len = diasto.inscripts.length();
    check_inscription_index(diamond, idx, insc_len, "")?;
    check_inscription_cooldown(
        rules,
        diasto.prev_engraved_height.uint(),
        pending_height,
        diamond,
    )?;
    Ok((diasto, owner))
}

fn add_dia_insc_u8(
    state: &mut CoreState,
    field: fn(&mut MintTotal) -> &mut field::Uint8,
    add: u64,
    name: &str,
) -> Rerr {
    if add == 0 {
        return Ok(());
    }
    with_mint_total(&mut MintState::wrap(&mut *state.0), |ttcount| {
        total_add_u8(field(ttcount), add, name)
    })
}

fn add_diamond_insc_burn_count(state: &mut CoreState, pfee: &Amount) -> Rerr {
    if !pfee.is_positive() {
        return Ok(());
    }
    with_mint_total(&mut MintState::wrap(&mut *state.0), |ttcount| {
        total_add_amount_238(
            &mut ttcount.diamond_insc_burn_238,
            pfee,
            "diamond_insc_burn_238",
        )
    })
}

fn saturating_sub_dia_insc_live_diamond(state: &mut CoreState, sub: u64) -> Rerr {
    if sub == 0 {
        return Ok(());
    }
    with_mint_total(&mut MintState::wrap(&mut *state.0), |ttcount| {
        let next = ttcount.dia_insc_live_diamond.uint().saturating_sub(sub);
        ttcount.dia_insc_live_diamond = field::Uint8::from_checked(next)
            .ok_or_else(|| sys::Error::fault("dia_insc_live_diamond overflow"))?;
        Ok(())
    })
}

pub fn engraved_one_diamond(
    rules: hacash_params::InscriptionRules,
    pending_height: u64,
    state: &mut CoreState,
    addr: &Address,
    diamond: &DiamondName,
    engraved_type: u8,
    content: &BytesW1,
) -> Ret<Amount> {
    let mut diasto = check_diamond_status_for_inscription(state, addr, diamond)?;
    check_inscription_cooldown(
        rules,
        diasto.prev_engraved_height.uint(),
        pending_height,
        diamond,
    )?;
    let haveng = diasto.inscripts.length();
    if haveng >= rules.max_per_diamond {
        return errf!(
            "maximum inscriptions for one diamond is {}",
            rules.max_per_diamond
        );
    }
    let Some(diaslt) = state.diamond_smelt(diamond)? else {
        return errf!("diamond {} not found", diamond_readable(diamond));
    };
    let cost = rules.append_cost(haveng, diaslt.average_bid_burn.uint());
    diasto.prev_engraved_height = BlockHeight::from(pending_height);
    diasto
        .inscripts
        .push(create_diamond_inscript(engraved_type, content))?;
    state.diamond_set(diamond, &diasto);
    Ok(cost)
}

pub fn engraved_clean_one_diamond(
    state: &mut CoreState,
    addr: &Address,
    diamond: &DiamondName,
) -> Ret<Amount> {
    let mut diasto = check_diamond_status_for_inscription(state, addr, diamond)?;
    let Some(diaslt) = state.diamond_smelt(diamond)? else {
        return errf!("diamond {} not found", diamond_readable(diamond));
    };
    if diasto.inscripts.length() == 0 {
        return errf!(
            "cannot find any inscriptions in HACD {}",
            diamond_readable(diamond)
        );
    }
    let cost = Amount::mei(diaslt.average_bid_burn.uint() as u64);
    diasto.prev_engraved_height = BlockHeight::from(0);
    diasto.inscripts = Inscripts::default();
    state.diamond_set(diamond, &diasto);
    Ok(cost)
}

fn diamond_inscription_push(this: &DiaInscPush, ctx: &mut dyn Context) -> Rerr {
    let rules = mint_rules(ctx)?;
    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pfee = this.protocol_cost.amount();
    check_protocol_cost(pfee)?;
    ctx.check_sign(&main_addr)?;
    this.diamonds.check()?;
    check_inscription_content_with_rules(
        &rules,
        this.engraved_type.uint(),
        &this.engraved_content,
    )?;

    let mut ttcost = Amount::zero();
    let mut live_diamond_add = 0u64;
    {
        let mut state = CoreState::wrap(ctx.layer());
        for dia in this.diamonds.as_list() {
            let prev_len = load_diamond_for_inscription(&mut state, dia)?
                .inscripts
                .length();
            let cc = engraved_one_diamond(
                rules,
                env.block.height,
                &mut state,
                &main_addr,
                dia,
                this.engraved_type.uint(),
                &this.engraved_content,
            )?;
            ttcost = ttcost.add_mode_u128(&cc)?;
            if prev_len == 0 {
                live_diamond_add += 1;
            }
        }
        if pfee < &ttcost {
            return errf!(
                "diamond inscription cost expected {:?} but got {:?}",
                ttcost,
                pfee
            );
        }
        add_dia_insc_u8(
            &mut state,
            |t| &mut t.diamond_engraved,
            1,
            "diamond_engraved",
        )?;
        add_dia_insc_u8(
            &mut state,
            |t| &mut t.dia_insc_push,
            this.diamonds.length() as u64,
            "dia_insc_push",
        )?;
        add_diamond_insc_burn_count(&mut state, pfee)?;
        add_dia_insc_u8(
            &mut state,
            |t| &mut t.dia_insc_live_diamond,
            live_diamond_add,
            "dia_insc_live_diamond",
        )?;
    }
    if pfee.is_positive() {
        hac_sub(ctx, &main_addr, pfee)?;
    }
    Ok(())
}

fn diamond_inscription_clean(this: &DiaInscClean, ctx: &mut dyn Context) -> Rerr {
    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pfee = &this.protocol_cost;
    check_protocol_cost(pfee)?;
    ctx.check_sign(&main_addr)?;
    this.diamonds.check()?;

    let mut ttcost = Amount::zero();
    let mut cleared_entries = 0u64;
    let mut cleared_diamonds = 0u64;
    {
        let mut state = CoreState::wrap(ctx.layer());
        for dia in this.diamonds.as_list() {
            let prev_len = load_diamond_for_inscription(&mut state, dia)?
                .inscripts
                .length();
            let cc = engraved_clean_one_diamond(&mut state, &main_addr, dia)?;
            ttcost = ttcost.add_mode_u128(&cc)?;
            cleared_entries += prev_len as u64;
            if prev_len > 0 {
                cleared_diamonds += 1;
            }
        }
        if pfee < &ttcost {
            return errf!(
                "diamond inscription cost expected {:?} but got {:?}",
                ttcost,
                pfee
            );
        }
        add_diamond_insc_burn_count(&mut state, pfee)?;
        add_dia_insc_u8(&mut state, |t| &mut t.dia_insc_clean, 1, "dia_insc_clean")?;
        add_dia_insc_u8(
            &mut state,
            |t| &mut t.dia_insc_drop,
            cleared_entries,
            "dia_insc_drop",
        )?;
        saturating_sub_dia_insc_live_diamond(&mut state, cleared_diamonds)?;
    }
    if pfee.is_positive() {
        hac_sub(ctx, &main_addr, pfee)?;
    }
    Ok(())
}

fn diamond_inscription_edit(this: &DiaInscEdit, ctx: &mut dyn Context) -> Rerr {
    let rules = mint_rules(ctx)?;
    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pfee = &this.protocol_cost;
    check_protocol_cost(pfee)?;
    ctx.check_sign(&main_addr)?;
    check_inscription_content_with_rules(
        &rules,
        this.engraved_type.uint(),
        &this.engraved_content,
    )?;
    let idx = this.index.uint() as usize;
    let (mut diasto, owner) = {
        let mut state = CoreState::wrap(ctx.layer());
        load_diamond_owner_for_inscription_index(
            rules,
            &mut state,
            &this.diamond,
            idx,
            env.block.height,
        )?
    };
    ctx.check_sign(&owner)?;
    {
        let mut state = CoreState::wrap(ctx.layer());
        let avg_bid_burn_mei = load_diamond_average_bid_burn_mei(&mut state, &this.diamond)?;
        let cost = rules.edit_cost(avg_bid_burn_mei);
        if pfee < &cost {
            return errf!(
                "inscription edit cost expected {:?} but got {:?}",
                cost,
                pfee
            );
        }
        diasto.inscripts.as_mut()[idx] =
            create_diamond_inscript(this.engraved_type.uint(), &this.engraved_content);
        diasto.prev_engraved_height = BlockHeight::from(env.block.height);
        state.diamond_set(&this.diamond, &diasto);
        add_diamond_insc_burn_count(&mut state, pfee)?;
        add_dia_insc_u8(&mut state, |t| &mut t.dia_insc_edit, 1, "dia_insc_edit")?;
    }
    if pfee.is_positive() {
        hac_sub(ctx, &main_addr, pfee)?;
    }
    Ok(())
}

fn diamond_inscription_move(this: &DiaInscMove, ctx: &mut dyn Context) -> Rerr {
    let rules = mint_rules(ctx)?;
    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pfee = &this.protocol_cost;
    check_protocol_cost(pfee)?;
    let idx = this.index.uint() as usize;
    if this.from_diamond == this.to_diamond {
        return errf!("source and target HACD cannot be the same");
    }
    let (mut from_sto, mut to_sto, from_owner, to_owner, move_cost, from_len, to_len) = {
        let mut state = CoreState::wrap(ctx.layer());
        let from_sto = load_diamond_for_inscription(&mut state, &this.from_diamond)?;
        let from_owner = from_sto.address;
        let from_len = from_sto.inscripts.length();
        check_inscription_index(&this.from_diamond, idx, from_len, "source ")?;
        check_inscription_cooldown(
            rules,
            from_sto.prev_engraved_height.uint(),
            env.block.height,
            &this.from_diamond,
        )?;
        let to_sto = load_diamond_for_inscription(&mut state, &this.to_diamond)?;
        let to_owner = to_sto.address;
        check_inscription_cooldown(
            rules,
            to_sto.prev_engraved_height.uint(),
            env.block.height,
            &this.to_diamond,
        )?;
        if to_sto.inscripts.length() >= rules.max_per_diamond {
            return errf!(
                "target HACD {} inscriptions full (max {})",
                diamond_readable(&this.to_diamond),
                rules.max_per_diamond
            );
        }
        let to_len = to_sto.inscripts.length();
        let avg_bid_burn_mei = load_diamond_average_bid_burn_mei(&mut state, &this.to_diamond)?;
        let move_cost = rules.append_cost(to_len, avg_bid_burn_mei);
        (
            from_sto, to_sto, from_owner, to_owner, move_cost, from_len, to_len,
        )
    };
    ctx.check_sign(&from_owner)?;
    ctx.check_sign(&to_owner)?;
    if pfee < &move_cost {
        return errf!(
            "inscription move cost expected {:?} but got {:?}",
            move_cost,
            pfee
        );
    }
    {
        let inscript = from_sto.inscripts.as_list()[idx].clone();
        from_sto.inscripts.drop(idx)?;
        from_sto.prev_engraved_height = BlockHeight::from(env.block.height);
        let mut state = CoreState::wrap(ctx.layer());
        state.diamond_set(&this.from_diamond, &from_sto);
        to_sto.inscripts.push(inscript)?;
        to_sto.prev_engraved_height = BlockHeight::from(env.block.height);
        state.diamond_set(&this.to_diamond, &to_sto);
        add_diamond_insc_burn_count(&mut state, pfee)?;
        add_dia_insc_u8(&mut state, |t| &mut t.dia_insc_move, 1, "dia_insc_move")?;
        if from_len == 1 {
            saturating_sub_dia_insc_live_diamond(&mut state, 1)?;
        }
        if to_len == 0 {
            add_dia_insc_u8(
                &mut state,
                |t| &mut t.dia_insc_live_diamond,
                1,
                "dia_insc_live_diamond",
            )?;
        }
    }
    if pfee.is_positive() {
        hac_sub(ctx, &main_addr, pfee)?;
    }
    Ok(())
}

fn diamond_inscription_drop(this: &DiaInscDrop, ctx: &mut dyn Context) -> Rerr {
    let rules = mint_rules(ctx)?;
    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pfee = &this.protocol_cost;
    check_protocol_cost(pfee)?;
    ctx.check_sign(&main_addr)?;
    let idx = this.index.uint() as usize;
    let (mut diasto, owner) = {
        let mut state = CoreState::wrap(ctx.layer());
        load_diamond_owner_for_inscription_index(
            rules,
            &mut state,
            &this.diamond,
            idx,
            env.block.height,
        )?
    };
    ctx.check_sign(&owner)?;
    {
        let mut state = CoreState::wrap(ctx.layer());
        let avg_bid_burn_mei = load_diamond_average_bid_burn_mei(&mut state, &this.diamond)?;
        let prev_len = diasto.inscripts.length();
        let cost = rules.drop_cost(avg_bid_burn_mei);
        if pfee < &cost {
            return errf!(
                "inscription drop cost expected {:?} but got {:?}",
                cost,
                pfee
            );
        }
        diasto.inscripts.drop(idx)?;
        diasto.prev_engraved_height = BlockHeight::from(env.block.height);
        state.diamond_set(&this.diamond, &diasto);
        add_diamond_insc_burn_count(&mut state, pfee)?;
        add_dia_insc_u8(&mut state, |t| &mut t.dia_insc_drop, 1, "dia_insc_drop")?;
        if prev_len == 1 {
            saturating_sub_dia_insc_live_diamond(&mut state, 1)?;
        }
    }
    if pfee.is_positive() {
        hac_sub(ctx, &main_addr, pfee)?;
    }
    Ok(())
}

base::impl_action_execute! {
    DiaInscPush {
        (self, ctx) {
            diamond_inscription_push(self, ctx)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    DiaInscClean {
        (self, ctx) {
            diamond_inscription_clean(self, ctx)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    DiaInscEdit {
        (self, ctx) {
            diamond_inscription_edit(self, ctx)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    DiaInscMove {
        (self, ctx) {
            diamond_inscription_move(self, ctx)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    DiaInscDrop {
        (self, ctx) {
            diamond_inscription_drop(self, ctx)?;
            Ok(vec![])
        }
    }
}
