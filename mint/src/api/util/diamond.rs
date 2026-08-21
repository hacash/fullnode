use base::{ApiRequest, CoreStateRead};
use field::{Amount, DiamondName};
use sys::ToHex;

use crate::action_diamond::calculate_diamond_visual_gene;

use super::request::{json_string, q_string};

pub(crate) fn diamond_view_item_json(
    name: &DiamondName,
    smelt: &field::DiamondSmelt,
    unit: &str,
) -> String {
    format!(
        concat!(
            "{{",
            "\"name\":{},",
            "\"number\":{},",
            "\"bid_fee\":{},",
            "\"life_gene\":{}",
            "}}"
        ),
        json_string(&name.to_readable()),
        smelt.number.uint(),
        json_string(&smelt.bid_fee.to_unit_string(unit)),
        json_string(&smelt.life_gene.as_ref().to_hex()),
    )
}

pub(crate) fn inscription_items_json(inscripts: &field::Inscripts) -> String {
    let items = inscripts
        .as_list()
        .iter()
        .map(|item| {
            format!(
                "{{\"engraved_type\":{},\"content\":{}}}",
                item.engraved_type.uint(),
                json_string(&item.to_readable_or_hex())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{}]", items)
}

pub(crate) fn string_array_json(items: Vec<String>) -> String {
    let body = items
        .iter()
        .map(|item| json_string(item))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{}]", body)
}

pub(crate) fn diamond_detail_json(
    name: &DiamondName,
    diaobj: &field::DiamondSto,
    smelt: &field::DiamondSmelt,
    unit: &str,
) -> String {
    format!(
        concat!(
            "{{",
            "\"ret\":0,",
            "\"name\":{},",
            "\"belong\":{},",
            "\"inscriptions\":{},",
            "\"inscription_items\":{},",
            "\"number\":{},",
            "\"miner\":{},",
            "\"born\":{{\"height\":{},\"hash\":{}}},",
            "\"prev_hash\":{},",
            "\"bid_fee\":{},",
            "\"average_bid_burn\":{},",
            "\"life_gene\":{},",
            "\"visual_gene\":{}",
            "}}"
        ),
        json_string(&name.to_readable()),
        json_string(&diaobj.address.to_readable()),
        string_array_json(diaobj.inscripts.array()),
        inscription_items_json(&diaobj.inscripts),
        smelt.number.uint(),
        json_string(&smelt.miner_address.to_readable()),
        smelt.born_height.uint(),
        json_string(&smelt.born_hash.as_ref().to_hex()),
        json_string(&smelt.prev_hash.as_ref().to_hex()),
        json_string(&smelt.bid_fee.to_unit_string(unit)),
        smelt.average_bid_burn.uint(),
        json_string(&smelt.life_gene.as_ref().to_hex()),
        json_string(
            &calculate_diamond_visual_gene(name, &smelt.life_gene)
                .as_ref()
                .to_hex()
        ),
    )
}

pub(crate) fn parse_one_diamond_param(req: &ApiRequest, key: &str) -> sys::Ret<DiamondName> {
    let raw = q_string(req, key, "");
    let val = raw.trim();
    if val.is_empty() {
        return sys::errf!("query '{}' cannot be empty", key);
    }
    DiamondName::from_readable(val)
        .map_err(|_| sys::Error::fault(format!("query '{}' diamond name format invalid", key)))
}

pub(crate) fn append_cost_for_one(state: &CoreStateRead, dia: &DiamondName) -> sys::Ret<Amount> {
    let rules = hacash_params::MAINNET_PARAMS.mint_rules.inscription;
    let Some(diaobj) = state.diamond(dia)? else {
        return sys::errf!("cannot find diamond {}", dia.to_readable());
    };
    if diaobj.inscripts.length() >= rules.max_per_diamond {
        return sys::errf!(
            "diamond {} inscriptions full (max {})",
            dia.to_readable(),
            rules.max_per_diamond
        );
    }
    let Some(diasmelt) = state.diamond_smelt(dia)? else {
        return sys::errf!("cannot find diamond {}", dia.to_readable());
    };
    Ok(rules.append_cost(diaobj.inscripts.length(), diasmelt.average_bid_burn.uint()))
}

pub(crate) fn move_cost_for_target(
    state: &CoreStateRead,
    to_diamond: &DiamondName,
) -> sys::Ret<Amount> {
    let rules = hacash_params::MAINNET_PARAMS.mint_rules.inscription;
    let Some(diaobj) = state.diamond(to_diamond)? else {
        return sys::errf!("cannot find diamond {}", to_diamond.to_readable());
    };
    if diaobj.inscripts.length() >= rules.max_per_diamond {
        return sys::errf!(
            "target diamond {} inscriptions full (max {})",
            to_diamond.to_readable(),
            rules.max_per_diamond
        );
    }
    let Some(diasmelt) = state.diamond_smelt(to_diamond)? else {
        return sys::errf!("cannot find diamond {}", to_diamond.to_readable());
    };
    Ok(rules.append_cost(diaobj.inscripts.length(), diasmelt.average_bid_burn.uint()))
}

pub(crate) fn smelt_average_bid(state: &CoreStateRead, dia: &DiamondName) -> sys::Ret<u16> {
    if state.diamond(dia)?.is_none() {
        return sys::errf!("cannot find diamond {}", dia.to_readable());
    }
    let Some(diasmelt) = state.diamond_smelt(dia)? else {
        return sys::errf!("cannot find diamond {}", dia.to_readable());
    };
    Ok(diasmelt.average_bid_burn.uint())
}

pub(crate) fn add_amount(total: &mut Amount, add: &Amount) -> sys::Rerr {
    *total = total.add_mode_u128(add)?;
    Ok(())
}
