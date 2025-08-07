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
use sys::*;
use field::*;



#[wasm_bindgen]
pub fn hac_to_unit(stuff: &str, unit: u8) -> Ret<f64> {
    Amount::from(stuff).map(|a|unsafe{a.to_unit_float(unit)})
}

#[wasm_bindgen]
pub fn hac_to_mei(stuff: &str) -> Ret<f64> {
    hac_to_unit(stuff, UNIT_MEI)
}

