//! This mod defines the identity of a raft log and provides supporting utilities to work with log
//! id related types.

mod log_id_option_ext;
mod log_index_option_ext;
mod raft_log_id;

use std::fmt::Display;
use std::fmt::Formatter;

pub use log_id_option_ext::LogIdOptionExt;
pub use log_index_option_ext::LogIndexOptionExt;
pub use raft_log_id::RaftLogId;

use crate::type_config::alias::CommittedLeaderIdOf;
use crate::RaftTypeConfig;

/// The identity of a raft log.
///
/// The log id serves as unique identifier for a log entry across the system. It is composed of two
/// parts: a leader id, which refers to the leader that proposed this log, and an integer index.
#[derive(Debug, Default, Clone, PartialOrd, Ord, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LogId<C>
where C: RaftTypeConfig
{
    /// The id of the leader that proposed this log
    pub leader_id: CommittedLeaderIdOf<C>,
    /// The index of a log in the storage.
    ///
    /// Log index is a consecutive integer.
    pub index: u64,
}

impl<C> Copy for LogId<C>
where
    C: RaftTypeConfig,
    CommittedLeaderIdOf<C>: Copy,
{
}

impl<C> RaftLogId<C> for LogId<C>
where C: RaftTypeConfig
{
    fn get_log_id(&self) -> &LogId<C> {
        self
    }

    fn set_log_id(&mut self, log_id: &LogId<C>) {
        *self = log_id.clone()
    }
}

impl<C> Display for LogId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.leader_id, self.index)
    }
}

impl<C> AsRef<LogId<C>> for LogId<C>
where C: RaftTypeConfig
{
    fn as_ref(&self) -> &LogId<C> {
        self
    }
}

impl<C> AsMut<LogId<C>> for LogId<C>
where C: RaftTypeConfig
{
    fn as_mut(&mut self) -> &mut LogId<C> {
        self
    }
}

impl<C> LogId<C>
where C: RaftTypeConfig
{
    /// Creates a log id proposed by a committed leader with `leader_id` at the given index.
    pub fn new(leader_id: CommittedLeaderIdOf<C>, index: u64) -> Self {
        LogId { leader_id, index }
    }

    /// Returns the leader id that proposed this log.
    pub fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        &self.leader_id
    }
}
