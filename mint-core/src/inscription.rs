use field::descfmt::user_text;
use field::{Amount, BytesW1, DiamondName, DiamondNameListMax200, Encode, Uint1, WireAmount};
use sys::{Rerr, errf};

/// Shared protocol-fee segment: the fee is only mentioned when it is actually
/// charged — a zero cost imposes no burn, so rendering it would claim a
/// payment that does not happen.
fn protocol_fee_segment(protocol_cost: &Amount) -> String {
    if protocol_cost.is_positive() {
        format!("; pay {} HAC protocol fee", protocol_cost.to_hac_string())
    } else {
        String::new()
    }
}

fn consensus_rules() -> &'static hacash_params::InscriptionRules {
    &hacash_params::MAINNET_PARAMS.mint_rules.inscription
}

base::action_simple! { HacdInscPush, 32, 2, TOP, {
    diamonds: DiamondNameListMax200,
    protocol_cost: WireAmount,
    engraved_type: Uint1,
    engraved_content: BytesW1
}, this, {
    extra9: true,
    description: format!(
        "Append inscription type {} {} to {} HACD ({}){}",
        this.engraved_type.uint(),
        user_text(this.engraved_content.as_ref()),
        this.diamonds.length(),
        this.diamonds.splitstr(),
        protocol_fee_segment(&this.protocol_cost)
    )
}}

base::action_simple! { HacdInscClean, 33, 2, TOP, {
    diamonds: DiamondNameListMax200,
    protocol_cost: Amount
}, this, {
    extra9: true,
    description: format!(
        "Remove all inscriptions from {} HACD ({}){}",
        this.diamonds.length(),
        this.diamonds.splitstr(),
        protocol_fee_segment(&this.protocol_cost)
    )
}}

base::action_simple! { HacdInscEdit, 34, 2, CALL, {
    diamond: DiamondName,
    index: Uint1,
    protocol_cost: Amount,
    engraved_type: Uint1,
    engraved_content: BytesW1
}, this, {
    extra9: true,
    description: format!(
        "Replace inscription at index {} (0-based) of HACD {} with type {} {}{}",
        this.index.uint(),
        this.diamond.to_readable(),
        this.engraved_type.uint(),
        user_text(this.engraved_content.as_ref()),
        protocol_fee_segment(&this.protocol_cost)
    )
}}

base::action_simple! { HacdInscMove, 35, 2, AST, {
    from_diamond: DiamondName,
    to_diamond: DiamondName,
    index: Uint1,
    protocol_cost: Amount
}, this, {
    extra9: true,
    description: format!(
        "Move inscription at index {} (0-based) from HACD {} to HACD {}{}",
        this.index.uint(),
        this.from_diamond.to_readable(),
        this.to_diamond.to_readable(),
        protocol_fee_segment(&this.protocol_cost)
    )
}}

base::action_simple! { HacdInscDrop, 36, 2, TOP, {
    diamond: DiamondName,
    index: Uint1,
    protocol_cost: Amount
}, this, {
    extra9: true,
    description: format!(
        "Delete inscription at index {} (0-based) from HACD {}{}",
        this.index.uint(),
        this.diamond.to_readable(),
        protocol_fee_segment(&this.protocol_cost)
    )
}}

pub fn check_protocol_cost(pfee: &Amount) -> Rerr {
    if pfee.is_negative() {
        return errf!("protocol cost cannot be negative");
    }
    if pfee.size() > 4 {
        return errf!("protocol cost amount size cannot exceed 4 bytes");
    }
    Ok(())
}

pub fn check_inscription_content(engraved_type: u8, content: &BytesW1) -> Rerr {
    check_inscription_content_with_rules(consensus_rules(), engraved_type, content)
}

pub fn check_inscription_content_with_rules(
    rules: &hacash_params::InscriptionRules,
    engraved_type: u8,
    content: &BytesW1,
) -> Rerr {
    let insc_len = content.length();
    if insc_len == 0 {
        return errf!("engraved content cannot be empty");
    }
    if insc_len > rules.content_max_bytes {
        return errf!(
            "engraved content size cannot exceed {} bytes",
            rules.content_max_bytes
        );
    }
    if engraved_type <= rules.readable_type_max && !sys::check_readable_string(content.as_ref()) {
        return errf!("engraved content must be a readable string");
    }
    Ok(())
}

/// Build-time index range check for inscription edit/move/drop: enforces only the
/// protocol maximum; the executor validates against the diamond's live list.
pub fn check_inscription_index_max(index: u8) -> Rerr {
    if index as usize >= consensus_rules().max_per_diamond {
        return errf!(
            "inscription index out of range, max per diamond is {}",
            consensus_rules().max_per_diamond
        );
    }
    Ok(())
}

pub fn calc_append_inscription_protocol_cost(
    cur_inscriptions: usize,
    average_bid_burn_mei: u16,
) -> Amount {
    consensus_rules().append_cost(cur_inscriptions, average_bid_burn_mei)
}

pub fn calc_move_inscription_protocol_cost(
    target_cur_inscriptions: usize,
    average_bid_burn_mei: u16,
) -> Amount {
    calc_append_inscription_protocol_cost(target_cur_inscriptions, average_bid_burn_mei)
}

pub fn calc_edit_inscription_protocol_cost(average_bid_burn_mei: u16) -> Amount {
    consensus_rules().edit_cost(average_bid_burn_mei)
}

pub fn calc_drop_inscription_protocol_cost(average_bid_burn_mei: u16) -> Amount {
    consensus_rules().drop_cost(average_bid_burn_mei)
}
