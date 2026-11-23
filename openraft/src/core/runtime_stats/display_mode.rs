use openraft_macros::since;

/// Display mode for [`RuntimeStatsDisplay`](super::RuntimeStatsDisplay).
#[since(version = "0.10.2")]
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
