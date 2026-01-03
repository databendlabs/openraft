//! Log identification and comparison.
//!
//! This module defines [`LogId`], the unique identifier for Raft log entries, and utilities for
//! working with log IDs.
//!
//! ## Key Types
//!
//! - [`LogId`] - Unique identifier for a log entry `(leader_id, index)`
//! - [`LogIdOptionExt`] - Extension trait for `Option<LogId>` comparisons
//! - [`LogIndexOptionExt`] - Extension trait for `Option<u64>` index comparisons
//!
//! ## Overview
//!
//! Each log entry is uniquely identified by a [`LogId`] containing:
//! - **Leader ID**: `(term, node_id)` of the leader that proposed the log
//! - **Index**: Consecutive integer position in the log
//!
//! ## Log ID Ordering
//!
//! Log IDs are ordered by:
//! 1. Leader ID (term, then node_id)
//! 2. Index
//!
//! This ordering ensures that logs from higher terms always supersede logs from lower terms,
//! which is fundamental to Raft's consistency guarantees.

mod log_id_option_ext;
mod log_index_option_ext;
pub(crate) mod option_raft_log_id_ext;
pub(crate) mod option_ref_log_id_ext;
pub(crate) mod raft_log_id;
pub(crate) mod raft_log_id_ext;
pub(crate) mod ref_log_id;
mod std_log_id;

use std::fmt::Display;
use std::fmt::Formatter;

pub use log_id_option_ext::LogIdOptionExt;
pub use log_index_option_ext::LogIndexOptionExt;

pub use self::raft_log_id::RaftLogId;
use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;

/// The identity of a raft log.
///
/// The log id serves as a unique identifier for a log entry across the system. It is composed of
/// two parts: a leader id, which refers to the leader that proposed this log, and an integer index.
#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
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

    /// Get the established(committed) leader ID of this log entry.
    #[deprecated(since = "0.10.0", note = "Use `committed_leader_id` instead.")]
    pub fn leader_id(&self) -> &CommittedLeaderIdOf<C> {
        &self.leader_id
    }

    /// Get the log index.
    pub fn index(&self) -> u64 {
        self.index
    }
}

/// Methods available only when using `leader_id_std::LeaderId`.
impl<C> LogId<C>
where C: RaftTypeConfig<LeaderId = crate::vote::leader_id_std::LeaderId<C>>
{
    /// Creates a log id from a term and index.
    ///
    /// This is a convenience method for standard Raft where `CommittedLeaderId` is
    /// just a wrapper around the term.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Equivalent to: LogId::new(CommittedLeaderId::new(5), 100)
    /// let log_id = LogId::new_term_index(5, 100);
    /// ```
    pub fn new_term_index(term: C::Term, index: u64) -> Self {
        LogId {
            leader_id: crate::vote::leader_id_std::CommittedLeaderId::new(term),
            index,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::declare_raft_types;
    use crate::log_id::raft_log_id::RaftLogId;
    use crate::vote::leader_id_std::CommittedLeaderId;

    declare_raft_types!(pub TestConfig: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=u64);

    #[test]
    fn test_new_term_index() {
        let log_id = super::LogId::<TestConfig>::new_term_index(5, 100);
        assert_eq!(100, log_id.index());
        assert_eq!(5u64, **log_id.committed_leader_id());
    }

    #[test]
    fn test_new_term_index_equivalence() {
        let log_id1 = super::LogId::<TestConfig>::new_term_index(5, 100);
        let log_id2 = super::LogId::<TestConfig>::new(CommittedLeaderId::<TestConfig>::new(5), 100);
        assert_eq!(log_id1.index(), log_id2.index());
        assert_eq!(**log_id1.committed_leader_id(), **log_id2.committed_leader_id());
    }

    #[test]
    fn test_to_type_log_id_to_tuple() {
        let log_id = super::LogId::<TestConfig>::new_term_index(5, 100);
        let tuple: (u64, u64) = log_id.to_type();
        assert_eq!((5, 100), tuple);
    }

    #[test]
    fn test_to_type_tuple_to_log_id() {
        let tuple: (u64, u64) = (5, 100);
        let log_id: super::LogId<TestConfig> = tuple.to_type();
        assert_eq!(100, log_id.index());
        assert_eq!(5, **log_id.committed_leader_id());
    }

    #[test]
    fn test_log_id_parts() {
        let log_id = super::LogId::<TestConfig>::new_term_index(5, 100);
        let (leader_id, index) = log_id.log_id_parts();
        assert_eq!(5, **leader_id);
        assert_eq!(100, index);
    }
}
