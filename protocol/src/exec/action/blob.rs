//! Message / Blob execute bodies.

use crate::codec::action::{Blob, Message};

base::impl_action_execute! {
    Message {
        (self, _ctx) {
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    Blob {
        (self, _ctx) {
            Ok(vec![])
        }
    }
}
