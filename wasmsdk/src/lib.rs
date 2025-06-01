#![no_std]
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



#[no_mangle]
pub fn add(left: u64, right: u64) -> u64 {
    let _ = left + right;
    panic!("{}", 2)
}


#[no_mangle]
pub fn create_by(stuff: i32) -> u32 {
    let sss = &[stuff][..];//.as_bytes();
    // let mut res = field::Fold64::from(sss[0] as u64).unwrap();
    let mut res = sss[0] as u64 + field::Fold64::MAX;
    res += 1;
    res as u32
}




