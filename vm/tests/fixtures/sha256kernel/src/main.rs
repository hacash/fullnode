#![no_std]
#![no_main]

// sha256 calc kernel: the full compression function in pure RV64IM
// instructions (no ecall) — the "custom hash" workload shape that the HVM
// bytecode channel cannot afford under resource metering.
//
// Container ABI: input = message bytes (padded here), output = 32-byte
// digest, a0 = 32 on return.

#[cfg(feature = "oracle")]
use core::arch::global_asm;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[inline(always)]
fn wr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

/// HALT ecall: a7=0, a0=exit, a1=output length. Spin afterwards so LLVM
/// does not emit `unimp` (which the verifier rejects) after a noreturn asm.
#[inline(never)]
fn halt(exit: u64, out_len: u64) -> ! {
    loop {
        unsafe {
            core::arch::asm!(
                "ecall",
                in("a0") exit,
                in("a1") out_len,
                in("a7") 0u64,
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn calc_entry(input_ptr: *const u8, input_len: u64, out_ptr: *mut u8, _out_cap: u64) -> u64 {
    let msg = unsafe { core::slice::from_raw_parts(input_ptr, input_len as usize) };
    let out = unsafe { core::slice::from_raw_parts_mut(out_ptr, 32) };
    // Two modes: a `{k=v,...}` flat-kv document runs the dict-walk digest
    // (mirroring the HVM `ascii_parse_flat_kv` native on this grammar
    // subset); anything else is hashed directly.
    let digest = if msg.first() == Some(&b'{') {
        match dict_walk_digest(msg) {
            Some(d) => d,
            None => {
                #[cfg(not(feature = "oracle"))]
                halt(0, 0);
                #[cfg(feature = "oracle")]
                return 0;
            }
        }
    } else {
        sha256(msg)
    };
    let mut i = 0;
    while i < 8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&digest[i].to_be_bytes());
        i += 1;
    }
    #[cfg(not(feature = "oracle"))]
    halt(0, 32);
    #[cfg(feature = "oracle")]
    { 32 }
}

const MAX_PAIRS: usize = 64;
const MAX_ACC: usize = 512;

/// Parse `{key=value,...}` (keys a-z, values alphanumeric, no whitespace —
/// the flags=0 grammar of `ascii_parse_flat_kv` with LOWER/ALNUM classes),
/// then walk: start at key `start`, each visited value names the next key,
/// stop after `end` (its value is appended too); digest the values.
/// Returns None on any grammar or walk error (the caller reports it by
/// returning an empty output).
#[inline(never)]
fn dict_walk_digest(doc: &[u8]) -> Option<[u32; 8]> {
    let lower = |c: u8| c.is_ascii_lowercase();
    let alnum = |c: u8| c.is_ascii_alphanumeric();
    // Parse/walk tables live in static storage (.bss). The committed image's
    // `.bss` is 0x400 bytes (`bss_len.txt`); packers fold that zero span
    // into the container `init` image, which stays writable like every RW
    // segment. Container memory is zeroed before every run.
    static mut KS: [u16; MAX_PAIRS] = [0; MAX_PAIRS];
    static mut KL: [u16; MAX_PAIRS] = [0; MAX_PAIRS];
    static mut VS: [u16; MAX_PAIRS] = [0; MAX_PAIRS];
    static mut VL: [u16; MAX_PAIRS] = [0; MAX_PAIRS];
    static mut ACC: [u8; MAX_ACC] = [0; MAX_ACC];
    let (mut ks, mut kl, mut vs, mut vl, mut acc) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(KS),
            &mut *core::ptr::addr_of_mut!(KL),
            &mut *core::ptr::addr_of_mut!(VS),
            &mut *core::ptr::addr_of_mut!(VL),
            &mut *core::ptr::addr_of_mut!(ACC),
        )
    };
    let mut n = 0usize;
    let mut i = 0usize;
    if doc.first() != Some(&b'{') {
        return None;
    }
    i = 1;
    loop {
        let k0 = i;
        while i < doc.len() && lower(doc[i]) {
            i += 1;
        }
        if i == k0 {
            return None;
        }
        if i >= doc.len() || doc[i] != b'=' {
            return None;
        }
        i += 1;
        let v0 = i;
        while i < doc.len() && alnum(doc[i]) {
            i += 1;
        }
        if i == v0 {
            return None;
        }
        // duplicate keys are a parse error, same as the native
        for p in 0..n {
            let (a0, a1) = (ks[p] as usize, ks[p] as usize + kl[p] as usize);
            if &doc[a0..a1] == &doc[k0..v0 - 1] {
                return None;
            }
        }
        if n == MAX_PAIRS {
            return None;
        }
        ks[n] = k0 as u16;
        kl[n] = (v0 - 1 - k0) as u16;
        vs[n] = v0 as u16;
        vl[n] = (i - v0) as u16;
        n += 1;
        if i >= doc.len() {
            return None;
        }
        if doc[i] == b',' {
            i += 1;
            continue;
        }
        if doc[i] == b'}' {
            i += 1;
            break;
        }
        return None;
    }
    if i != doc.len() || n == 0 {
        return None;
    }
    // walk
    let mut an = 0usize;
    let mut key: &[u8] = b"start";
    let mut steps = 0usize;
    loop {
        if steps >= 64 {
            return None;
        }
        let mut found = None;
        for p in 0..n {
            let (a0, a1) = (ks[p] as usize, ks[p] as usize + kl[p] as usize);
            if &doc[a0..a1] == key {
                found = Some(p);
                break;
            }
        }
        let p = found?;
        let (v0i, v1i) = (vs[p] as usize, vs[p] as usize + vl[p] as usize);
        let val = &doc[v0i..v1i];
        if an + val.len() > MAX_ACC {
            return None;
        }
        acc[an..an + val.len()].copy_from_slice(val);
        an += val.len();
        if val == b"end" {
            break;
        }
        key = val;
        steps += 1;
    }
    Some(sha256(&acc[..an]))
}

#[inline(never)]
fn sha256(msg: &[u8]) -> [u32; 8] {
    // Streaming compression: 64-byte blocks built on demand (the whole
    // kernel keeps its stack frame well inside the 2 KiB calc stack).
    let mut h = H0;
    let nblocks = (msg.len() + 9).div_ceil(64);
    let bits = (msg.len() as u64) * 8;
    let mut block = [0u8; 64];
    let mut w = [0u32; 64];
    let mut b = 0usize;
    while b < nblocks {
        let base = b * 64;
        let avail = (msg.len() - base).min(64);
        block = [0u8; 64];
        block[..avail].copy_from_slice(&msg[base..base + avail]);
        if avail < 64 && base + avail == msg.len() {
            block[avail] = 0x80;
        }
        if b == nblocks - 1 {
            block[56..64].copy_from_slice(&bits.to_be_bytes());
        }
        let mut i = 0;
        while i < 16 {
            let o = i * 4;
            w[i] = u32::from_be_bytes([block[o], block[o + 1], block[o + 2], block[o + 3]]);
            i += 1;
        }
        let mut i = 16;
        while i < 64 {
            let s0 = wr(w[i - 15], 7) ^ wr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            let s1 = wr(w[i - 2], 17) ^ wr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
            i += 1;
        }
        let (mut a, mut bb, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        let mut i = 0;
        while i < 64 {
            let s1 = wr(e, 6) ^ wr(e, 11) ^ wr(e, 25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = wr(a, 2) ^ wr(a, 13) ^ wr(a, 22);
            let maj = (a & bb) ^ (a & c) ^ (bb & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = bb;
            bb = a;
            a = t1.wrapping_add(t2);
            i += 1;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(bb);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
        b += 1;
    }
    h
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

// Oracle-build stack only (the container runtime provides sp at the top
// of declared memory). Keeping it out of the container build leaves .bss
// to the dict tables below.
#[cfg(feature = "oracle")]
#[used]
#[no_mangle]
pub static mut KSTACK: [u64; 1024] = [0u64; 1024];

#[cfg(feature = "oracle")]
core::arch::global_asm!(
    ".section .oracle_start,\"ax\",@progbits",
    ".globl _start",
    "_start:",
    "   la   sp, KSTACK+8192",
    "   la   a0, KINPUT",
    "   li   a1, 64",
    "   la   a2, KSTACK",   // scratch output inside .bss for the oracle run
    "   li   a3, 32",
    "   call calc_entry",
    "   li   a7, 64",
    "   li   a0, 1",
    "   la   a1, KSTACK",
    "   li   a2, 32",
    "   ecall",
    "   li   a7, 93",
    "   li   a0, 0",
    "   ecall",
    "   j    .",
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::arch::asm!("ebreak", options(noreturn)) }
}
