use crate::RaftTypeConfig;
use crate::log_id::raft_log_id::RaftLogId;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::log_id::ref_log_id::RefLogId;
use crate::type_config::alias::CommittedLeaderIdOf;

/// This helper trait extracts information from an `Option<T>` where T impls [`RaftLogId`].
pub(crate) trait OptionRaftLogIdExt<C>
where C: RaftTypeConfig
{
    /// Returns the log index if it is not a `None`.
    fn index(&self) -> Option<u64>;

    /// Returns the next log index.
    ///
    /// If self is `None`, it returns 0.
    fn next_index(&self) -> u64;

    /// Returns the leader id that proposed this log id.
    ///
    /// In standard raft, committed leader id is just `term`.
    fn committed_leader_id(&self) -> Option<&CommittedLeaderIdOf<C>>;

    /// Converts this `Option<T: RaftLogId>` into a reference-based log ID.
    ///
    /// Returns `Some(RefLogId)` if `self` is `Some(T)`, `None` otherwise.
    fn to_ref(&self) -> Option<RefLogId<'_, C>>;
}

impl<C, T> OptionRaftLogIdExt<C> for Option<T>
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
{
    fn index(&self) -> Option<u64> {
        self.as_ref().map(|x| x.index())
    }

    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(log_id) => log_id.index() + 1,
        }
    }

    fn committed_leader_id(&self) -> Option<&CommittedLeaderIdOf<C>> {
        self.as_ref().map(|x| x.committed_leader_id())
    }

    fn to_ref(&self) -> Option<RefLogId<'_, C>> {
        self.as_ref().map(|x| x.to_ref())
    }
}
