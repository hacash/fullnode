use base::{ApiExecCtx, PowBlockExt};
use sys::ToHex;

use super::request::json_string;

pub(crate) fn block_message_string(msg: Option<&field::Fixed16>) -> String {
    msg.map(|m| sys::left_readable_string(m.as_ref()))
        .unwrap_or_default()
}

pub(crate) fn block_summary_json(block: &dyn base::Block, hash: field::Hash, unit: &str) -> String {
    let prelude = block.prelude_transaction().ok();
    let miner = prelude
        .and_then(|tx| tx.author())
        .unwrap_or_else(|| prelude.map(|tx| tx.main()).unwrap_or_default());
    let reward = prelude
        .and_then(|tx| tx.block_reward())
        .cloned()
        .unwrap_or_default();
    let message = block_message_string(prelude.and_then(|tx| tx.block_message()));
    format!(
        concat!(
            "{{",
            "\"height\":{},",
            "\"hash\":{},",
            "\"msg\":{},",
            "\"reward\":{},",
            "\"miner\":{},",
            "\"time\":{},",
            "\"txs\":{}",
            "}}"
        ),
        block.height(),
        json_string(&hash.as_ref().to_hex()),
        json_string(&message),
        json_string(&reward.to_unit_string(unit)),
        json_string(&miner.to_readable()),
        block.timestamp(),
        block.transaction_count().saturating_sub(1),
    )
}

pub(crate) fn block_intro_json(pkg: &base::BlkPkg, unit: &str, tx_hash_list: bool) -> String {
    let block = pkg.block();
    let prelude = block.prelude_transaction().ok();
    let miner = prelude
        .and_then(|tx| tx.author())
        .unwrap_or_else(|| prelude.map(|tx| tx.main()).unwrap_or_default());
    let reward = prelude
        .and_then(|tx| tx.block_reward())
        .cloned()
        .unwrap_or_default();
    let message = block_message_string(prelude.and_then(|tx| tx.block_message()));
    let mut fields = vec![
        format!("\"hash\":{}", json_string(&pkg.hash().as_ref().to_hex())),
        format!("\"version\":{}", block.version()),
        format!("\"height\":{}", block.height()),
        format!("\"timestamp\":{}", block.timestamp()),
        format!(
            "\"mrklroot\":{}",
            json_string(&block.mrklroot().as_ref().to_hex())
        ),
        format!(
            "\"prevhash\":{}",
            json_string(&block.prev_hash().as_ref().to_hex())
        ),
        format!("\"nonce\":{}", block.pow_nonce()),
        format!("\"difficulty\":{}", block.pow_difficulty()),
        format!("\"miner\":{}", json_string(&miner.to_readable())),
        format!("\"reward\":{}", json_string(&reward.to_unit_string(unit))),
        format!("\"message\":{}", json_string(&message)),
        format!(
            "\"transaction\":{}",
            block.transaction_count().saturating_sub(1)
        ),
    ];
    if tx_hash_list {
        let hashes = block
            .transactions()
            .iter()
            .skip(1)
            .map(|tx| json_string(&tx.hash().as_ref().to_hex()))
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"tx_hash_list\":[{}]", hashes));
    }
    format!("{{\"ret\":0,{}}}", fields.join(","))
}

pub(crate) fn load_block_by_key(ctx: &ApiExecCtx, key: &str) -> sys::Ret<base::BlkPkg> {
    let store = ctx.engine.store();
    let data = if key.len() == field::Hash::SIZE * 2 {
        let hx = hex::decode(key).map_err(|_| sys::Error::fault("block hash format invalid"))?;
        if hx.len() != field::Hash::SIZE {
            return sys::errf!("block hash format invalid");
        }
        let mut raw = [0u8; field::Hash::SIZE];
        raw.copy_from_slice(&hx);
        store.block_data(&field::Hash::from(raw))?
    } else if let Ok(height) = key.parse::<u64>() {
        store.block_data_by_height(height)?.map(|(_, data)| data)
    } else {
        None
    };
    let Some(data) = data else {
        return sys::errf!("block not found");
    };
    base::BlkPkg::from_bytes(
        ctx.engine.services().as_ref(),
        data.to_vec(),
        base::PkgSource::new(base::PkgOrigin::Api),
    )
    .map_err(|e| sys::Error::fault(format!("block parse failed: {}", e)))
}

pub(crate) fn block_recent_json(li: &base::RecentBlock, unit: &str) -> String {
    format!(
        concat!(
            "{{",
            "\"height\":{},",
            "\"hash\":{},",
            "\"prev\":{},",
            "\"txs\":{},",
            "\"miner\":{},",
            "\"message\":{},",
            "\"reward\":{},",
            "\"time\":{},",
            "\"arrive\":{}",
            "}}"
        ),
        li.height,
        json_string(&li.hash.as_ref().to_hex()),
        json_string(&li.prev.as_ref().to_hex()),
        li.txs.saturating_sub(1),
        json_string(&li.miner.to_readable()),
        json_string(&li.message),
        json_string(&li.reward.to_unit_string(unit)),
        li.timestamp,
        li.arrive,
    )
}
