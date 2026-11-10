#[no_mangle]
pub extern "C" fn calc_entry(a0: *const u8, a1: u64, a2: *mut u8, a3: u64) -> u64 {
    let input = unsafe { core::slice::from_raw_parts(a0, a1 as usize) };
    let out = unsafe { core::slice::from_raw_parts_mut(a2, a3 as usize) };
    let n = kernel(input, out);
    if n > out.len() {
        trap();
    }
    n as u64
}

fn trap() -> ! {
    loop {
        unsafe {
            core::arch::asm!(
                ".option push",
                ".option norvc",
                "ebreak",
                ".option pop",
            );
        }
    }
}

#[panic_handler]
fn calc_panic(_: &core::panic::PanicInfo) -> ! {
    trap()
}
