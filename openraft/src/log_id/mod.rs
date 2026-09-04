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
use openraft_macros::since;

pub use self::raft_log_id::RaftLogId;
use crate::vote::RaftCommittedLeaderId;
use crate::vote::RaftTerm;
use crate::vote::leader_id_std;

/// The identity of a raft log.
///
/// The log id serves as a unique identifier for a log entry across the system. It is composed of
/// two parts: a leader id, which refers to the leader that proposed this log, and an integer index.
#[since(
    version = "0.10.0",
    change = "from `LogId<C: RaftTypeConfig>` to `LogId<CLID: RaftCommittedLeaderId>`"
)]
#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LogId<CLID>
where CLID: RaftCommittedLeaderId
{
    /// The id of the leader that proposed this log
    pub leader_id: CLID,
    /// The index of a log in the storage.
    ///
    /// Log index is a consecutive integer.
    pub index: u64,
}

impl<CLID> Copy for LogId<CLID> where CLID: RaftCommittedLeaderId + Copy {}

impl<CLID> RaftLogId for LogId<CLID>
where CLID: RaftCommittedLeaderId
{
    type CommittedLeaderId = CLID;

    fn new(leader_id: CLID, index: u64) -> Self {
        LogId { leader_id, index }
    }

    fn committed_leader_id(&self) -> &CLID {
        &self.leader_id
    }

    fn index(&self) -> u64 {
        self.index
    }
}

impl<CLID> Display for LogId<CLID>
where CLID: RaftCommittedLeaderId
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.committed_leader_id(), self.index())
    }
}

/// Inherent counterparts of the [`RaftLogId`] methods, so callers do not have to import the trait.
///
/// Each one delegates rather than reimplementing, so the trait impl stays the only definition. An
/// inherent method shadows the trait method at every call site, so a second copy here would let the
/// two drift apart without any call site changing.
impl<CLID> LogId<CLID>
where CLID: RaftCommittedLeaderId
{
    /// Creates a log id proposed by a committed leader with `leader_id` at the given index.
    pub fn new(leader_id: CLID, index: u64) -> Self {
        RaftLogId::new(leader_id, index)
    }

    /// Returns the leader id that proposed this log.
    pub fn committed_leader_id(&self) -> &CLID {
        RaftLogId::committed_leader_id(self)
    }

    /// Get the established(committed) leader ID of this log entry.
    #[deprecated(since = "0.10.0", note = "Use `committed_leader_id` instead.")]
    pub fn leader_id(&self) -> &CLID {
        RaftLogId::committed_leader_id(self)
    }

    /// Get the log index.
    pub fn index(&self) -> u64 {
        RaftLogId::index(self)
    }
}

/// Methods available only when using `leader_id_std::LeaderId`.
///
/// `Term` and `NID` are extracted as separate type parameters to avoid a rustc cycle error
/// that occurs when using `C::Term` or `C::NodeId` inside an associated type equality constraint
/// (e.g., `LeaderId = LeaderId<C::Term, C::NodeId>`).
impl<Term> LogId<leader_id_std::CommittedLeaderId<Term>>
where Term: RaftTerm
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
    pub fn new_term_index(term: Term, index: u64) -> Self {
        LogId {
            leader_id: leader_id_std::CommittedLeaderId::new(term),
            index,
        }
    }
}

#[cfg(test)]
mod tests {
    use hegel::generators;
    use hegel::generators::Generator;
    use hegel::generators::PrintableGenerator;

    use crate::LogIdOptionExt;
    use crate::LogIndexOptionExt;
    use crate::log_id::raft_log_id::RaftLogId;
    use crate::vote::leader_id_adv;
    use crate::vote::leader_id_std;

    type TestCLID = leader_id_std::CommittedLeaderId<u64>;
    type AdvCLID = leader_id_adv::CommittedLeaderId<u64, u64>;

    #[test]
    fn test_new_term_index() {
        let log_id = super::LogId::<TestCLID>::new_term_index(5, 100);
        assert_eq!(100, log_id.index());
        assert_eq!(5u64, **log_id.committed_leader_id());
    }

    #[test]
    fn test_new_term_index_equivalence() {
        let log_id1 = super::LogId::<TestCLID>::new_term_index(5, 100);
        let log_id2 = super::LogId::<TestCLID>::new(TestCLID::new(5), 100);
        assert_eq!(log_id1.index(), log_id2.index());
        assert_eq!(**log_id1.committed_leader_id(), **log_id2.committed_leader_id());
    }

    #[test]
    fn test_to_type_log_id_to_tuple() {
        let log_id = super::LogId::<TestCLID>::new_term_index(5, 100);
        let tuple: (u64, u64) = log_id.to_type();
        assert_eq!((5, 100), tuple);
    }

    #[test]
    fn test_to_type_tuple_to_log_id() {
        let tuple: (u64, u64) = (5, 100);
        let log_id: super::LogId<TestCLID> = tuple.to_type();
        assert_eq!(100, log_id.index());
        assert_eq!(5, **log_id.committed_leader_id());
    }

    #[test]
    fn test_log_id_parts() {
        let log_id = super::LogId::<TestCLID>::new_term_index(5, 100);
        let (leader_id, index) = log_id.log_id_parts();
        assert_eq!(5, **leader_id);
        assert_eq!(100, index);
    }

    // ---
    // Property-based tests (hegel).
    // ---

    /// Terms, node ids and indexes are mostly drawn from a three-value pool so that ties on each
    /// component are common, and full-range draws cover the `u64` boundaries.
    #[hegel::composite]
    fn small_or_any_u64(tc: &hegel::TestCase) -> u64 {
        tc.draw(hegel::one_of!(
            generators::integers::<u64>().max_value(2),
            generators::integers::<u64>(),
        ))
    }

    fn std_log_ids() -> impl PrintableGenerator<super::LogId<TestCLID>> {
        hegel::compose!(|tc| {
            let term = tc.draw(small_or_any_u64());
            let index = tc.draw(small_or_any_u64());
            super::LogId::<TestCLID>::new_term_index(term, index)
        })
        .print_as_debug()
    }

    fn adv_log_ids() -> impl PrintableGenerator<super::LogId<AdvCLID>> {
        hegel::compose!(|tc| {
            let term = tc.draw(small_or_any_u64());
            let node_id = tc.draw(small_or_any_u64());
            let index = tc.draw(small_or_any_u64());
            super::LogId::<AdvCLID>::new(leader_id_adv::LeaderId { term, node_id }, index)
        })
        .print_as_debug()
    }

    /// The module docs promise log ids are ordered by leader id (term, then node id) and then by
    /// index. The order comes from a derive, so a field reorder would silently change it.
    #[hegel::test]
    fn test_log_id_order_matches_the_leader_id_then_index_oracle(tc: hegel::TestCase) {
        let adv_a = tc.draw(adv_log_ids());
        let adv_b = tc.draw(adv_log_ids());
        let adv_key = |log_id: &super::LogId<AdvCLID>| (log_id.leader_id.term, log_id.leader_id.node_id, log_id.index);
        assert_eq!(adv_key(&adv_a).cmp(&adv_key(&adv_b)), adv_a.cmp(&adv_b));

        // A committed leader id in standard Raft is just the term, so the node id is gone.
        let std_a = tc.draw(std_log_ids());
        let std_b = tc.draw(std_log_ids());
        let std_key = |log_id: &super::LogId<TestCLID>| (log_id.leader_id.term, log_id.index);
        assert_eq!(std_key(&std_a).cmp(&std_key(&std_b)), std_a.cmp(&std_b));
    }

    /// `to_type` converts between `RaftLogId` implementations, so a `LogId` sent through the tuple
    /// implementation and back must come out unchanged.
    #[hegel::test]
    fn test_to_type_roundtrips_a_log_id_through_the_tuple_impl(tc: hegel::TestCase) {
        let log_id = tc.draw(std_log_ids());

        let tuple: (u64, u64) = log_id.to_type();
        assert_eq!(log_id, tuple.to_type());
    }

    /// `LogIdOptionExt` and `LogIndexOptionExt` both document `next_index` as one past the log id,
    /// or 0 for `None`, so the two must agree.
    #[hegel::test]
    fn test_log_id_next_index_agrees_with_log_index_next_index(tc: hegel::TestCase) {
        let index = tc.draw(generators::optional(
            generators::integers::<u64>().max_value(u64::MAX - 1),
        ));
        let term = tc.draw(small_or_any_u64());
        let log_id = index.map(|index| super::LogId::<TestCLID>::new_term_index(term, index));

        assert_eq!(index.next_index(), log_id.next_index());
        assert_eq!(index, log_id.index());
    }

    /// A log id survives a round trip through a self-describing format and through a tagged one.
    /// It is part of every persisted log entry, and the two committed-leader-id flavors encode
    /// differently: standard Raft's is `serde(transparent)` over the term.
    #[cfg(feature = "serde")]
    #[hegel::test]
    fn test_log_id_serde_roundtrip(tc: hegel::TestCase) {
        let std_log_id = tc.draw(std_log_ids());
        let json = serde_json::to_string(&std_log_id).unwrap();
        assert_eq!(std_log_id, serde_json::from_str(&json).unwrap());
        let binary = bincode::serialize(&std_log_id).unwrap();
        assert_eq!(std_log_id, bincode::deserialize(&binary).unwrap());

        let adv_log_id = tc.draw(adv_log_ids());
        let json = serde_json::to_string(&adv_log_id).unwrap();
        assert_eq!(adv_log_id, serde_json::from_str(&json).unwrap());
        let binary = bincode::serialize(&adv_log_id).unwrap();
        assert_eq!(adv_log_id, bincode::deserialize(&binary).unwrap());
    }
}
