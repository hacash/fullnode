#![no_std]
#![no_main]

// Calc kernel fixture: loop + table lookup + hash-style mixing. One entry
// point, `calc_entry`, following the
// container ABI (a0=input, a1=len, a2=output, a3=out capacity).
//
// Two builds:
//  - default (container): text+rodata become the container code image
//  - `oracle` feature: adds `_start` with Linux write/exit syscalls so the
//    same logic runs under qemu-riscv64 -user as a differential oracle.

#[cfg(feature = "oracle")]
use core::arch::global_asm;

#[no_mangle]
pub extern "C" fn calc_entry(input_ptr: *const u8, input_len: u64, out_ptr: *mut u8, _out_cap: u64) -> u64 {
    let input = unsafe { core::slice::from_raw_parts(input_ptr, input_len as usize) };
    let mut out = unsafe { core::slice::from_raw_parts_mut(out_ptr, 32) };
    mix256(input, &mut out);
    #[cfg(not(feature = "oracle"))]
    { 32 }
    #[cfg(feature = "oracle")]
    { 32 }
}

#[cfg(feature = "oracle")]
#[used]
#[no_mangle]
pub static mut KINPUT: [u8; 64] = [
    0x61, 0x63, 0x64, 0x63, 0x61, 0x63, 0x21, 0x68, 0x61, 0x63, 0x61, 0x73, 0x68, 0x21, 0x72,
    0x76, 0x36, 0x34, 0x69, 0x6d, 0x21, 0x6b, 0x65, 0x72, 0x6e, 0x65, 0x6c, 0x21, 0x74, 0x65,
    0x73, 0x74, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
    0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
    0x0d, 0x0e, 0x0f, 0x10,
];

#[used]
#[no_mangle]
pub static mut KSTACK: [u64; 1024] = [0u64; 1024];

#[cfg(feature = "oracle")]
core::arch::global_asm!(
    ".section .oracle_start,\"ax\",@progbits",
    ".globl _start",
    "_start:",
    "   la   sp, KSTACK+8192",
    "   li   s0, 200",
    "1: la   a0, KINPUT",
    "   li   a1, 64",
    "   la   a2, KSTACK", // scratch output inside .bss for the oracle run
    "   li   a3, 32",
    "   call calc_entry",
    "   addi s0, s0, -1",
    "   bnez s0, 1b",
    // write(1, out, 32)
    "   li   a7, 64",
    "   li   a0, 1",
    "   la   a1, KSTACK",
    "   li   a2, 32",
    "   ecall",
    // exit(0)
    "   li   a7, 93",
    "   li   a0, 0",
    "   ecall",
    "   j    .",
);

const TBL: [u64; 64] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4b3cfa, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
];

#[inline(never)]
fn mix256(input: &[u8], out: &mut [u8]) {
    let mut h: u64 = 0x9e3779b97f4a7c15;
    let mut i: usize = 0;
    while i < input.len() {
        let b = input[i];
        let t = TBL[(i & 63) as usize];
        h ^= (b as u64).wrapping_mul(t);
        h = h.rotate_left(13);
        h = h.wrapping_mul(5).wrapping_add(0x9e3779b97f4a7c15);
        if i % 16 == 15 {
            h ^= h / 0x9e3779b1;
        }
        i += 1;
    }
    let mut j = 0;
    while j < 8 {
        out[j] = (h >> (j * 8)) as u8;
        j += 1;
    }
    let _ = out;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::arch::asm!("ebreak", options(noreturn)) }
}
