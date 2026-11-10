//! One height window for the upgrade that ships together: calc kernels and
//! the HACD smelt views (`hacd_smelt`, `hacd_number_batch`,
//! `hacd_visual_gene_batch`, `hacd_life_gene_batch`).
//!
//! Both are refused on the half-open interval
//! `[UPGRADE_DISABLE_FROM_HEIGHT, UPGRADE_DISABLE_UNTIL_HEIGHT)`.
//! Heights below the start stay open, so test chains and historical replay
//! keep the features. Once the chain is past `UPGRADE_DISABLE_UNTIL_HEIGHT`,
//! delete this module and the two call sites that consult it.

/// First height at which the upgrade is refused.
pub const UPGRADE_DISABLE_FROM_HEIGHT: u64 = 790_000;
/// First height at which the upgrade is available again.
pub const UPGRADE_DISABLE_UNTIL_HEIGHT: u64 = 800_000;

/// Whether calc deployment and the HACD smelt views may run at `height`.
pub const fn upgrade_enabled_at(height: u64) -> bool {
    height < UPGRADE_DISABLE_FROM_HEIGHT || height >= UPGRADE_DISABLE_UNTIL_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_is_disabled_only_inside_the_window() {
        assert!(upgrade_enabled_at(UPGRADE_DISABLE_FROM_HEIGHT - 1));
        assert!(!upgrade_enabled_at(UPGRADE_DISABLE_FROM_HEIGHT));
        assert!(!upgrade_enabled_at(UPGRADE_DISABLE_UNTIL_HEIGHT - 1));
        assert!(upgrade_enabled_at(UPGRADE_DISABLE_UNTIL_HEIGHT));
    }
}
