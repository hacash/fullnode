//! Height windows for temporarily disabled features.
//!
//! Two windows coexist:
//!
//! * `TexCellRepeat` (kind 23) is refused on the half-open interval
//!   `[TEX_REPEAT_DISABLE_FROM_HEIGHT, TEX_REPEAT_DISABLE_UNTIL_HEIGHT)`.
//!   Heights below the start stay open, so test chains and historical replay
//!   keep the action. From `TEX_REPEAT_DISABLE_UNTIL_HEIGHT` it stays enabled.
//! * The fitsh HVM opcodes `ITEMSET` / `BUFSET` / `ITEMCAT` are refused on the
//!   same-shaped interval
//!   `[FITSH_OPCODES_DISABLE_FROM_HEIGHT, FITSH_OPCODES_DISABLE_UNTIL_HEIGHT)`.
//!   The interpreter dispatch returns `InstDisabled` inside the window; this is
//!   an upgrade-window refusal, not an opcode semantic.
//!
//! The previous calc / HACD smelt freeze (`[790_000, 800_000)`) has expired
//! and is gone: those features are unconditional. Delete each window's
//! constants once the chain is past its height.

/// First height at which `TexCellRepeat` is refused.
pub const TEX_REPEAT_DISABLE_FROM_HEIGHT: u64 = 800_000;
/// First height at which `TexCellRepeat` is available again.
pub const TEX_REPEAT_DISABLE_UNTIL_HEIGHT: u64 = 802_000;

/// Whether kind 23 may run at `height`.
pub const fn tex_repeat_enabled_at(height: u64) -> bool {
    height < TEX_REPEAT_DISABLE_FROM_HEIGHT || height >= TEX_REPEAT_DISABLE_UNTIL_HEIGHT
}

/// First height at which the fitsh HVM opcodes (ITEMSET / BUFSET / ITEMCAT)
/// are refused.
pub const FITSH_OPCODES_DISABLE_FROM_HEIGHT: u64 = 800_000;
/// First height at which the fitsh HVM opcodes are available again.
pub const FITSH_OPCODES_DISABLE_UNTIL_HEIGHT: u64 = 802_000;

/// Whether the fitsh HVM opcodes may run at `height`.
pub const fn fitsh_opcodes_enabled_at(height: u64) -> bool {
    height < FITSH_OPCODES_DISABLE_FROM_HEIGHT || height >= FITSH_OPCODES_DISABLE_UNTIL_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tex_repeat_is_disabled_only_inside_the_window() {
        assert!(tex_repeat_enabled_at(TEX_REPEAT_DISABLE_FROM_HEIGHT - 1));
        assert!(!tex_repeat_enabled_at(TEX_REPEAT_DISABLE_FROM_HEIGHT));
        assert!(!tex_repeat_enabled_at(TEX_REPEAT_DISABLE_UNTIL_HEIGHT - 1));
        assert!(tex_repeat_enabled_at(TEX_REPEAT_DISABLE_UNTIL_HEIGHT));
    }

    #[test]
    fn fitsh_opcodes_are_disabled_only_inside_the_window() {
        assert!(fitsh_opcodes_enabled_at(FITSH_OPCODES_DISABLE_FROM_HEIGHT - 1));
        assert!(!fitsh_opcodes_enabled_at(FITSH_OPCODES_DISABLE_FROM_HEIGHT));
        assert!(!fitsh_opcodes_enabled_at(FITSH_OPCODES_DISABLE_UNTIL_HEIGHT - 1));
        assert!(fitsh_opcodes_enabled_at(FITSH_OPCODES_DISABLE_UNTIL_HEIGHT));
    }

    /// Both windows are the same `[800_000, 802_000)` interval: one upgrade
    /// window refuses both the action and the opcodes.
    #[test]
    fn fitsh_opcode_window_matches_tex_repeat_window() {
        assert_eq!(
            (FITSH_OPCODES_DISABLE_FROM_HEIGHT, FITSH_OPCODES_DISABLE_UNTIL_HEIGHT),
            (TEX_REPEAT_DISABLE_FROM_HEIGHT, TEX_REPEAT_DISABLE_UNTIL_HEIGHT)
        );
        for height in [0u64, 799_999, 800_000, 801_111, 801_999, 802_000, u64::MAX] {
            assert_eq!(
                fitsh_opcodes_enabled_at(height),
                tex_repeat_enabled_at(height),
                "height {height}"
            );
        }
    }
}

