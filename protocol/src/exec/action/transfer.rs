//! Hac / Sat / Asset / Diamond transfer execute bodies.

use crate::codec::action::{
    TransferAssetFrom, TransferAssetFromTo, TransferAssetTo, TransferHacFrom, TransferHacFromTo,
    TransferHacTo, TransferHacdFrom, TransferHacdFromTo, TransferHacdSingleTo, TransferHacdTo,
    TransferSatFrom, TransferSatFromTo, TransferSatTo,
};
use crate::exec::apply::apply_transfer_intent;
use base::Action;

macro_rules! impl_transfer_execute {
    ($($ty:ty),+ $(,)?) => {$(
        base::impl_action_execute! {
            $ty {
                (self, ctx) {
                    let Some(intent) = Action::transfer_intent(self) else {
                        return sys::errf!(
                            "action kind {} missing transfer intent",
                            Self::KIND
                        );
                    };
                    apply_transfer_intent(ctx, intent)
                }
            }
        }
    )+};
}

impl_transfer_execute! {
    TransferHacTo,
    TransferHacFrom,
    TransferHacFromTo,
    TransferSatTo,
    TransferSatFrom,
    TransferSatFromTo,
    TransferAssetTo,
    TransferAssetFrom,
    TransferAssetFromTo,
    TransferHacdSingleTo,
    TransferHacdFromTo,
    TransferHacdTo,
    TransferHacdFrom,
}
