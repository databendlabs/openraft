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

use crate::CommittedLeaderId;
use crate::MessageSummary;
use crate::NodeId;

/// The identity of a raft log.
///
/// The log id serves as unique identifier for a log entry across the system. It is composed of two
/// parts: a leader id, which refers to the leader that proposed this log, and an integer index.
#[derive(Debug, Default, Copy, Clone, PartialOrd, Ord, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LogId<NID: NodeId> {
    /// The id of the leader that proposed this log
    pub leader_id: CommittedLeaderId<NID>,
    /// The index of a log in the storage.
    ///
    /// Log index is a consecutive integer.
    pub index: u64,
}

impl<NID: NodeId> RaftLogId<NID> for LogId<NID> {
    fn get_log_id(&self) -> &LogId<NID> {
        self
    }

    fn set_log_id(&mut self, log_id: &LogId<NID>) {
        *self = *log_id
    }
}

impl<NID: NodeId> Display for LogId<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.leader_id, self.index)
    }
}

impl<NID: NodeId> MessageSummary<LogId<NID>> for LogId<NID> {
    fn summary(&self) -> String {
        format!("{}", self)
    }
}

impl<NID: NodeId> LogId<NID> {
    /// Creates a log id proposed by a committed leader with `leader_id` at the given index.
    pub fn new(leader_id: CommittedLeaderId<NID>, index: u64) -> Self {
        if leader_id.term == 0 || index == 0 {
            assert_eq!(
                leader_id,
                CommittedLeaderId::default(),
                "zero-th log entry must be (0,0,0), but {} {}",
                leader_id,
                index
            );
            assert_eq!(
                index, 0,
                "zero-th log entry must be (0,0,0), but {} {}",
                leader_id, index
            );
        }
        LogId { leader_id, index }
    }

    /// Returns the leader id that proposed this log.
    pub fn committed_leader_id(&self) -> &CommittedLeaderId<NID> {
        &self.leader_id
    }
}
