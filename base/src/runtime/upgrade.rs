//! Height window for `TexCellRepeat` (kind 23).
//!
//! The action is refused on the half-open interval
//! `[TEX_REPEAT_DISABLE_FROM_HEIGHT, TEX_REPEAT_DISABLE_UNTIL_HEIGHT)`.
//! Heights below the start stay open, so test chains and historical replay
//! keep the action. From `TEX_REPEAT_DISABLE_UNTIL_HEIGHT` it stays enabled;
//! delete this module once the chain is past that height.
//!
//! The previous calc / HACD smelt freeze (`[790_000, 800_000)`) has expired
//! and is gone: those features are unconditional.

/// First height at which `TexCellRepeat` is refused.
pub const TEX_REPEAT_DISABLE_FROM_HEIGHT: u64 = 800_000;
/// First height at which `TexCellRepeat` is available again.
pub const TEX_REPEAT_DISABLE_UNTIL_HEIGHT: u64 = 802_000;

/// Whether kind 23 may run at `height`.
pub const fn tex_repeat_enabled_at(height: u64) -> bool {
    height < TEX_REPEAT_DISABLE_FROM_HEIGHT || height >= TEX_REPEAT_DISABLE_UNTIL_HEIGHT
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
}
