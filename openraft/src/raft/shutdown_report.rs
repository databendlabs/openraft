use openraft_macros::since;

use crate::raft::TaskJoinOutcome;

/// Join outcomes of the Raft core and tick tasks.
///
/// Outcomes are retained across shutdown calls, including calls to [`Raft::shutdown`].
/// A successful join says that the task returned; it does not describe its return value or
/// certify that other background tasks or application resources have been shut down.
///
/// [`Raft::shutdown`]: crate::Raft::shutdown
#[since(version = "0.10.0", change = "added core and tick task shutdown report")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    /// The core task's join outcome.
    pub core: TaskJoinOutcome,
    /// The tick task's join outcome.
    pub tick: TaskJoinOutcome,
    /// Whether the core join had completed before this call tried to join it.
    pub core_already_joined: bool,
    /// Whether the tick join had completed before this call tried to join it.
    pub tick_already_joined: bool,
}
