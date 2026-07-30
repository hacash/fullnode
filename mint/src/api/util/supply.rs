use base::BaseTotal;

use crate::minter::cumulative_block_reward;
use crate::state::MintTotal;

use super::request::{UNIT_238, hac238_to_unit};

pub(crate) fn supply_json(
    last_height: u64,
    base_total: &BaseTotal,
    mint_total: &MintTotal,
) -> sys::Ret<String> {
    let block_reward = (cumulative_block_reward(last_height) as u128)
        .checked_mul(UNIT_238)
        .ok_or_else(|| sys::Error::fault("block_reward overflow"))?;
    let burned_diamond_bid = mint_total.hacd_bid_burn_238.uint();
    let burned_diamond_insc = mint_total.diamond_insc_burn_238.uint();
    let burned_legacy_tx_extra9 = base_total.tx_fee_burn90_238.uint();
    let burned_vm_ast_gas = base_total.ast_vm_gas_burn_238.uint();
    let burned_asset_issue = mint_total.asset_issue_burn_238.uint();
    let burned_contract_protocol_cost = base_total.contract_protocol_cost_burn_238.uint();
    let burned_blackhole_hac = base_total.blackhole_hac_burn_238.uint();
    let burned_fee = burned_legacy_tx_extra9
        .checked_add(burned_diamond_insc)
        .and_then(|v| v.checked_add(burned_vm_ast_gas))
        .and_then(|v| v.checked_add(burned_asset_issue))
        .and_then(|v| v.checked_add(burned_contract_protocol_cost))
        .ok_or_else(|| sys::Error::fault("burned_fee overflow"))?;
    let current_circulation = block_reward
        .checked_add(mint_total.channel_interest_238.uint() as u128)
        .and_then(|v| v.checked_sub(burned_fee))
        .and_then(|v| v.checked_sub(burned_blackhole_hac))
        .ok_or_else(|| sys::Error::fault("current_circulation overflow"))?;
    let dia_insc_created = mint_total.dia_insc_push.uint();
    let dia_insc_destroyed = mint_total.dia_insc_drop.uint();
    let dia_insc_live = dia_insc_created.saturating_sub(dia_insc_destroyed);
    Ok(format!(
        concat!(
            "{{",
            "\"ret\":0,",
            "\"latest_height\":{},",
            "\"current_circulation\":{},",
            "\"block_reward\":{},",
            "\"burned_fee\":{},",
            "\"minted_diamond\":{},",
            "\"burned_diamond_bid\":{},",
            "\"channel_opening\":{},",
            "\"channel_deposit\":{},",
            "\"channel_interest\":{},",
            "\"channel_open_total\":{},",
            "\"channel_close_total\":{},",
            "\"channel_closed_hac_volume\":{},",
            "\"created_asset\":{},",
            "\"burned_asset_issue\":{},",
            "\"burned_legacy_tx_extra9_fee\":{},",
            "\"burned_ast_vm_gas\":{},",
            "\"tx_fee_pay_total\":{},",
            "\"tx_fee_got_total\":{},",
            "\"diamond_engraved\":{},",
            "\"burned_diamond_insc\":{},",
            "\"diamond_inscription_created\":{},",
            "\"diamond_inscription_destroyed\":{},",
            "\"diamond_inscription_live\":{},",
            "\"diamond_inscription_clean\":{},",
            "\"diamond_inscription_edit\":{},",
            "\"diamond_inscription_move\":{},",
            "\"diamond_inscription_live_diamond\":{},",
            "\"burned_contract_protocol_cost\":{},",
            "\"contract_deploy_count\":{},",
            "\"contract_update_count\":{},",
            "\"contract_charge_bytes_total\":{},",
            "\"burned_blackhole_hac\":{},",
            "\"blackhole_sat_burn\":{},",
            "\"blackhole_asset_burn_count\":{},",
            "\"blackhole_hacd_burn_count\":{},",
            "\"transferred_bitcoin\":0,",
            "\"trsbtc_subsidy\":0",
            "}}"
        ),
        last_height,
        hac238_to_unit(current_circulation),
        hac238_to_unit(block_reward),
        hac238_to_unit(burned_fee),
        mint_total.minted_diamond.uint(),
        hac238_to_unit(burned_diamond_bid),
        mint_total.opening_channel.uint(),
        hac238_to_unit(mint_total.channel_deposit_238.uint()),
        hac238_to_unit(mint_total.channel_interest_238.uint() as u128),
        mint_total.channel_open_total.uint(),
        mint_total.channel_close_total.uint(),
        hac238_to_unit(mint_total.channel_closed_hac_volume_238.uint()),
        mint_total.created_asset.uint(),
        hac238_to_unit(burned_asset_issue),
        hac238_to_unit(burned_legacy_tx_extra9),
        hac238_to_unit(burned_vm_ast_gas),
        hac238_to_unit(base_total.tx_fee_pay_total_238.uint()),
        hac238_to_unit(base_total.tx_fee_got_total_238.uint()),
        mint_total.diamond_engraved.uint(),
        hac238_to_unit(burned_diamond_insc),
        dia_insc_created,
        dia_insc_destroyed,
        dia_insc_live,
        mint_total.dia_insc_clean.uint(),
        mint_total.dia_insc_edit.uint(),
        mint_total.dia_insc_move.uint(),
        mint_total.dia_insc_live_diamond.uint(),
        hac238_to_unit(burned_contract_protocol_cost),
        base_total.contract_deploy_count.uint(),
        base_total.contract_update_count.uint(),
        base_total.contract_charge_bytes_total.uint(),
        hac238_to_unit(burned_blackhole_hac),
        base_total.blackhole_sat_burn.uint(),
        base_total.blackhole_asset_burn_count.uint(),
        base_total.blackhole_hacd_burn_count.uint(),
    ))
}
