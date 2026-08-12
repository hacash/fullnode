use base::{ApiExecCtx, ApiRequest, ApiResponse, CoreStateRead, Transaction};

use crate::action::diamond_insc::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush,
};
use crate::action::util::pickout_diamond_mint_action;
use crate::api::util::*;

use field::{Amount, DiamondName, DiamondNameListMax200, DiamondNumber};
use sys::ToHex;

pub(crate) fn diamond_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let mut name = q_string(&req, "name", "");
    let number = req.query_u64("number").unwrap_or(0) as u32;

    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    if number > 0 {
        let dian = match DiamondNumber::from_usize(number as usize) {
            Ok(v) => v,
            Err(_) => return api_error("diamond number error"),
        };
        let Some(dian) = state.diamond_name(&dian) else {
            if !ctx.engine.validate_optimistic(start_epoch) {
                return api_error("state changed");
            }
            return api_error("cannot find diamond");
        };
        name = dian.to_readable();
    } else if !DiamondName::is_valid(name.as_bytes()) {
        return api_error("invalid diamond name");
    }

    let dian = match DiamondName::from_readable(name.as_bytes()) {
        Ok(v) => v,
        Err(_) => return api_error("invalid diamond name"),
    };
    let Some(diaobj) = state.diamond(&dian) else {
        if !ctx.engine.validate_optimistic(start_epoch) {
            return api_error("state changed");
        }
        return api_error("cannot find diamond");
    };
    let Some(diasmelt) = state.diamond_smelt(&dian) else {
        if !ctx.engine.validate_optimistic(start_epoch) {
            return api_error("state changed");
        }
        return api_error("cannot find diamond");
    };
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    ApiResponse::json(diamond_detail_json(&dian, &diaobj, &diasmelt, &unit))
}

pub(crate) fn diamond_views_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let mut limit = q_i64(&req, "limit", 20);
    let page = q_i64(&req, "page", 1);
    let start = q_i64(&req, "start", i64::MAX);
    let desc = q_bool(&req, "desc", false);
    let name = q_string(&req, "name", "");

    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    let lastdianum = state.latest_diamond().unwrap_or_default().number.uint() as i64;
    if limit > 200 {
        limit = 200;
    }

    let mut list = Vec::new();
    if name.len() >= DiamondName::SIZE {
        let names = match DiamondNameListMax200::from_readable(&name) {
            Ok(v) => v,
            Err(_) => return api_error("invalid diamond name"),
        };
        for dian in names.as_list() {
            if state.diamond(dian).is_none() {
                continue;
            }
            if let Some(smelt) = state.diamond_smelt(dian) {
                list.push(diamond_view_item_json(dian, &smelt, &unit));
            }
        }
    } else {
        for id in get_id_range(lastdianum, page, limit, start, desc) {
            let dianum = match DiamondNumber::from_usize(id as usize) {
                Ok(v) => v,
                Err(_) => return api_error("diamond number error"),
            };
            let Some(dian) = state.diamond_name(&dianum) else {
                continue;
            };
            if state.diamond(&dian).is_none() {
                continue;
            }
            if let Some(smelt) = state.diamond_smelt(&dian) {
                list.push(diamond_view_item_json(&dian, &smelt, &unit));
            }
        }
    }
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    api_data_list_field("latest_number", lastdianum, list)
}

pub(crate) fn diamond_inscription_protocol_cost_impl(
    ctx: &ApiExecCtx,
    req: ApiRequest,
    force_action: Option<&str>,
) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let action_key = force_action
        .map(|v| v.to_owned())
        .unwrap_or_else(|| q_string(&req, "action", "append").to_lowercase());
    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    let mut cost = Amount::zero();

    let res = match action_key.as_str() {
        "append" => {
            let name = q_string(&req, "name", "");
            let names = match DiamondNameListMax200::from_readable(&name) {
                Ok(v) => v,
                Err(_) => return api_error("diamond name format or count error"),
            };
            (|| -> sys::Rerr {
                for dia in names.as_list() {
                    let one = append_cost_for_one(&state, dia)?;
                    add_amount(&mut cost, &one)?;
                }
                Ok(())
            })()
        }
        "move" => {
            let to_diamond = match parse_one_diamond_param(&req, "to") {
                Ok(v) => v,
                Err(e) => return api_error(&e.to_string()),
            };
            let from_raw = q_string(&req, "from", "");
            if !from_raw.trim().is_empty() {
                let from_diamond = match parse_one_diamond_param(&req, "from") {
                    Ok(v) => v,
                    Err(e) => return api_error(&e.to_string()),
                };
                if from_diamond == to_diamond {
                    return api_error("source and target HACD cannot be the same");
                }
            }
            match move_cost_for_target(&state, &to_diamond) {
                Ok(v) => {
                    cost = v;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        "edit" => {
            let dia = match parse_one_diamond_param(&req, "name") {
                Ok(v) => v,
                Err(e) => return api_error(&e.to_string()),
            };
            match smelt_average_bid(&state, &dia) {
                Ok(v) => {
                    cost = crate::action::diamond_insc::calc_edit_inscription_protocol_cost(v);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        "drop" => {
            let dia = match parse_one_diamond_param(&req, "name") {
                Ok(v) => v,
                Err(e) => return api_error(&e.to_string()),
            };
            match smelt_average_bid(&state, &dia) {
                Ok(v) => {
                    cost = crate::action::diamond_insc::calc_drop_inscription_protocol_cost(v);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ => return api_error("action must be append/move/edit/drop"),
    };
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    if let Err(e) = res {
        return api_error(&e.to_string());
    }
    ApiResponse::json(format!(
        "{{\"ret\":0,\"action\":{},\"cost\":{}}}",
        json_string(&action_key),
        json_string(&cost.to_unit_string(&unit))
    ))
}

pub(crate) fn diamond_inscription_protocol_cost_handler(
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    diamond_inscription_protocol_cost_impl(ctx, req, None)
}

pub(crate) fn diamond_inscription_protocol_cost_append_handler(
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    diamond_inscription_protocol_cost_impl(ctx, req, Some("append"))
}

pub(crate) fn diamond_inscription_protocol_cost_move_handler(
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    diamond_inscription_protocol_cost_impl(ctx, req, Some("move"))
}

pub(crate) fn diamond_inscription_protocol_cost_edit_handler(
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    diamond_inscription_protocol_cost_impl(ctx, req, Some("edit"))
}

pub(crate) fn diamond_inscription_protocol_cost_drop_handler(
    ctx: &ApiExecCtx,
    req: ApiRequest,
) -> ApiResponse {
    diamond_inscription_protocol_cost_impl(ctx, req, Some("drop"))
}

pub(crate) fn diamond_bidding_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let limit = req.query_usize("limit").unwrap_or(20);
    let number = req.query_u64("number").unwrap_or(0) as u32;
    let since = q_bool(&req, "since", false);

    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = CoreStateRead::wrap(snapshot.view());
    let lastdia = state.latest_diamond().unwrap_or_default();
    let txpool = ctx.node.txpool();
    let mut datalist = Vec::new();

    let _ = txpool.iter(crate::HacashConsensus::TX_GROUP_DIAMOND_MINT, &mut |a| {
        if datalist.len() >= limit {
            return false;
        }
        let txhx = a.hash();
        let txr = a.tx();
        let Some(diamtact) = pickout_diamond_mint_action(txr) else {
            return true;
        };
        let act = &diamtact.d;
        if number > 0 && number != act.number.uint() {
            return true;
        }
        let mut fields = vec![
            format!("\"tx\":{}", json_string(&txhx.as_ref().to_hex())),
            format!("\"fee\":{}", json_string(&txr.fee().to_unit_string(&unit))),
            format!("\"bid\":{}", json_string(&txr.main().to_readable())),
            format!("\"name\":{}", json_string(&act.diamond.to_readable())),
            format!("\"belong\":{}", json_string(&act.address.to_readable())),
        ];
        if number == 0 {
            fields.push(format!("\"number\":{}", act.number.uint()));
        }
        datalist.push(format!("{{{}}}", fields.join(",")));
        true
    });

    let mut fields = vec![
        format!("\"number\":{}", lastdia.number.uint() + 1),
        format!("\"list\":[{}]", datalist.join(",")),
    ];
    if since {
        if let Ok(blk) = load_block_by_key(ctx, &lastdia.born_height.uint().to_string()) {
            fields.push(format!("\"since\":{}", blk.block().timestamp()));
        }
    }
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    ApiResponse::json(format!("{{\"ret\":0,{}}}", fields.join(",")))
}

fn push_diamond_engrave_item(
    datalist: &mut Vec<String>,
    txhx: &field::Hash,
    with_tx_hash: bool,
    mut fields: Vec<String>,
) {
    if with_tx_hash {
        fields.push(format!(
            "\"tx_hash\":{}",
            json_string(&txhx.as_ref().to_hex())
        ));
    }
    datalist.push(format!("{{{}}}", fields.join(",")));
}

pub(crate) fn diamond_engrave_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let height = req.query_u64("height").unwrap_or(0);
    let tx_hash = q_bool(&req, "tx_hash", false);
    let txposi = q_i64(&req, "txposi", -1);

    let blkpkg = match load_block_by_key(ctx, &height.to_string()) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let trs = blkpkg.block().transactions();
    if trs.is_empty() {
        return api_error("transaction length invalid");
    }
    if txposi >= 0 && txposi >= trs.len() as i64 - 1 {
        return api_error("txposi overflow");
    }

    let mut datalist = Vec::new();
    let mut pick_engrave = |tx: &dyn Transaction| {
        let txhx = tx.hash();
        for act in tx.actions() {
            if let Some(a) = act.as_any().downcast_ref::<DiaInscPush>() {
                push_diamond_engrave_item(
                    &mut datalist,
                    &txhx,
                    tx_hash,
                    vec![
                        format!("\"action\":{}", json_string("inscription")),
                        format!("\"diamonds\":{}", json_string(&a.diamonds.readable())),
                        format!(
                            "\"inscription\":{}",
                            json_string(&a.engraved_content.to_readable_or_hex())
                        ),
                        format!("\"engraved_type\":{}", a.engraved_type.uint()),
                        format!(
                            "\"protocol_cost\":{}",
                            json_string(&a.protocol_cost.to_fin_string())
                        ),
                    ],
                );
            } else if let Some(a) = act.as_any().downcast_ref::<DiaInscClean>() {
                push_diamond_engrave_item(
                    &mut datalist,
                    &txhx,
                    tx_hash,
                    vec![
                        format!("\"action\":{}", json_string("clear")),
                        // Documented clear marker (fullnode_api_doc_v2 §3.6).
                        format!("\"clear\":true"),
                        format!("\"diamonds\":{}", json_string(&a.diamonds.readable())),
                        format!(
                            "\"protocol_cost\":{}",
                            json_string(&a.protocol_cost.to_fin_string())
                        ),
                    ],
                );
            } else if let Some(a) = act.as_any().downcast_ref::<DiaInscMove>() {
                let from = a.from_diamond.to_readable();
                let to = a.to_diamond.to_readable();
                push_diamond_engrave_item(
                    &mut datalist,
                    &txhx,
                    tx_hash,
                    vec![
                        format!("\"action\":{}", json_string("move")),
                        format!("\"diamonds\":{}", json_string(&format!("{}{}", from, to))),
                        format!("\"index\":{}", a.index.uint() as u64),
                        format!(
                            "\"protocol_cost\":{}",
                            json_string(&a.protocol_cost.to_fin_string())
                        ),
                    ],
                );
            } else if let Some(a) = act.as_any().downcast_ref::<DiaInscDrop>() {
                push_diamond_engrave_item(
                    &mut datalist,
                    &txhx,
                    tx_hash,
                    vec![
                        format!("\"action\":{}", json_string("drop")),
                        format!("\"diamonds\":{}", json_string(&a.diamond.to_readable())),
                        format!("\"index\":{}", a.index.uint() as u64),
                        format!(
                            "\"protocol_cost\":{}",
                            json_string(&a.protocol_cost.to_fin_string())
                        ),
                    ],
                );
            } else if let Some(a) = act.as_any().downcast_ref::<DiaInscEdit>() {
                push_diamond_engrave_item(
                    &mut datalist,
                    &txhx,
                    tx_hash,
                    vec![
                        format!("\"action\":{}", json_string("edit")),
                        format!("\"diamonds\":{}", json_string(&a.diamond.to_readable())),
                        format!("\"index\":{}", a.index.uint() as u64),
                        format!("\"engraved_type\":{}", a.engraved_type.uint()),
                        format!(
                            "\"protocol_cost\":{}",
                            json_string(&a.protocol_cost.to_fin_string())
                        ),
                        format!(
                            "\"inscription\":{}",
                            json_string(&a.engraved_content.to_readable_or_hex())
                        ),
                    ],
                );
            }
        }
    };

    if txposi >= 0 {
        pick_engrave(trs[txposi as usize + 1].as_ref());
    } else {
        for tx in &trs[1..] {
            pick_engrave(tx.as_ref());
        }
    }

    ApiResponse::json(format!("{{\"ret\":0,\"list\":[{}]}}", datalist.join(",")))
}
