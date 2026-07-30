use crate::minter::MinerPendingWork;

use super::request::{encode_miner_bytes, json_string};

pub(crate) fn miner_pending_json(
    work: MinerPendingWork,
    detail: bool,
    transaction: bool,
    stuff: bool,
    base64: bool,
) -> String {
    let mut fields = vec![
        format!("\"height\":{}", work.height),
        format!(
            "\"coinbase_nonce\":{}",
            json_string(&encode_miner_bytes(work.coinbase_nonce.as_ref(), base64))
        ),
        format!(
            "\"block_intro\":{}",
            json_string(&encode_miner_bytes(&work.block_intro, base64))
        ),
        format!(
            "\"target_hash\":{}",
            json_string(&encode_miner_bytes(work.target_hash.as_ref(), base64))
        ),
    ];
    if detail {
        fields.push(format!("\"version\":{}", work.version));
        fields.push(format!(
            "\"prevhash\":{}",
            json_string(&encode_miner_bytes(work.prevhash.as_ref(), base64))
        ));
        fields.push(format!("\"timestamp\":{}", work.timestamp));
        fields.push(format!("\"transaction_count\":{}", work.transaction_count));
        fields.push(format!(
            "\"reward_address\":{}",
            json_string(&work.reward_address.to_readable())
        ));
    }
    if transaction {
        let txs = work
            .transaction_body_list
            .iter()
            .map(|tx| json_string(&encode_miner_bytes(tx, base64)))
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"transaction_body_list\":[{}]", txs));
    }
    if stuff {
        fields.push(format!(
            "\"coinbase_body\":{}",
            json_string(&encode_miner_bytes(&work.coinbase_body, base64))
        ));
        let roots = work
            .mkrl_modify_list
            .iter()
            .map(|hx| json_string(&encode_miner_bytes(hx.as_ref(), base64)))
            .collect::<Vec<_>>()
            .join(",");
        fields.push(format!("\"mkrl_modify_list\":[{}]", roots));
    }
    format!("{{\"ret\":0,{}}}", fields.join(","))
}
