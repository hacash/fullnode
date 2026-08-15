use base::{ApiExecCtx, ApiRequest, ApiResponse};

use crate::api::util::*;

use field::{Amount, ChannelId};

use crate::state::MintStateRead;

pub(crate) fn channel_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let id = q_string(&req, "id", "");
    let raw = match hex::decode(&id) {
        Ok(v) => v,
        Err(_) => return api_error("channel id format invalid"),
    };
    if raw.len() != ChannelId::SIZE {
        return api_error("channel id format invalid");
    }
    let mut id_raw = [0u8; ChannelId::SIZE];
    id_raw.copy_from_slice(&raw);
    let chid = ChannelId::from(id_raw);

    let snapshot = match optimistic_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let start_epoch = snapshot.epoch;
    let state = MintStateRead::wrap(snapshot.view());
    let channel = match state.channel(&chid) {
        Ok(Some(channel)) => channel,
        Ok(None) => {
            if !ctx.engine.validate_optimistic(start_epoch) {
                return api_error("state changed");
            }
            return api_error("channel not found");
        }
        Err(e) => return api_state_read_error(&e),
    };
    let result = channel_json(&chid, &channel, &unit);
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed");
    }
    match result {
        Ok(body) => ApiResponse::json(body),
        Err(_) => api_error("channel interest calculation failed"),
    }
}

pub(crate) fn fee_average_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let consumption = req.query_u64("consumption").unwrap_or(0);
    let extra9 = q_bool(&req, "extra9", q_bool(&req, "burn90", false));
    let txty = req.query_u64("tx_type").unwrap_or(2) as u8;
    let avgfeep = ctx.engine.average_fee_purity();
    let mut fields = vec![format!("\"purity\":{}", avgfeep)];

    if consumption > 0 {
        let Some(fee238) = (avgfeep as u128).checked_mul(consumption as u128) else {
            return api_error("fee estimate overflow");
        };
        if fee238 > u64::MAX as u128 {
            return api_error("fee estimate overflow");
        }
        let mut base = Amount::unit238(fee238 as u64);
        if base.is_zero() {
            base = Amount::zhu(1);
        }
        let mut setfee = base.clone();
        if extra9 {
            if txty < protocol::tx_std::TransactionType3::TYPE {
                if let Ok(f) = base.dist_mul(10) {
                    setfee = f;
                }
            } else if let Ok(f) = base.dist_mul(9) {
                setfee = f;
            }
        }
        fields.push(format!(
            "\"feasible\":{}",
            json_string(&setfee.to_unit_string(&unit))
        ));
    }
    ApiResponse::json(format!("{{\"ret\":0,{}}}", fields.join(",")))
}
