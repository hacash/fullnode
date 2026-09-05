//! Execute-free, versioned Hacash consensus parameters. `base` owns the
//! reusable shapes; this crate owns the standard network values (one profile for protocol, mint, SDK, app).

use base::{ContractStorageFeeParams, MintParams, VmExecutionParams};

/// Mainnet contract storage discount parameters (§3.1 of the storage fee budget
/// design): decimal 1 MB capacity refilled at 1,000 bytes/block over a 1000-block
/// target period, activation `H0` pending governance freeze.
pub const MAINNET_CONTRACT_STORAGE_FEE: ContractStorageFeeParams = ContractStorageFeeParams {
    rule_version: base::CONTRACT_STORAGE_RULE_V1,
    activation_height: 784_000,
    target_capacity_blocks: 1_000,
    curve_steps: 1_000,
    period_floor: 10,
    max_block_discount_bytes: 16 * 1024,
    supplement_schedule: &[(784_000, 1_000)],
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolParams {
    pub ast_tree_depth_max: usize,
    pub ast_snapshot_try_gas: i64,
    pub vm: VmExecutionParams,
    pub diamond_form_flag: u64,
    pub max_type3_signers: usize,
    pub tex_diamond_pay_max: usize,
    pub tex_diamond_get_max_per_tx: usize,
    pub tx_actions_max: usize,
    pub tx_gas_budget_cap_byte: u8,
    pub tx_type_1: u8,
    pub tx_type_2: u8,
    pub tx_type_3: u8,
    pub type1_deprecated_after_height: u64,
    pub fee_size_limit_after_height: u64,
    pub max_fee_size_after_limit_height: usize,
    pub gas_budget_lookup: &'static [u32; 256],
}

impl ProtocolParams {
    #[inline(always)]
    pub const fn decode_gas_budget(&self, byte: u8) -> i64 {
        self.gas_budget_lookup[byte as usize] as i64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiamondRules {
    pub custom_message_after: u32,
    pub burn_90_percent_after: u32,
    pub average_bid_burn_after: u32,
    pub visual_gene_block_hash_after: u32,
    pub visual_gene_bid_fee_after: u32,
    pub minimum_bid_after: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InscriptionRules {
    pub cooldown_blocks: u64,
    pub content_max_bytes: usize,
    pub readable_type_max: u8,
    pub max_per_diamond: usize,
    pub append_free_max: usize,
    pub append_tier1_max: usize,
    pub append_tier2_max: usize,
}

impl InscriptionRules {
    pub fn append_cost(&self, current: usize, average_bid_burn_mei: u16) -> field::Amount {
        let multiplier = if current < self.append_free_max {
            0
        } else if current < self.append_tier1_max {
            2
        } else if current < self.append_tier2_max {
            5
        } else {
            10
        };
        field::Amount::coin(average_bid_burn_mei as u64 * multiplier, 246)
    }

    pub fn edit_cost(&self, average_bid_burn_mei: u16) -> field::Amount {
        field::Amount::coin(average_bid_burn_mei as u64, 246)
    }

    pub fn drop_cost(&self, average_bid_burn_mei: u16) -> field::Amount {
        field::Amount::coin(average_bid_burn_mei as u64 * 2, 246)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MintRules {
    pub asset_alive_height: u64,
    pub asset_mainnet_min_serial: u64,
    pub asset_non_mainnet_alive_height: u64,
    pub asset_non_mainnet_min_serial: u64,
    pub diamond: DiamondRules,
    pub inscription: InscriptionRules,
    pub block_reward_step_blocks: u64,
    pub block_reward_schedule: &'static [u8; 66],
}

impl MintRules {
    pub fn block_reward_number(&self, block_height: u64) -> u8 {
        let step = block_height / self.block_reward_step_blocks;
        self.block_reward_schedule
            .get(step as usize)
            .copied()
            .unwrap_or(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HacashParams {
    pub version: u32,
    pub protocol: ProtocolParams,
    pub mint: MintParams,
    pub mint_rules: MintRules,
}

/// The sole standard-network parameter initialization.
pub const MAINNET_PARAMS: HacashParams = HacashParams {
    version: 1,
    protocol: ProtocolParams {
        ast_tree_depth_max: 6,
        ast_snapshot_try_gas: 40,
        vm: VmExecutionParams {
            contract_store_perm_periods: 10_000,
            contract_storage_fee: MAINNET_CONTRACT_STORAGE_FEE,
            initial_fee_purity_floor: 50_000,
            fee_purity_reductions: &[],
            gas_budget_lookup: &GAS_BUDGET_LOOKUP_1P07_FROM_138,
            tx_gas_budget_cap_byte: 99,
            compute_limit_byte: 72,
            resource_limit_byte: 56,
            storage_limit_byte: 99,
        },
        diamond_form_flag: 1,
        max_type3_signers: 200,
        tex_diamond_pay_max: 60_000,
        tex_diamond_get_max_per_tx: 200,
        tx_actions_max: 200,
        tx_gas_budget_cap_byte: 99,
        tx_type_1: 1,
        tx_type_2: 2,
        tx_type_3: 3,
        type1_deprecated_after_height: 33_033,
        fee_size_limit_after_height: 200_000,
        max_fee_size_after_limit_height: 6,
        gas_budget_lookup: &GAS_BUDGET_LOOKUP_1P07_FROM_138,
    },
    mint: MintParams {
        max_block_txs: 1000,
        max_block_size: 1024 * 1024,
        max_tx_size: 16 * 1024,
        difficulty_adjust_blocks: 288,
        difficulty_group_blocks: 4,
        each_block_target_time: 300,
    },
    mint_rules: MintRules {
        asset_alive_height: 765_432,
        asset_mainnet_min_serial: 1025,
        asset_non_mainnet_alive_height: 0,
        asset_non_mainnet_min_serial: 5,
        diamond: DiamondRules {
            custom_message_after: 20_000,
            burn_90_percent_after: 30_000,
            average_bid_burn_after: 40_000,
            visual_gene_block_hash_after: 40_000,
            visual_gene_bid_fee_after: 41_000,
            minimum_bid_after: 107_000,
        },
        inscription: InscriptionRules {
            cooldown_blocks: 200,
            content_max_bytes: 64,
            readable_type_max: 100,
            max_per_diamond: 200,
            append_free_max: 10,
            append_tier1_max: 40,
            append_tier2_max: 100,
        },
        block_reward_step_blocks: 100_000,
        block_reward_schedule: &BLOCK_REWARD_SCHEDULE,
    },
};

pub const MAX_TX_SIZE: usize = MAINNET_PARAMS.mint.max_tx_size;
pub const TX_ACTIONS_MAX: usize = MAINNET_PARAMS.protocol.tx_actions_max;
pub const TX_TYPE_1: u8 = MAINNET_PARAMS.protocol.tx_type_1;
pub const TX_TYPE_2: u8 = MAINNET_PARAMS.protocol.tx_type_2;
pub const TX_TYPE_3: u8 = MAINNET_PARAMS.protocol.tx_type_3;
pub const TX_GAS_BUDGET_CAP_BYTE: u8 = MAINNET_PARAMS.protocol.tx_gas_budget_cap_byte;

/// Height after which type-1 user transactions are rejected at execute.
pub const TYPE1_DEPRECATED_AFTER_HEIGHT: u64 =
    MAINNET_PARAMS.protocol.type1_deprecated_after_height;
/// Height after which the fee encoding may not exceed `MAX_FEE_SIZE_AFTER_LIMIT_HEIGHT` bytes.
pub const FEE_SIZE_LIMIT_AFTER_HEIGHT: u64 = MAINNET_PARAMS.protocol.fee_size_limit_after_height;
pub const MAX_FEE_SIZE_AFTER_LIMIT_HEIGHT: usize =
    MAINNET_PARAMS.protocol.max_fee_size_after_limit_height;

pub const BLOCK_REWARD_SCHEDULE: [u8; 66] = [
    1, 1, 2, 3, 5, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1,
];

/// Compatibility names for code that consumes the standard profile directly.
/// Consensus execution uses the injected `MintRules` instead.
pub const BLOCK_REWARD_STEP_BLOCK: u64 = MAINNET_PARAMS.mint_rules.block_reward_step_blocks;
pub const BLOCK_REWARD_DEF_LIST: [u8; 66] = BLOCK_REWARD_SCHEDULE;

#[inline(always)]
pub fn block_reward_number(block_height: u64) -> u8 {
    MAINNET_PARAMS.mint_rules.block_reward_number(block_height)
}

/// Stable SHA3-256 fingerprint of a complete network parameter profile. Integers
/// use fixed-width big-endian encodings so the fingerprint is independent of pointer width.
pub fn params_hash(params: &HacashParams) -> [u8; 32] {
    use sha3::{Digest, Sha3_256};

    let mut hasher = Sha3_256::new();
    hasher.update(b"hacash-params/v1\0");
    hasher.update(params.version.to_be_bytes());
    hasher.update((params.protocol.ast_tree_depth_max as u64).to_be_bytes());
    hasher.update(params.protocol.ast_snapshot_try_gas.to_be_bytes());
    hasher.update(params.protocol.vm.contract_store_perm_periods.to_be_bytes());
    let csf = params.protocol.vm.contract_storage_fee;
    hasher.update(b"hacash-csf/v1\0");
    hasher.update([csf.rule_version]);
    hasher.update(csf.activation_height.to_be_bytes());
    hasher.update(csf.target_capacity_blocks.to_be_bytes());
    hasher.update(csf.curve_steps.to_be_bytes());
    hasher.update(csf.period_floor.to_be_bytes());
    hasher.update(csf.max_block_discount_bytes.to_be_bytes());
    hasher.update((csf.supplement_schedule.len() as u64).to_be_bytes());
    for &(height, rate) in csf.supplement_schedule {
        hasher.update(height.to_be_bytes());
        hasher.update(rate.to_be_bytes());
    }
    hasher.update(params.protocol.vm.initial_fee_purity_floor.to_be_bytes());
    hasher.update((params.protocol.vm.fee_purity_reductions.len() as u64).to_be_bytes());
    for &(height, floor) in params.protocol.vm.fee_purity_reductions {
        hasher.update(height.to_be_bytes());
        hasher.update(floor.to_be_bytes());
    }
    hasher.update([
        params.protocol.vm.tx_gas_budget_cap_byte,
        params.protocol.vm.compute_limit_byte,
        params.protocol.vm.resource_limit_byte,
        params.protocol.vm.storage_limit_byte,
    ]);
    hasher.update(params.protocol.diamond_form_flag.to_be_bytes());
    hasher.update((params.protocol.max_type3_signers as u64).to_be_bytes());
    hasher.update((params.protocol.tex_diamond_pay_max as u64).to_be_bytes());
    hasher.update((params.protocol.tex_diamond_get_max_per_tx as u64).to_be_bytes());
    hasher.update((params.mint.max_block_txs as u64).to_be_bytes());
    hasher.update((params.mint.max_block_size as u64).to_be_bytes());
    hasher.update((params.mint.max_tx_size as u64).to_be_bytes());
    hasher.update(params.mint.difficulty_adjust_blocks.to_be_bytes());
    hasher.update(params.mint.difficulty_group_blocks.to_be_bytes());
    hasher.update(params.mint.each_block_target_time.to_be_bytes());
    hasher.update((params.protocol.tx_actions_max as u64).to_be_bytes());
    hasher.update([
        params.protocol.tx_gas_budget_cap_byte,
        params.protocol.tx_type_1,
        params.protocol.tx_type_2,
        params.protocol.tx_type_3,
    ]);
    hasher.update(params.protocol.type1_deprecated_after_height.to_be_bytes());
    hasher.update(params.protocol.fee_size_limit_after_height.to_be_bytes());
    hasher.update((params.protocol.max_fee_size_after_limit_height as u64).to_be_bytes());
    for value in params.protocol.gas_budget_lookup {
        hasher.update(value.to_be_bytes());
    }
    hasher.update(params.mint_rules.asset_alive_height.to_be_bytes());
    hasher.update(params.mint_rules.asset_mainnet_min_serial.to_be_bytes());
    hasher.update(
        params
            .mint_rules
            .asset_non_mainnet_alive_height
            .to_be_bytes(),
    );
    hasher.update(params.mint_rules.asset_non_mainnet_min_serial.to_be_bytes());
    let diamond = params.mint_rules.diamond;
    for value in [
        diamond.custom_message_after,
        diamond.burn_90_percent_after,
        diamond.average_bid_burn_after,
        diamond.visual_gene_block_hash_after,
        diamond.visual_gene_bid_fee_after,
        diamond.minimum_bid_after,
    ] {
        hasher.update(value.to_be_bytes());
    }
    let inscription = params.mint_rules.inscription;
    hasher.update(inscription.cooldown_blocks.to_be_bytes());
    hasher.update((inscription.content_max_bytes as u64).to_be_bytes());
    hasher.update([inscription.readable_type_max]);
    for value in [
        inscription.max_per_diamond,
        inscription.append_free_max,
        inscription.append_tier1_max,
        inscription.append_tier2_max,
    ] {
        hasher.update((value as u64).to_be_bytes());
    }
    hasher.update(params.mint_rules.block_reward_step_blocks.to_be_bytes());
    hasher.update(params.mint_rules.block_reward_schedule);
    hasher.finalize().into()
}

/// Obtain the standard profile installed by an application composition root.
/// The caller owns the registry abstraction; this crate owns the concrete type and its downcast.
pub fn as_hacash_params(
    profile: &'static dyn base::ExecutionProfile,
) -> Option<&'static HacashParams> {
    profile.as_any().downcast_ref::<HacashParams>()
}

/// Hacash transaction/VM gas budget schedule. VM resource accounting itself
/// remains in `base`; the price table and cap are network parameters.
pub const GAS_BUDGET_LOOKUP_1P07_FROM_138: [u32; 256] = [
    0, 147, 157, 169, 180, 193, 207, 221, 237, 253, 271, 290, 310, 332, 355, 380, 407, 435, 466,
    499, 534, 571, 611, 654, 699, 748, 801, 857, 917, 981, 1050, 1124, 1202, 1286, 1376, 1473,
    1576, 1686, 1804, 1931, 2066, 2211, 2365, 2531, 2708, 2898, 3101, 3318, 3550, 3799, 4065, 4349,
    4654, 4979, 5328, 5701, 6100, 6527, 6984, 7473, 7996, 8556, 9155, 9796, 10481, 11215, 12000,
    12840, 13739, 14701, 15730, 16831, 18009, 19270, 20619, 22062, 23607, 25259, 27027, 28919,
    30944, 33110, 35428, 37908, 40561, 43401, 46439, 49689, 53168, 56889, 60872, 65133, 69692,
    74571, 79791, 85376, 91352, 97747, 104589, 111911, 119744, 128126, 137095, 146692, 156961,
    167948, 179704, 192284, 205743, 220146, 235556, 252045, 269688, 288566, 308766, 330379, 353506,
    378251, 404729, 433060, 463374, 495811, 530517, 567654, 607389, 649907, 695400, 744078, 796164,
    851895, 911528, 975335, 1043608, 1116661, 1194827, 1278465, 1367958, 1463715, 1566175, 1675807,
    1793114, 1918632, 2052936, 2196642, 2350407, 2514935, 2690980, 2879349, 3080904, 3296567,
    3527327, 3774240, 4038436, 4321127, 4623606, 4947258, 5293566, 5664116, 6060604, 6484847,
    6938786, 7424501, 7944216, 8500311, 9095333, 9732006, 10413247, 11142174, 11922126, 12756675,
    13649643, 14605118, 15627476, 16721399, 17891897, 19144330, 20484433, 21918343, 23452627,
    25094311, 26850913, 28730477, 30741611, 32893523, 35196070, 37659795, 40295981, 43116699,
    46134868, 49364309, 52819811, 56517198, 60473402, 64706540, 69235998, 74082517, 79268294,
    84817074, 90754270, 97107068, 103904563, 111177883, 118960335, 127287558, 136197687, 145731525,
    155932732, 166848023, 178527385, 191024302, 204396003, 218703723, 234012984, 250393893,
    267921466, 286675968, 306743286, 328215316, 351190388, 375773715, 402077876, 430223327,
    460338960, 492562687, 527042075, 563935020, 603410472, 645649205, 690844649, 739203775,
    790948039, 846314402, 905556410, 968945359, 1036771534, 1109345541, 1186999729, 1270089710,
    1358995990, 1454125710, 1555914509, 1664828525, 1781366522, 1906062178, 2039486531, 2182250588,
    2335008129, 2498458698, 2673350807, 2860485364, 3060719339, 3274969693, 3504217571, 3749512802,
    4011978698, 4292817207,
];

#[inline(always)]
pub const fn decode_gas_budget(b: u8) -> i64 {
    GAS_BUDGET_LOOKUP_1P07_FROM_138[b as usize] as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_profile_is_self_consistent() {
        assert_eq!(MAINNET_PARAMS.protocol.ast_tree_depth_max, 6);
        assert_eq!(MAINNET_PARAMS.protocol.diamond_form_flag, 1);
        assert_eq!(MAINNET_PARAMS.protocol.vm.initial_fee_purity_floor, 50_000);
        assert_eq!(MAINNET_PARAMS.protocol.vm.tx_gas_budget_cap_byte, 99);
        assert_eq!(MAINNET_PARAMS.protocol.vm.compute_limit_byte, 72);
        assert_eq!(MAINNET_PARAMS.protocol.vm.resource_limit_byte, 56);
        assert_eq!(MAINNET_PARAMS.protocol.vm.storage_limit_byte, 99);
        assert_eq!(
            MAINNET_PARAMS.protocol.vm.gas_budget_lookup as *const [u32; 256],
            MAINNET_PARAMS.protocol.gas_budget_lookup as *const [u32; 256]
        );
        assert_eq!(MAINNET_PARAMS.mint.max_tx_size, 16 * 1024);
        assert_eq!(MAINNET_PARAMS.mint.max_block_txs, 1000);
        assert_eq!(MAINNET_PARAMS.protocol.tx_actions_max, 200);
        assert_eq!(MAINNET_PARAMS.protocol.tx_gas_budget_cap_byte, 99);
        assert_eq!(MAINNET_PARAMS.protocol.tx_type_2, 2);
    }

    /// Published consensus fingerprint after the storage-fee-budget upgrade:
    /// `ef9f644f5d4e3de53428ce32124d8e4536a8a65786c1b28d02abc0d8f43693a8`
    /// (v1 profile + §3.1 discount parameters, H0 = 784,000). Any change to the
    /// storage fee schedule or activation height must move this value (A14).
    #[test]
    fn mainnet_params_hash_is_locked() {
        assert_eq!(
            params_hash(&MAINNET_PARAMS),
            [
                239, 159, 100, 79, 93, 78, 61, 229, 52, 40, 206, 50, 18, 77, 142, 69, 54, 168, 166,
                87, 134, 193, 178, 141, 2, 171, 192, 216, 244, 54, 147, 168,
            ]
        );
    }

    /// §3.1 design matrix, frozen as a test: decimal 1 MB capacity, 1000-block
    /// target period, 1,000 bytes/block fill rate, activation on a 1000-multiple.
    #[test]
    fn mainnet_contract_storage_fee_matches_design_matrix() {
        let csf = MAINNET_CONTRACT_STORAGE_FEE;
        let p_max = MAINNET_PARAMS.protocol.vm.contract_store_perm_periods;
        assert_eq!(csf.rule_version, base::CONTRACT_STORAGE_RULE_V1);
        assert_eq!(csf.activation_height, 784_000);
        assert_eq!(csf.activation_height % 1000, 0);
        assert_eq!(csf.target_capacity_blocks, 1_000);
        assert_eq!(csf.curve_steps, 1_000);
        assert_eq!(csf.period_floor, 10);
        assert_eq!(p_max, 10_000);
        assert_eq!(csf.max_block_discount_bytes, 16 * 1024);
        // Decimal 1 MB, NOT 1 MiB: R0 × T = C0 exactly (§3.2).
        assert_eq!(csf.capacity_at(csf.activation_height).unwrap(), 1_000_000);
        assert_ne!(csf.capacity_at(csf.activation_height).unwrap(), 1_048_576);
        // K_max > R0, otherwise the budget can never drain and the price cannot
        // recover (§3.1). K_max equals the current max tx size.
        assert!(csf.max_block_discount_bytes > 1_000);
        assert_eq!(
            csf.max_block_discount_bytes as usize,
            MAINNET_PARAMS.mint.max_tx_size
        );
        csf.validate(p_max).expect("mainnet storage fee params must validate");
    }

    /// §3.3/§3.4 fixed price vectors over the linear integer ceil curve.
    #[test]
    fn mainnet_contract_storage_price_curve_vectors() {
        let csf = MAINNET_CONTRACT_STORAGE_FEE;
        let c = 1_000_000u128;
        let p_max = 10_000u64;
        let periods = |remaining: u128| csf.discount_periods(remaining, c, p_max).unwrap();
        assert_eq!(periods(c), 10); // B=C floor, never below P_min
        assert_eq!(periods(c * 9 / 10), 1_000); // 90% remaining
        assert_eq!(periods(500_000), 5_000); // 50% remaining
        assert_eq!(periods(c / 10), 9_000); // 10% remaining
        assert_eq!(periods(0), 10_000); // B=0 → full price, never above
        // Integer ceil rounding: 1 byte of usage still sits in step 1; step 2
        // starts just after the 1000-byte granularity.
        assert_eq!(periods(c - 1), 10);
        assert_eq!(periods(c - 1_000), 10);
        assert_eq!(periods(c - 1_001), 20);
        // Monotonicity over all reachable capacities.
        let mut last = periods(0);
        for b in (1..=c).step_by(7919) {
            let p = periods(b);
            assert!(p <= last, "periods must not rise as remaining grows");
            assert!((10..=p_max).contains(&p));
            last = p;
        }
        // Out-of-range inputs clamp into the curve, never panic.
        assert_eq!(periods(c + 12345), 10);
    }

    /// The height-gated schedule must stay append-only monotone (§7.1/§7.4):
    /// every violation is rejected by `validate`.
    #[test]
    fn storage_schedule_rejects_non_governance_changes() {
        let base_pmax = MAINNET_PARAMS.protocol.vm.contract_store_perm_periods;
        let mk = |supplement_schedule: &'static [(u64, u64)]| ContractStorageFeeParams {
            supplement_schedule,
            activation_height: 784_000,
            ..MAINNET_CONTRACT_STORAGE_FEE
        };
        // valid expansion: R 1000 → 2000 (ΔR is a 1000-multiple, C = T×R = 2 MB)
        mk(&[(784_000, 1_000), (1_000_000, 2_000)])
            .validate(base_pmax)
            .expect("governance expansion must validate");
        // rate decrease (would raise future prices) rejected
        assert!(mk(&[(784_000, 1_000), (1_000_000, 500)]).validate(base_pmax).is_err());
        // height not a multiple of T rejected
        assert!(mk(&[(784_000, 1_000), (1_000_500, 2_000)]).validate(base_pmax).is_err());
        // non-increasing heights rejected
        assert!(mk(&[(784_000, 1_000), (784_000, 2_000)]).validate(base_pmax).is_err());
        // schedule must start at H0
        assert!(mk(&[(781_000, 1_000)]).validate(base_pmax).is_err());
        // fine-grained delta (governance must review separately) rejected
        assert!(mk(&[(784_000, 1_000), (1_000_000, 1_500)]).validate(base_pmax).is_err());
        // K_max must stay strictly above every rate
        assert!(
            ContractStorageFeeParams {
                max_block_discount_bytes: 1_000,
                ..MAINNET_CONTRACT_STORAGE_FEE
            }
            .validate(base_pmax)
            .is_err()
        );
        // curve constants are frozen by rule version: any tweak fails validation
        assert!(
            ContractStorageFeeParams {
                period_floor: 11,
                ..MAINNET_CONTRACT_STORAGE_FEE
            }
            .validate(base_pmax)
            .is_err()
        );
    }

    /// Every schedule/parameter change must move the consensus params hash.
    #[test]
    fn storage_fee_params_are_hash_committed() {
        let mut altered = MAINNET_PARAMS;
        let schedule: &'static [(u64, u64)] = Box::leak(vec![(784_000, 1_000), (1_000_000, 2_000)].into_boxed_slice());
        altered.protocol.vm.contract_storage_fee.supplement_schedule = schedule;
        assert_ne!(params_hash(&MAINNET_PARAMS), params_hash(&altered));
        let mut altered2 = MAINNET_PARAMS;
        altered2.protocol.vm.contract_storage_fee.activation_height = 779_000;
        assert_ne!(params_hash(&MAINNET_PARAMS), params_hash(&altered2));
    }
}
