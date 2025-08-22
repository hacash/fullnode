// #![no_std]
#![no_main]

// use wasm_bindgen::prelude::wasm_bindgen;

// #[panic_handler]
// fn handle_panic(_: &core::panic::PanicInfo) -> ! {
//     loop {}
// }

#[allow(unused_macros)]
macro_rules! panic {
    ($s:expr) => {
        loop {}
    };
    ($fmt:expr, $($s:expr),+) => {
        loop {}
    };
}


use wasm_bindgen::prelude::*;
use sys::Account as SysAccount;
use sys::*;
use field::*;
use field::interface::*;


include!{"param.rs"}
include!{"util.rs"}
include!{"account.rs"}
include!{"coin.rs"}
include!{"sign.rs"}

