//! TxMessage / TxBlob actions.

use field::{BytesW1, BytesW2};

base::action_simple! { TxMessage, 0x0401, 2, GUARD, {
    data: BytesW1
}, this, {
    blob,
    description: "Transaction message".to_owned()
}}

base::action_simple! { TxBlob, 0x0402, 2, GUARD, {
    data: BytesW2
}, this, {
    blob,
    description: "Transaction blob data".to_owned()
}}
