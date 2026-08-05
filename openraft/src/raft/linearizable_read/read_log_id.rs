use std::fmt;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// The least log ID through which a state machine must apply before linearizing a read.
///
/// This boundary is inclusive: the state machine must apply this log ID or a later one.
///
/// A read log ID is the maximum of the producing leader's no-op log ID and the local committed log
/// ID. Therefore, it belongs to the leadership that produced it, but the log entry itself may not
/// yet be committed or applied.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "", transparent)
)]
pub struct ReadLogId<C>
where C: RaftTypeConfig
{
    log_id: LogIdOf<C>,
}

impl<C> Copy for ReadLogId<C>
where
    C: RaftTypeConfig,
    LogIdOf<C>: Copy,
{
}

impl<C> fmt::Display for ReadLogId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.log_id.fmt(f)
    }
}

impl<C> ReadLogId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(noop_log_id: LogIdOf<C>, committed: Option<LogIdOf<C>>) -> Self {
        let log_id = match committed {
            Some(committed) => std::cmp::max(noop_log_id.clone(), committed),
            None => noop_log_id.clone(),
        };

        debug_assert_eq!(
            log_id.committed_leader_id(),
            noop_log_id.committed_leader_id(),
            "read log ID must belong to the producing leadership"
        );

        Self { log_id }
    }

    /// Rebuild a `ReadLogId` from a log ID previously produced by a leader.
    ///
    /// Use this on a follower to reconstruct the read log ID received through an
    /// application-defined wire format when implementing follower reads. The caller is
    /// responsible for the value originating from a leader, such as via
    /// [`LinearizeState::read_log_id()`]; this method does not verify it.
    ///
    /// [`LinearizeState::read_log_id()`]: crate::raft::linearizable_read::LinearizeState::read_log_id
    #[since(version = "0.10.0")]
    pub fn from_log_id(log_id: LogIdOf<C>) -> Self {
        Self { log_id }
    }

    /// Return the underlying log ID.
    #[since(version = "0.10.0")]
    pub fn log_id(&self) -> &LogIdOf<C> {
        &self.log_id
    }

    /// Return the ID of the leadership that produced this read log ID.
    #[since(version = "0.10.0")]
    pub fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        self.log_id.committed_leader_id()
    }

    /// Return the log index of this read log ID.
    #[since(version = "0.10.0")]
    pub fn index(&self) -> u64 {
        self.log_id.index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    #[test]
    fn test_new() {
        let noop = log_id(2, 1, 2);

        let got = ReadLogId::<UTConfig>::new(noop, None);
        assert_eq!(ReadLogId { log_id: noop }, got);

        let got = ReadLogId::<UTConfig>::new(noop, Some(log_id(1, 1, 3)));
        assert_eq!(ReadLogId { log_id: noop }, got);

        let committed = log_id(2, 1, 4);
        let got = ReadLogId::<UTConfig>::new(noop, Some(committed));
        assert_eq!(ReadLogId { log_id: committed }, got);
    }

    #[test]
    fn test_from_log_id() {
        let log_id = log_id(2, 1, 3);
        let got = ReadLogId::<UTConfig>::from_log_id(log_id);
        assert_eq!(ReadLogId { log_id }, got);
    }

    #[test]
    fn test_accessors() {
        let log_id = log_id(2, 1, 3);
        let read_log_id = ReadLogId::<UTConfig>::new(log_id, None);

        assert_eq!(&log_id, read_log_id.log_id());
        assert_eq!(log_id.committed_leader_id(), read_log_id.committed_leader_id());
        assert_eq!(log_id.index(), read_log_id.index());
    }
}
