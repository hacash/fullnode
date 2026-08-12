use std::collections::BTreeMap;

use base::{ApiExecCtx, ApiRequest, ApiResponse};

use crate::api::util::*;

use sys::{ToBase64, ToHex};

pub(crate) fn block_intro_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let mut key = q_string(&req, "hash", "");
    let height = req.query_u64("height").unwrap_or(0);
    let tx_hash_list = q_bool(&req, "tx_hash_list", false);
    if height > 0 {
        key = height.to_string();
    }
    match load_block_by_key(ctx, &key) {
        Ok(pkg) => ApiResponse::json(block_intro_json(&pkg, &unit, tx_hash_list)),
        Err(e) => api_error(&format!("block query failed: {}", e)),
    }
}

pub(crate) fn block_datas_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let store = ctx.engine.store();
    let unstable_block = ctx.engine.config().unstable_block;
    let mut last_height = ctx.engine.latest_height();

    const MB: usize = 1024 * 1024;
    let hexbody = q_bool(&req, "hexbody", false);
    let base64body = q_bool(&req, "base64body", false);
    let start_height = req.query_u64("start_height").unwrap_or(0);
    let limit = req.query_u64("limit").unwrap_or(u64::MAX);
    let mut max_size = req.query_usize("max_size").unwrap_or(MB);
    let confirm = q_bool(&req, "confirm", false);
    if max_size > 10 * MB {
        max_size = 10 * MB;
    }
    if confirm && last_height > unstable_block {
        last_height -= unstable_block;
    }

    let mut alldatas = Vec::with_capacity(max_size);
    let mut count = 0u64;
    for height in start_height..u64::MAX {
        if height > last_height || count >= limit || alldatas.len() >= max_size {
            break;
        }
        let found = match store.block_data_by_height(height) {
            Ok(found) => found,
            Err(e) => {
                return api_error(&format!("block read failed: {}", e));
            }
        };
        let Some((_, block_data)) = found else {
            break;
        };
        alldatas.extend_from_slice(block_data.as_ref());
        count += 1;
    }

    let content_type = if hexbody || base64body {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    if hexbody {
        return api_bytes(alldatas.to_hex().into_bytes(), content_type);
    }
    if base64body {
        return api_bytes(alldatas.to_base64().into_bytes(), content_type);
    }
    api_bytes(alldatas, content_type)
}

pub(crate) fn block_views_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let mut limit = q_i64(&req, "limit", 20);
    let page = q_i64(&req, "page", 1);
    let start = q_i64(&req, "start", i64::MAX);
    let desc = q_bool(&req, "desc", false);
    if limit > 200 {
        limit = 200;
    }
    let last_height = ctx.engine.latest_height() as i64;
    let mut list = Vec::new();
    for id in get_id_range(last_height, page, limit, start, desc) {
        if id < 0 {
            continue;
        }
        let block = match ctx.engine.block_history().block_at_height(id as u64) {
            Ok(Some(block)) => block,
            Ok(None) => continue,
            Err(e) => {
                return api_error(&format!("block history read failed: {}", e));
            }
        };
        list.push(block_summary_json(block.as_ref(), block.hash(), &unit));
    }
    api_data_list_field("latest_height", last_height, list)
}

const BLOCKS_PER_DAY: u64 = 288;
const POOL_PERIODS: [(&str, u64); 3] = [
    ("1d", BLOCKS_PER_DAY),
    ("7d", BLOCKS_PER_DAY * 7),
    ("30d", BLOCKS_PER_DAY * 30),
];
const POOL_RANK_LIMIT: usize = 7;

pub(crate) fn block_pool_stats_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    let last_height = ctx.engine.latest_height();
    let longest_period = POOL_PERIODS
        .iter()
        .map(|(_, block_count)| *block_count)
        .max()
        .unwrap_or_default();
    let first_height = last_height
        .saturating_sub(longest_period.saturating_sub(1))
        .max(1);
    let mut totals = vec![0u64; POOL_PERIODS.len()];
    let mut pool_counts = (0..POOL_PERIODS.len())
        .map(|_| BTreeMap::<(String, String), u64>::new())
        .collect::<Vec<_>>();

    // All periods end at the same height, so one longest-window scan populates every period.
    for height in first_height..=last_height {
        let block = match ctx.engine.block_history().block_at_height(height) {
            Ok(Some(block)) => block,
            Ok(None) => continue,
            Err(e) => {
                return api_error(&format!("block history read failed: {}", e));
            }
        };
        let prelude = block.prelude_transaction().ok();
        let miner = prelude
            .and_then(|tx| tx.author())
            .unwrap_or_else(|| prelude.map(|tx| tx.main()).unwrap_or_default())
            .to_readable();
        let message = block_message_string(prelude.and_then(|tx| tx.block_message()));
        let age = last_height.saturating_sub(height);

        for (index, (_, block_count)) in POOL_PERIODS.iter().enumerate() {
            if age >= *block_count {
                continue;
            }
            *pool_counts[index]
                .entry((message.clone(), miner.clone()))
                .or_insert(0) += 1;
            totals[index] += 1;
        }
    }

    let mut periods = Vec::with_capacity(POOL_PERIODS.len());
    for (index, (key, block_count)) in POOL_PERIODS.iter().enumerate() {
        let total = totals[index];
        let mut ranked = std::mem::take(&mut pool_counts[index])
            .into_iter()
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let list = ranked
            .into_iter()
            .take(POOL_RANK_LIMIT)
            .map(|((name, miner), count)| {
                format!(
                    "{{\"name\":{},\"miner\":{},\"count\":{}}}",
                    json_string(&name),
                    json_string(&miner),
                    count
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        periods.push(format!(
            "{{\"key\":{},\"blocks\":{},\"total\":{},\"list\":[{}]}}",
            json_string(key),
            block_count,
            total,
            list
        ));
    }

    ApiResponse::json(format!(
        "{{\"ret\":0,\"latest_height\":{},\"periods\":[{}]}}",
        last_height,
        periods.join(",")
    ))
}

pub(crate) fn block_recents_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let unit = q_string(&req, "unit", "fin");
    let list = ctx
        .engine
        .recent_blocks()
        .iter()
        .map(|li| block_recent_json(li, &unit))
        .collect::<Vec<_>>();
    ApiResponse::json(format!("{{\"ret\":0,\"list\":[{}]}}", list.join(",")))
}
