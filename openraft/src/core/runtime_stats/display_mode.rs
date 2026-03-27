#[cfg(doc)]
use crate::core::runtime_stats::RuntimeStatsDisplay;

/// Display mode for [`RuntimeStatsDisplay`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// Compact single-line format (default).
    #[default]
    Compact,
    /// Multiline format with one piece of information per line.
    Multiline,
    /// Human-readable format with better formatting and spacing.
    HumanReadable,
}
