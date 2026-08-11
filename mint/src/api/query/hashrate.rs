use base::{ApiExecCtx, ApiRequest, ApiResponse, PowBlockExt};

use crate::api::util::*;

use crate::difficulty::DifficultyConfig;

pub(crate) fn hashrate_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    let latest = ctx.engine.latest_block();
    let prev100_timestamp = match latest.height().checked_sub(100) {
        Some(height) => match ctx.engine.block_history().block_at_height(height) {
            Ok(block) => block.map(|block| block.timestamp()),
            Err(e) => {
                return ApiResponse::err(503, &format!("block history read failed: {}", e));
            }
        },
        None => None,
    };
    ApiResponse::json(hashrate_json(
        latest.height(),
        latest.timestamp(),
        latest.pow_difficulty(),
        prev100_timestamp,
    ))
}

pub(crate) fn hashrate_logs_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let mut days = req.query_u64("days").unwrap_or(200);
    if days == 0 {
        days = 1;
    }
    let target = q_bool(&req, "target", false);
    let scale = q_f64(&req, "scale", 0.0);
    let blocks_per_adjust = DifficultyConfig::default().difficulty_adjust_blocks;

    if days > 500 {
        return api_error("param days cannot exceed 500");
    }
    let latest = ctx.engine.latest_block();
    let last_height = latest.height();
    if last_height < days {
        return api_error("param days value overflow");
    }
    let secs = last_height / days;

    let mut day200 = Vec::with_capacity(days as usize);
    let mut dayall = Vec::with_capacity(days as usize);
    let mut day200_max = 0u128;
    let mut dayall_max = 0u128;
    for i in 0..days {
        let distance = (days - 1 - i).saturating_mul(blocks_per_adjust);
        let s1 = last_height.saturating_sub(distance);
        let s2 = secs + secs * i;
        let rt1 = match block_rate_at(ctx, s1) {
            Ok(rate) => rate,
            Err(e) => {
                return ApiResponse::err(503, &format!("block history read failed: {}", e));
            }
        };
        let rt2 = match block_rate_at(ctx, s2) {
            Ok(rate) => rate,
            Err(e) => {
                return ApiResponse::err(503, &format!("block history read failed: {}", e));
            }
        };
        day200_max = day200_max.max(rt1);
        dayall_max = dayall_max.max(rt2);
        day200.push(rt1);
        dayall.push(rt2);
    }

    scale_u128_series(&mut day200, day200_max, scale);
    scale_u128_series(&mut dayall, dayall_max, scale);

    let mut fields = Vec::new();
    if target {
        let prev100_timestamp = match latest.height().checked_sub(100) {
            Some(height) => match ctx.engine.block_history().block_at_height(height) {
                Ok(block) => block.map(|block| block.timestamp()),
                Err(e) => {
                    return ApiResponse::err(503, &format!("block history read failed: {}", e));
                }
            },
            None => None,
        };
        fields.push(hashrate_fields_json(
            latest.height(),
            latest.timestamp(),
            latest.pow_difficulty(),
            prev100_timestamp,
        ));
    }
    fields.push(format!("\"day200\":{}", u128_array_json(&day200)));
    fields.push(format!("\"dayall\":{}", u128_array_json(&dayall)));
    ApiResponse::json(format!("{{\"ret\":0,{}}}", fields.join(",")))
}
