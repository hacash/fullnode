//! Execute-free, versioned Hacash consensus parameters. `base` owns the
//! reusable shapes; this crate owns the standard network values (one profile for protocol, mint, SDK, app).

use base::{MintParams, VmExecutionParams};

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
    pub default_gas_budget: i64,
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
            initial_fee_purity_floor: 50_000,
            fee_purity_reductions: &[],
        },
        diamond_form_flag: 1,
        max_type3_signers: 200,
        tex_diamond_pay_max: 60_000,
        tex_diamond_get_max_per_tx: 200,
        tx_actions_max: 200,
        default_gas_budget: 1_000_000,
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
pub const DEFAULT_GAS_BUDGET: i64 = MAINNET_PARAMS.protocol.default_gas_budget;
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
    hasher.update(params.protocol.vm.initial_fee_purity_floor.to_be_bytes());
    hasher.update((params.protocol.vm.fee_purity_reductions.len() as u64).to_be_bytes());
    for &(height, floor) in params.protocol.vm.fee_purity_reductions {
        hasher.update(height.to_be_bytes());
        hasher.update(floor.to_be_bytes());
    }
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
    hasher.update(params.protocol.default_gas_budget.to_be_bytes());
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
        assert_eq!(MAINNET_PARAMS.mint.max_tx_size, 16 * 1024);
        assert_eq!(MAINNET_PARAMS.mint.max_block_txs, 1000);
        assert_eq!(MAINNET_PARAMS.protocol.tx_actions_max, 200);
        assert_eq!(MAINNET_PARAMS.protocol.default_gas_budget, 1_000_000);
        assert_eq!(MAINNET_PARAMS.protocol.tx_gas_budget_cap_byte, 99);
        assert_eq!(MAINNET_PARAMS.protocol.tx_type_2, 2);
    }

    #[test]
    fn mainnet_params_hash_is_locked() {
        assert_eq!(
            params_hash(&MAINNET_PARAMS),
            [
                212, 180, 77, 38, 118, 234, 47, 29, 136, 156, 23, 232, 198, 130, 118, 153, 9, 205,
                16, 91, 142, 14, 151, 110, 122, 132, 20, 183, 64, 150, 96, 252,
            ]
        );
    }
}
