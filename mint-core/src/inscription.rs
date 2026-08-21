use std::sync::Arc;

use base::{ActScope, ActionJsonCodec, ActionRef, decode_regular_action};
use field::{
    Amount, BytesW1, DiamondName, DiamondNameListMax200, Encode, Uint1, Uint2, WireAmount,
};
use sys::{Rerr, Ret, errf};

fn wire_rules() -> &'static hacash_params::InscriptionRules {
    &hacash_params::MAINNET_PARAMS.mint_rules.inscription
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaInscPush {
    pub kind: Uint2,
    pub diamonds: DiamondNameListMax200,
    pub protocol_cost: WireAmount,
    pub engraved_type: Uint1,
    pub engraved_content: BytesW1,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaInscClean {
    pub kind: Uint2,
    pub diamonds: DiamondNameListMax200,
    pub protocol_cost: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaInscEdit {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub index: Uint1,
    pub protocol_cost: Amount,
    pub engraved_type: Uint1,
    pub engraved_content: BytesW1,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaInscMove {
    pub kind: Uint2,
    pub from_diamond: DiamondName,
    pub to_diamond: DiamondName,
    pub index: Uint1,
    pub protocol_cost: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaInscDrop {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub index: Uint1,
    pub protocol_cost: Amount,
}

impl DiaInscClean {
    pub const KIND: u16 = 33;

    pub fn new(diamonds: DiamondNameListMax200, protocol_cost: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            diamonds,
            protocol_cost,
        }
    }
}

impl DiaInscPush {
    pub const KIND: u16 = 32;

    pub fn new(
        diamonds: DiamondNameListMax200,
        protocol_cost: WireAmount,
        engraved_type: Uint1,
        engraved_content: BytesW1,
    ) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            diamonds,
            protocol_cost,
            engraved_type,
            engraved_content,
        }
    }
}

impl DiaInscEdit {
    pub const KIND: u16 = 34;

    pub fn new(
        diamond: DiamondName,
        index: Uint1,
        protocol_cost: Amount,
        engraved_type: Uint1,
        engraved_content: BytesW1,
    ) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            diamond,
            index,
            protocol_cost,
            engraved_type,
            engraved_content,
        }
    }
}

impl DiaInscMove {
    pub const KIND: u16 = 35;

    pub fn new(
        from_diamond: DiamondName,
        to_diamond: DiamondName,
        index: Uint1,
        protocol_cost: Amount,
    ) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from_diamond,
            to_diamond,
            index,
            protocol_cost,
        }
    }
}

impl DiaInscDrop {
    pub const KIND: u16 = 36;

    pub fn new(diamond: DiamondName, index: Uint1, protocol_cost: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            diamond,
            index,
            protocol_cost,
        }
    }
}

base::impl_action_facts! {
    DiaInscPush {
        name: "hacd_insc_push",
        scope: ActScope::TOP,
        min_tx_type: 2,
        extra9: |_: &DiaInscPush| true,
        req_sign: |_: &DiaInscPush| vec![],
        as_transfer_like: none,
        description: |this: &DiaInscPush| {
            let mut desc = format!(
                "Inscript {} HACD ({}) with \"{}\"",
                this.diamonds.length(),
                this.diamonds.splitstr(),
                this.engraved_content.to_readable_or_hex()
            );
            if this.protocol_cost.is_positive() {
                desc.push_str(&format!(" cost {} HAC fee", this.protocol_cost.to_fin_string()));
            }
            desc
        },

    }
}

base::impl_action_facts! {
    DiaInscClean {
        name: "hacd_insc_clean",
        scope: ActScope::TOP,
        min_tx_type: 2,
        extra9: |_: &DiaInscClean| true,
        req_sign: |_: &DiaInscClean| vec![],
        as_transfer_like: none,
        description: |this: &DiaInscClean| format!(
            "Clean inscript {} HACD ({}) cost {} HAC fee",
            this.diamonds.length(),
            this.diamonds.splitstr(),
            this.protocol_cost.to_fin_string()
        ),

    }
}

base::impl_action_facts! {
    DiaInscEdit {
        name: "hacd_insc_edit",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaInscEdit| true,
        req_sign: |_: &DiaInscEdit| vec![],
        as_transfer_like: none,
        description: |this: &DiaInscEdit| {
            let mut desc = format!(
                "Edit inscription #{} of HACD {} to \"{}\"",
                this.index.uint(),
                this.diamond.to_readable(),
                this.engraved_content.to_readable_or_hex()
            );
            if this.protocol_cost.is_positive() {
                desc.push_str(&format!(" cost {} HAC fee", this.protocol_cost.to_fin_string()));
            }
            desc
        },

    }
}

base::impl_action_facts! {
    DiaInscMove {
        name: "hacd_insc_move",
        scope: ActScope::AST,
        min_tx_type: 2,
        extra9: |_: &DiaInscMove| true,
        req_sign: |_: &DiaInscMove| vec![],
        as_transfer_like: none,
        description: |this: &DiaInscMove| {
            let mut desc = format!(
                "Move inscription #{} from HACD {} to HACD {}",
                this.index.uint(),
                this.from_diamond.to_readable(),
                this.to_diamond.to_readable()
            );
            if this.protocol_cost.is_positive() {
                desc.push_str(&format!(" cost {} HAC fee", this.protocol_cost.to_fin_string()));
            }
            desc
        },

    }
}

base::impl_action_facts! {
    DiaInscDrop {
        name: "hacd_insc_drop",
        scope: ActScope::TOP,
        min_tx_type: 2,
        extra9: |_: &DiaInscDrop| true,
        req_sign: |_: &DiaInscDrop| vec![],
        as_transfer_like: none,
        description: |this: &DiaInscDrop| format!(
            "Drop inscription #{} from HACD {} cost {} HAC fee",
            this.index.uint(),
            this.diamond.to_readable(),
            this.protocol_cost.to_fin_string()
        ),

    }
}

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

pub fn create_dia_insc_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    match kind {
        DiaInscPush::KIND => decode_regular_action::<DiaInscPush>(buf),
        DiaInscClean::KIND => decode_regular_action::<DiaInscClean>(buf),
        DiaInscEdit::KIND => decode_regular_action::<DiaInscEdit>(buf),
        DiaInscMove::KIND => decode_regular_action::<DiaInscMove>(buf),
        DiaInscDrop::KIND => decode_regular_action::<DiaInscDrop>(buf),
        _ => sys::normalf!("inscription action kind {} not registered", kind),
    }
}

/// JSON decoder for inscription actions. Diamond lists keep the same
/// duplicate/quantity checks as the legacy transaction API.
pub fn decode_dia_insc_json(
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
        DiaInscPush::KIND => {
            let action = DiaInscPush::decode_json(json)?;
            action.diamonds.check()?;
            Ok(Arc::new(action))
        }
        DiaInscClean::KIND => {
            let action = DiaInscClean::decode_json(json)?;
            action.diamonds.check()?;
            Ok(Arc::new(action))
        }
        DiaInscEdit::KIND => decode_action!(DiaInscEdit),
        DiaInscMove::KIND => decode_action!(DiaInscMove),
        DiaInscDrop::KIND => decode_action!(DiaInscDrop),
        _ => sys::normalf!("inscription JSON action kind {} not registered", kind),
    }
}
