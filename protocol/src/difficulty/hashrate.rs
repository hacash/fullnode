
pub const HASHRATE_VALUE_BASE: [u8; HXS] = [0,0,0,0,0,0,0,0,
    255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255];

const VK: f64 = 1000.0; 
const VM: f64 = VK * VK;
const VG: f64 = VM * VK;
const VT: f64 = VG * VK;
const VP: f64 = VT * VK;
const VE: f64 = VP * VK;
const VZ: f64 = VE * VK;
const VY: f64 = VZ * VK;
const VB: f64 = VY * VK;

const HNS: [&str; 9] = ["K","M","G","T","P","E","Z","Y","B"];
const HVS: [f64;  9] = [VK, VM, VG, VT, VP, VE, VZ, VY, VB];


pub const LOWEST_DIFFICULTY: u32 = 4294967294;


/*
*
*/

pub fn rates_to_show(rates: f64) -> String {
    if rates < VK {
        return format!("{:.2}c/s", rates)
    }
    let mut hsx = HVS.len() - 1;
    for i in 0..HVS.len() {
        if rates / HVS[i] < VK {
            hsx = i;
            break
        }
    }
    let num = rates / HVS[hsx];
    let unit = HNS[hsx];
    format!("{:.2}{}c/s", num, unit)
}

pub fn hash_to_rateshow(hx: &[u8; HXS], secs: f64) -> String {
    let rates = hash_to_rates(hx, secs);
    rates_to_show(rates)
}

pub fn u32_to_rateshow(num: u32, secs: f64) -> String {
    let rates = u32_to_rates(num, secs);
    rates_to_show(rates)
}

pub fn u32_to_rates(num: u32, secs: f64) -> f64 {
    let hx = u32_to_hash(num);
    hash_to_rates(&hx, secs)
}

pub fn hash_to_rates(hx: &[u8; HXS], secs: f64) -> f64 {
    hash_to_power(&hx) / secs
}

pub fn hash_to_power_u128(hx: &[u8; HXS]) -> u128 {
    let power = hash_to_power(hx);
    power as u128
}

pub fn hash_to_power(hx: &[u8; HXS]) -> f64 {
    let bigv = BigInt::from_bytes_be(BigSign::Plus, &hx[..]);
    let base = BigInt::from_bytes_be(BigSign::Plus, &HASHRATE_VALUE_BASE[..]);
    let power = base.to_f64().unwrap() / bigv.to_f64().unwrap() * (u64::MAX as f64 + 1.0);
    power
}






