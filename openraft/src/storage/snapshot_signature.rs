use crate::LogId;
use crate::RaftTypeConfig;
use crate::SnapshotId;

/// A small piece of information for identifying a snapshot and error tracing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotSignature<C>
where C: RaftTypeConfig
{
    /// Log entries upto which this snapshot includes, inclusive.
    pub last_log_id: Option<LogId<C>>,

    /// The last applied membership log id.
    pub last_membership_log_id: Option<LogId<C>>,

    /// To identify a snapshot when transferring.
    pub snapshot_id: SnapshotId,
}
