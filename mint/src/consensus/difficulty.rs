use base::PowBlockExt;
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use sys::{Ret, errf};

pub const LOWEST_DIFFICULTY: u32 = 0xffff_ffff;

/// Mainnet ASERT activation height (inclusive).
///
pub const ASERT_UPGRADE_HEIGHT: u64 = 738_654;
pub const ASERT_START_TARGET_NUM: u32 = 0xe9cf_ffff;

const ASERT_HALF_LIFE: i64 = 10_800;
const ASERT_RADIX_BITS: u32 = 16;
const ASERT_RADIX: i64 = 1i64 << ASERT_RADIX_BITS;
const ASERT_POLY_1: u128 = 195_766_423_245_049;
const ASERT_POLY_2: u128 = 971_821_376;
const ASERT_POLY_3: u128 = 5127;
const ASERT_POLY_TERM_SHIFT: usize = 48;
const ASERT_EASING_MAX_SCALE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DifficultyConfig {
    pub chain_id: base::ChainId,
    pub difficulty_adjust_blocks: u64,
    pub difficulty_group_blocks: u64,
    pub each_block_target_time: u64,
}

impl Default for DifficultyConfig {
    fn default() -> Self {
        Self::from_mint_params(base::ChainId::MAINNET, crate::MINT_PARAMS)
    }
}

impl DifficultyConfig {
    pub const fn from_mint_params(chain_id: base::ChainId, params: base::MintParams) -> Self {
        Self {
            chain_id,
            difficulty_adjust_blocks: params.difficulty_adjust_blocks,
            difficulty_group_blocks: params.difficulty_group_blocks,
            each_block_target_time: params.each_block_target_time,
        }
    }

    pub fn is_mainnet(&self) -> bool {
        self.chain_id.is_mainnet()
    }
}

pub struct DifficultyGnr {
    cnf: DifficultyConfig,
}

impl Default for DifficultyGnr {
    fn default() -> Self {
        Self::new(DifficultyConfig::default())
    }
}

impl DifficultyGnr {
    pub fn new(cnf: DifficultyConfig) -> Self {
        assert!(cnf.difficulty_adjust_blocks > 0);
        assert!(cnf.difficulty_group_blocks > 0);
        assert_eq!(
            cnf.difficulty_adjust_blocks % cnf.difficulty_group_blocks,
            0
        );
        Self { cnf }
    }

    /// Compute retarget for `hei`.
    ///
    /// - **Mainnet**: only ASERT (`hei >= ASERT_UPGRADE_HEIGHT`). Pre-ASERT callers
    ///   must skip retarget checks (see module comment / `is_pre_asert_mainnet`).
    /// - **Non-mainnet**: bootstrap + weighted sliding until the chain's early ASERT.
    pub fn target(
        &self,
        prevdiff: u32,
        prevblkt: u64,
        hei: u64,
        blkt: u64,
        history: &dyn base::BlockHistory,
    ) -> Ret<DifficultyTarget> {
        if self.is_asert_height(hei) {
            return self.target_asert(prevdiff, hei, blkt, history);
        }
        if self.cnf.is_mainnet() {
            return Ok(DifficultyTarget::from_num(prevdiff));
        }
        if self.use_bootstrap_rule(hei) {
            return Ok(self.target_bootstrap());
        }
        self.target_weighted_sliding(prevdiff, prevblkt, hei, history)
    }

    fn asert_upgrade_height(&self) -> u64 {
        if self.cnf.is_mainnet() {
            ASERT_UPGRADE_HEIGHT
        } else {
            // Test / side chains: enter ASERT after one adjust window.
            self.window_blocks() + 2
        }
    }

    pub(crate) fn is_asert_height(&self, hei: u64) -> bool {
        hei >= self.asert_upgrade_height()
    }

    pub(crate) fn is_pre_asert_mainnet(&self, hei: u64) -> bool {
        self.cnf.is_mainnet() && !self.is_asert_height(hei)
    }

    pub(crate) fn adjust_blocks(&self) -> u64 {
        self.cnf.difficulty_adjust_blocks
    }

    fn window_blocks(&self) -> u64 {
        self.cnf.difficulty_adjust_blocks
    }

    fn group_blocks(&self) -> u64 {
        self.cnf.difficulty_group_blocks
    }

    fn window_groups(&self) -> u64 {
        self.window_blocks() / self.group_blocks()
    }

    fn use_bootstrap_rule(&self, hei: u64) -> bool {
        hei <= self.window_blocks() + 1
    }

    fn block_intro(&self, hei: u64, history: &dyn base::BlockHistory) -> Ret<(u64, u32)> {
        let block = match history.block_at_height(hei) {
            Ok(Some(block)) => block,
            Ok(None) => return errf!("difficulty block missing: block_height={}", hei),
            Err(error) => return Err(error),
        };
        Ok((block.timestamp(), block.pow_difficulty()))
    }

    fn target_bootstrap(&self) -> DifficultyTarget {
        DifficultyTarget::from_num(LOWEST_DIFFICULTY)
    }

    /// Non-mainnet only: weighted sliding window until ASERT activates.
    fn target_weighted_sliding(
        &self,
        prevdiff: u32,
        prevblkt: u64,
        hei: u64,
        history: &dyn base::BlockHistory,
    ) -> Ret<DifficultyTarget> {
        let prevbign = u32_to_biguint(prevdiff);
        let mut observed: u128 = 0;
        let mut expected: u128 = 0;
        let group_target = (self.cnf.each_block_target_time * self.group_blocks()) as u128;
        let mut bound = hei - self.window_blocks() - 1;
        let mut prev_time = self.block_intro(bound, history)?.0;
        let last_group = self.window_groups() - 1;
        for i in 0..self.window_groups() {
            let next_time = if i == last_group {
                prevblkt
            } else {
                bound += self.group_blocks();
                self.block_intro(bound, history)?.0
            };
            let weight = (i + 1) as u128;
            observed += (next_time.saturating_sub(prev_time) as u128) * weight;
            expected += group_target * weight;
            prev_time = next_time;
        }
        Ok(DifficultyTarget::from_big(clamp_target_half_double(
            &prevbign,
            scale_target_by_ratio(&prevbign, observed, expected),
        )))
    }

    pub(crate) fn target_asert(
        &self,
        prevdiff: u32,
        hei: u64,
        blkt: u64,
        history: &dyn base::BlockHistory,
    ) -> Ret<DifficultyTarget> {
        let upgrade_hei = self.asert_upgrade_height();
        if hei == upgrade_hei {
            return Ok(DifficultyTarget::from_num(ASERT_START_TARGET_NUM));
        }
        let anchor_time = self.block_intro(upgrade_hei, history)?.0;
        let anchor_target = u32_to_biguint(ASERT_START_TARGET_NUM);
        let time_delta = blkt as i128 - anchor_time as i128;
        let height_delta = hei as i128 - upgrade_hei as i128;
        let exponent = ((time_delta - self.cnf.each_block_target_time as i128 * height_delta)
            * ASERT_RADIX as i128)
            / ASERT_HALF_LIFE as i128;
        let num_shifts = exponent >> ASERT_RADIX_BITS;
        let frac = (exponent - (num_shifts << ASERT_RADIX_BITS)) as u128;
        let frac2 = frac * frac;
        let frac3 = frac2 * frac;
        let factor = (((ASERT_POLY_1 * frac
            + ASERT_POLY_2 * frac2
            + ASERT_POLY_3 * frac3
            + (1u128 << (ASERT_POLY_TERM_SHIFT - 1)))
            >> ASERT_POLY_TERM_SHIFT)
            + ASERT_RADIX as u128) as u64;
        let prev_target = u32_to_biguint(prevdiff);
        let ease_target = prev_target * BigUint::from(ASERT_EASING_MAX_SCALE);
        let max_target = u32_to_biguint(LOWEST_DIFFICULTY);
        let mut next_target = anchor_target * BigUint::from(factor);
        if num_shifts < 0 {
            next_target >>= (-num_shifts) as usize;
        } else if num_shifts > 0 {
            next_target <<= num_shifts as usize;
        }
        next_target >>= ASERT_RADIX_BITS as usize;
        if next_target == BigUint::default() {
            return Ok(DifficultyTarget::from_big(BigUint::from(1u8)));
        }
        if next_target > ease_target {
            next_target = ease_target;
        }
        if next_target > max_target {
            return Ok(DifficultyTarget::from_num(LOWEST_DIFFICULTY));
        }
        Ok(DifficultyTarget::from_big(next_target))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DifficultyTarget {
    pub num: u32,
    pub hash: [u8; 32],
    pub big: BigUint,
}

impl DifficultyTarget {
    pub fn from_num(num: u32) -> Self {
        Self::from_big(u32_to_biguint(num))
    }

    pub fn from_big(big: BigUint) -> Self {
        let num = biguint_to_u32(&big);
        Self {
            num,
            hash: u32_to_hash(num),
            big,
        }
    }

    pub fn into_tuple(self) -> (u32, [u8; 32], BigUint) {
        (self.num, self.hash, self.big)
    }
}

pub fn rates_to_show(rates: f64) -> String {
    const VK: f64 = 1000.0;
    const HNS: [&str; 9] = ["K", "M", "G", "T", "P", "E", "Z", "Y", "B"];
    const HVS: [f64; 9] = [
        VK,
        VK * VK,
        VK * VK * VK,
        VK * VK * VK * VK,
        VK * VK * VK * VK * VK,
        VK * VK * VK * VK * VK * VK,
        VK * VK * VK * VK * VK * VK * VK,
        VK * VK * VK * VK * VK * VK * VK * VK,
        VK * VK * VK * VK * VK * VK * VK * VK * VK,
    ];

    if !rates.is_finite() || rates <= 0.0 {
        return "0.00H/s".to_owned();
    }
    if rates < VK {
        return format!("{:.2}H/s", rates);
    }
    let mut hsx = HVS.len() - 1;
    for (i, unit) in HVS.iter().enumerate() {
        if rates < unit * VK {
            hsx = i;
            break;
        }
    }
    let num = rates / HVS[hsx];
    if !num.is_finite() {
        return format!("{:.2e}H/s", rates);
    }
    format!("{:.2}{}H/s", num, HNS[hsx])
}

pub fn u32_to_rates(num: u32, secs: f64) -> f64 {
    hash_to_rates(&u32_to_hash(num), secs)
}

pub fn hash_to_rates(hash: &[u8; 32], secs: f64) -> f64 {
    if secs <= 0.0 || !secs.is_finite() {
        return 0.0;
    }
    hash_to_power(hash) / secs
}

pub fn hash_to_power(hash: &[u8; 32]) -> f64 {
    let target = BigUint::from_bytes_be(hash);
    if target == BigUint::from(0u8) {
        return 0.0;
    }
    let numerator = BigUint::from(1u8) << 256usize;
    let denominator = target + BigUint::from(1u8);
    let power =
        numerator.to_f64().unwrap_or(f64::INFINITY) / denominator.to_f64().unwrap_or(f64::INFINITY);
    if power.is_finite() { power } else { 0.0 }
}

pub fn u32_to_biguint(diff: u32) -> BigUint {
    BigUint::from_bytes_be(&u32_to_hash(diff))
}

pub fn biguint_to_hash(bn: &BigUint) -> [u8; 32] {
    let res = bn.to_bytes_be();
    if res.len() > 32 {
        // Targets above the 256-bit hash space are equivalent to the maximum representable target.
        return [255; 32];
    }
    let mut hash = [0u8; 32];
    hash[32 - res.len()..].copy_from_slice(&res);
    hash
}

/// Compact difficulty `u32` → 32-byte target hash (mainnet-compatible bit packing).
pub fn u32_to_hash(num: u32) -> [u8; 32] {
    if num == 0 {
        return [0; 32];
    }
    let numbts = num.to_be_bytes();
    let lzero = 255usize.saturating_sub(numbts[0] as usize);
    let bits2 = [
        byte_to_bits(numbts[1]),
        byte_to_bits(numbts[2]),
        byte_to_bits(numbts[3]),
    ]
    .concat();
    let keep = BITS.saturating_sub(lzero).min(bits2.len());
    let mut bits = Vec::with_capacity(BITS);
    bits.extend(std::iter::repeat(0u8).take(lzero));
    bits.extend_from_slice(&bits2[..keep]);
    bits.resize(BITS, 0);
    bits_to_bytes(bits.as_slice().try_into().unwrap())
}

pub fn hash_to_u32(hx: &[u8; 32]) -> u32 {
    let bits = bytes_to_bits(hx).to_vec();
    let lzero = left_zero(&bits);
    if lzero >= BITS {
        return 0;
    }
    let mut body = bits[lzero..].to_vec();
    body.truncate(24);
    body.resize(24, 1);
    let mut padded = body;
    padded.resize(BITS, 0);
    let reshx = bits_to_bytes(padded.as_slice().try_into().unwrap());
    let mut u32bts = [0u8; 4];
    u32bts[0] = 255 - lzero as u8;
    u32bts[1] = reshx[0];
    u32bts[2] = reshx[1];
    u32bts[3] = reshx[2];
    u32::from_be_bytes(u32bts)
}

/// Big-endian byte compare: `a > b`.
pub fn hash_bigger_than(a: &[u8], b: &[u8]) -> bool {
    let sz = a.len().min(b.len());
    for i in 0..sz {
        if a[i] > b[i] {
            return true;
        } else if a[i] < b[i] {
            return false;
        }
    }
    false
}

pub fn biguint_to_u32(big: &BigUint) -> u32 {
    hash_to_u32(&biguint_to_hash(big))
}

pub fn clamp_target_half_double(prev: &BigUint, next: BigUint) -> BigUint {
    let min = prev / BigUint::from(2u8);
    let max = prev * BigUint::from(2u8);
    if next < min {
        min
    } else if next > max {
        max
    } else {
        next
    }
}

pub fn scale_target_by_ratio(prev: &BigUint, observed: u128, expected: u128) -> BigUint {
    if expected == 0 {
        return prev.clone();
    }
    prev * BigUint::from(observed) / BigUint::from(expected)
}

const HXS: usize = 32;
const BITS: usize = HXS * 8;

fn left_zero(buf: &[u8]) -> usize {
    for (i, a) in buf.iter().enumerate() {
        if *a > 0 {
            return i;
        }
    }
    buf.len()
}

fn bits_to_bytes(bits: &[u8; BITS]) -> [u8; HXS] {
    let mut res = [0u8; HXS];
    for i in 0..HXS {
        let x = i * 8;
        res[i] = bits_to_byte(bits[x..x + 8].try_into().unwrap());
    }
    res
}

fn bytes_to_bits(bytes: &[u8; HXS]) -> [u8; BITS] {
    let mut res = [0u8; BITS];
    for (i, b) in bytes.iter().enumerate() {
        let bits = byte_to_bits(*b);
        res[i * 8..(i + 1) * 8].copy_from_slice(&bits);
    }
    res
}

fn bits_to_byte(bits: [u8; 8]) -> u8 {
    bits[7]
        + 2 * bits[6]
        + 4 * bits[5]
        + 8 * bits[4]
        + 16 * bits[3]
        + 32 * bits[2]
        + 64 * bits[1]
        + 128 * bits[0]
}

fn byte_to_bits(b: u8) -> [u8; 8] {
    [
        (b >> 7) & 0x1,
        (b >> 6) & 0x1,
        (b >> 5) & 0x1,
        (b >> 4) & 0x1,
        (b >> 3) & 0x1,
        (b >> 2) & 0x1,
        (b >> 1) & 0x1,
        (b >> 0) & 0x1,
    ]
}
