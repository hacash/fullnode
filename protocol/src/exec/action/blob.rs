//! TxMessage / TxBlob execute bodies.

use crate::codec::action::{TxBlob, TxMessage};

base::impl_action_execute! {
    TxMessage {
        (self, _ctx) {
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    TxBlob {
        (self, _ctx) {
            Ok(vec![])
        }
    }
}
