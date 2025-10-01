use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// The state about logs.
///
/// Invariance: last_purged_log_id <= last_applied <= last_log_id
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogState<C: RaftTypeConfig> {
    /// The greatest log id that has been purged after being applied to state machine.
    pub last_purged_log_id: Option<LogIdOf<C>>,

    /// The log id of the last present entry if there are any entries.
    /// Otherwise, the same value as `last_purged_log_id`.
    pub last_log_id: Option<LogIdOf<C>>,
}
