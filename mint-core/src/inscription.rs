use std::sync::Arc;

use base::{ActionJsonCodec, ActionRef};
use field::{Amount, BytesW1, DiamondName, DiamondNameListMax200, Encode, Uint1, WireAmount};
use sys::{Rerr, Ret, errf};

fn wire_rules() -> &'static hacash_params::InscriptionRules {
    &hacash_params::MAINNET_PARAMS.mint_rules.inscription
}

base::action_simple! { HacdInscPush, 32, 2, TOP, {
    diamonds: DiamondNameListMax200,
    protocol_cost: WireAmount,
    engraved_type: Uint1,
    engraved_content: BytesW1
}, this, {
    extra9: true,
    description: {
        let mut desc = format!("Inscript {} HACD ({}) with \"{}\"", this.diamonds.length(), this.diamonds.splitstr(), this.engraved_content.to_readable_or_hex());
        if this.protocol_cost.is_positive() { desc.push_str(&format!(" cost {} HAC fee", this.protocol_cost.to_fin_string())); }
        desc
    }
}}

base::action_simple! { HacdInscClean, 33, 2, TOP, {
    diamonds: DiamondNameListMax200,
    protocol_cost: Amount
}, this, {
    extra9: true,
    description: format!("Clean inscript {} HACD ({}) cost {} HAC fee", this.diamonds.length(), this.diamonds.splitstr(), this.protocol_cost.to_fin_string())
}}

base::action_simple! { HacdInscEdit, 34, 2, CALL, {
    diamond: DiamondName,
    index: Uint1,
    protocol_cost: Amount,
    engraved_type: Uint1,
    engraved_content: BytesW1
}, this, {
    extra9: true,
    description: {
        let mut desc = format!("Edit inscription #{} of HACD {} to \"{}\"", this.index.uint(), this.diamond.to_readable(), this.engraved_content.to_readable_or_hex());
        if this.protocol_cost.is_positive() { desc.push_str(&format!(" cost {} HAC fee", this.protocol_cost.to_fin_string())); }
        desc
    }
}}

base::action_simple! { HacdInscMove, 35, 2, AST, {
    from_diamond: DiamondName,
    to_diamond: DiamondName,
    index: Uint1,
    protocol_cost: Amount
}, this, {
    extra9: true,
    description: {
        let mut desc = format!("Move inscription #{} from HACD {} to HACD {}", this.index.uint(), this.from_diamond.to_readable(), this.to_diamond.to_readable());
        if this.protocol_cost.is_positive() { desc.push_str(&format!(" cost {} HAC fee", this.protocol_cost.to_fin_string())); }
        desc
    }
}}

base::action_simple! { HacdInscDrop, 36, 2, TOP, {
    diamond: DiamondName,
    index: Uint1,
    protocol_cost: Amount
}, this, {
    extra9: true,
    description: format!("Drop inscription #{} from HACD {} cost {} HAC fee", this.index.uint(), this.diamond.to_readable(), this.protocol_cost.to_fin_string())
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
    check_inscription_content_with_rules(wire_rules(), engraved_type, content)
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
    if index as usize >= wire_rules().max_per_diamond {
        return errf!(
            "inscription index out of range, max per diamond is {}",
            wire_rules().max_per_diamond
        );
    }
    Ok(())
}

pub fn calc_append_inscription_protocol_cost(
    cur_inscriptions: usize,
    average_bid_burn_mei: u16,
) -> Amount {
    wire_rules().append_cost(cur_inscriptions, average_bid_burn_mei)
}

pub fn calc_move_inscription_protocol_cost(
    target_cur_inscriptions: usize,
    average_bid_burn_mei: u16,
) -> Amount {
    calc_append_inscription_protocol_cost(target_cur_inscriptions, average_bid_burn_mei)
}

pub fn calc_edit_inscription_protocol_cost(average_bid_burn_mei: u16) -> Amount {
    wire_rules().edit_cost(average_bid_burn_mei)
}

pub fn calc_drop_inscription_protocol_cost(average_bid_burn_mei: u16) -> Amount {
    wire_rules().drop_cost(average_bid_burn_mei)
}

/// JSON decoder for inscription actions. Diamond lists keep the same
/// duplicate/quantity checks as the legacy transaction API.
pub fn decode_hacd_insc_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    macro_rules! decode_action {
        ($ty:ty) => {{
            let action = <$ty as ActionJsonCodec>::decode_json(json)?;
            Ok(Arc::new(action) as ActionRef)
        }};
    }
    match kind {
        HacdInscPush::KIND => {
            let action = HacdInscPush::decode_json(json)?;
            action.diamonds.check()?;
            Ok(Arc::new(action))
        }
        HacdInscClean::KIND => {
            let action = HacdInscClean::decode_json(json)?;
            action.diamonds.check()?;
            Ok(Arc::new(action))
        }
        HacdInscEdit::KIND => decode_action!(HacdInscEdit),
        HacdInscMove::KIND => decode_action!(HacdInscMove),
        HacdInscDrop::KIND => decode_action!(HacdInscDrop),
        _ => sys::normalf!("inscription JSON action kind {} not registered", kind),
    }
}
