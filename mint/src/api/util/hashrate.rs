use base::{ApiExecCtx, PowBlockExt};
use sys::ToHex;

use crate::difficulty::{DifficultyConfig, hash_to_rates, rates_to_show, u32_to_hash};

use super::request::json_string;

pub(crate) fn right_00_to_ff(hash: &mut [u8]) {
    if hash.is_empty() || *hash.last().unwrap() != 0 {
        return;
    }
    for i in (0..hash.len()).rev() {
        if hash[i] > 0 {
            hash[i] -= 1;
            break;
        }
        hash[i] = 255;
    }
}

pub(crate) fn hashrate_fields_json(
    height: u64,
    timestamp: u64,
    difficulty: u32,
    prev100_timestamp: Option<u64>,
) -> String {
    let btt = DifficultyConfig::default().each_block_target_time as f64;
    let mut target_hash = u32_to_hash(difficulty);
    let target_rate = hash_to_rates(&target_hash, btt);
    let target_show = rates_to_show(target_rate);
    let mut realtime_rate = target_rate;
    let mut realtime_show = target_show.clone();
    if height > 100 {
        if let Some(prev_time) = prev100_timestamp {
            let cttt = timestamp.saturating_sub(prev_time) / 100;
            if cttt > 0 {
                realtime_rate = realtime_rate * btt / cttt as f64;
                realtime_show = rates_to_show(realtime_rate);
            }
        }
    }
    right_00_to_ff(&mut target_hash);
    format!(
        concat!(
            "\"target\":{{",
            "\"rate\":{},",
            "\"show\":{},",
            "\"unit\":\"H/s\",",
            "\"hash\":{},",
            "\"difn\":{}",
            "}},",
            "\"realtime\":{{",
            "\"rate\":{},",
            "\"show\":{},",
            "\"unit\":\"H/s\"",
            "}}",
        ),
        target_rate,
        json_string(&target_show),
        json_string(&target_hash.to_hex()),
        difficulty,
        realtime_rate,
        json_string(&realtime_show),
    )
}

pub(crate) fn hashrate_json(
    height: u64,
    timestamp: u64,
    difficulty: u32,
    prev100_timestamp: Option<u64>,
) -> String {
    format!(
        "{{\"ret\":0,{}}}",
        hashrate_fields_json(height, timestamp, difficulty, prev100_timestamp)
    )
}

pub(crate) fn u128_array_json(values: &[u128]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(crate) fn block_rate_at(ctx: &ApiExecCtx, height: u64) -> sys::Ret<u128> {
    let Some(block) = ctx.engine.block_history().block_at_height(height)? else {
        return Ok(0);
    };
    let secs = DifficultyConfig::default().each_block_target_time as f64;
    Ok(crate::difficulty::u32_to_rates(block.pow_difficulty(), secs) as u128)
}

pub(crate) fn scale_u128_series(values: &mut [u128], max: u128, scale: f64) {
    if scale <= 0.0 || max == 0 {
        return;
    }
    let divisor = max as f64 / scale;
    for value in values.iter_mut() {
        *value = (*value as f64 / divisor) as u128;
    }
}
