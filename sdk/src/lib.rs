#![cfg_attr(all(target_arch = "wasm32", not(test)), no_main)]

mod account;
mod codec;
mod coin;
mod error;
mod sign;
mod util;

pub use account::{Account, VerifyAddressResult, create_account, verify_address};
pub use coin::{CoinTransferParam, CoinTransferResult, create_coin_transfer};
pub use sign::{SignTxParam, SignTxResult, sign_transaction};
pub use util::{hac_to_mei, hac_to_unit};
