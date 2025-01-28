//! This mod defines the identity of a raft log and provides supporting utilities to work with log
//! id related types.

mod log_id_option_ext;
mod log_index_option_ext;
pub(crate) mod option_raft_log_id_ext;
pub(crate) mod option_ref_log_id_ext;
pub(crate) mod raft_log_id;
pub(crate) mod raft_log_id_ext;
pub(crate) mod ref_log_id;

use std::fmt::Display;
use std::fmt::Formatter;

pub use log_id_option_ext::LogIdOptionExt;
pub use log_index_option_ext::LogIndexOptionExt;

use crate::log_id::raft_log_id::RaftLogId;
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
    fn new(leader_id: CommittedLeaderIdOf<C>, index: u64) -> Self {
        LogId { leader_id, index }
    }

    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        &self.leader_id
    }

    fn index(&self) -> u64 {
        self.index
    }
}

impl<C> Display for LogId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.committed_leader_id(), self.index())
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

    #[deprecated(since = "0.10.0", note = "Use `committed_leader_id` instead.")]
    pub fn leader_id(&self) -> &CommittedLeaderIdOf<C> {
        &self.leader_id
    }

    pub fn index(&self) -> u64 {
        self.index
    }
}
