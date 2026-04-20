//! Invariant-violation enum and its [`Display`] impl.
//!
//! Each variant documents the safety property it defends, with a cite to the
//! authoritative source (Ongaro's paper/dissertation or Vanlightly's TLA+ spec).
//! The formatting is intentionally terse so violations render clearly in a
//! multi-line crash report.

use super::CLeaderId;
use crate::typ::NodeId;

/// A single safety-property violation.
#[derive(Debug, Clone)]
pub enum InvariantViolation {
    /// Election Safety (Paper §3.6.3):
    /// two distinct nodes report themselves leader with the **same committed leader id**.
    ///
    /// - In standard Raft (`leader_id_std`) `CLeaderId == term`, so this reduces to the classic "at
    ///   most one leader per term".
    /// - In `leader_id_adv` mode, `CLeaderId == (term, node_id)`, so different node_ids leading in
    ///   the same term are *not* violations — only identical `(term, node_id)` on two different
    ///   nodes is.
    DuplicateLeader { leader_id: CLeaderId, nodes: Vec<NodeId> },

    /// Log Matching (Paper §3.5, Vanlightly `NoLogDivergence`):
    /// two nodes have committed entries at the same index with different committed leader ids.
    LogDivergence {
        index: u64,
        node_a: NodeId,
        lid_a: CLeaderId,
        node_b: NodeId,
        lid_b: CLeaderId,
    },

    /// Leader Completeness (Paper §3.6.3, Vanlightly `LeaderHasAllAckedValues`):
    /// a node that was elected in a later term is missing an entry committed in an earlier term.
    LeaderMissingCommitted {
        leader: NodeId,
        leader_id: CLeaderId,
        missing_index: u64,
        committed_by: CLeaderId,
    },

    /// State Machine Safety (Paper §3.6.3):
    /// two nodes applied the same index but have divergent state machine state.
    StateMachineDivergence { index: u64, nodes: Vec<NodeId> },

    /// Committed On Quorum (Vanlightly `CommittedEntriesReachMajority`):
    /// a leader's recently-committed entry is not replicated on a voter quorum
    /// in its own current membership.
    CommittedNotOnQuorum {
        leader: NodeId,
        index: u64,
        voters: Vec<NodeId>,
        matching: Vec<NodeId>,
    },

    /// Committed Immutable (history-based derivation):
    /// an index previously reported as committed now shows a different committed leader id.
    /// Logically follows from Log Matching; an explicit check catches bugs that
    /// only manifest across ticks (e.g., an entry getting overwritten and re-committed).
    CommittedChanged {
        index: u64,
        original: CLeaderId,
        original_observer: NodeId,
        new: CLeaderId,
        new_observer: NodeId,
    },

    /// State Ordering (implementation sanity):
    /// a node's own log pointers violate `purged ≤ snapshot ≤ applied ≤ committed ≤ last_log`.
    StateOrdering {
        node: NodeId,
        lower: &'static str,
        lower_index: u64,
        higher: &'static str,
        higher_index: u64,
    },

    /// Monotonic Term (Vanlightly `MonotonicTerm`):
    /// `current_term` must never decrease on a node across ticks.
    TermRegressed { node: NodeId, previous: u64, current: u64 },

    /// Monotonic Commit Index (Vanlightly `MonotonicCommitIndex`):
    /// the `committed` index must never decrease on a node across ticks.
    CommitIndexRegressed { node: NodeId, previous: u64, current: u64 },

    /// Monotonic Applied Index (derived — applied is implied monotonic in Raft):
    /// `last_applied.index` must never decrease on a node across ticks.
    AppliedIndexRegressed { node: NodeId, previous: u64, current: u64 },

    /// Monotonic Vote (Vanlightly `MonotonicVote`):
    /// a node's persisted vote (ordered as `(term, leader_id, committed)`)
    /// must never regress.
    VoteRegressed {
        node: NodeId,
        previous: String,
        current: String,
    },
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateLeader { leader_id, nodes } => {
                write!(
                    f,
                    "ElectionSafety: leader_id={leader_id} claimed by multiple nodes {nodes:?}"
                )
            }
            Self::LogDivergence {
                index,
                node_a,
                lid_a,
                node_b,
                lid_b,
            } => write!(
                f,
                "LogMatching: index={index} differs: n{node_a}@{lid_a} vs n{node_b}@{lid_b}"
            ),
            Self::LeaderMissingCommitted {
                leader,
                leader_id,
                missing_index,
                committed_by,
            } => write!(
                f,
                "LeaderCompleteness: leader n{leader}@{leader_id} is missing \
                 committed index {missing_index} (committed by {committed_by})"
            ),
            Self::StateMachineDivergence { index, nodes } => {
                write!(
                    f,
                    "StateMachineSafety: divergent state at applied index {index} across {nodes:?}"
                )
            }
            Self::CommittedNotOnQuorum {
                leader,
                index,
                voters,
                matching,
            } => write!(
                f,
                "CommittedOnQuorum: leader n{leader} committed index {index}; \
                 only {matching:?} of voters {voters:?} have matching entry"
            ),
            Self::CommittedChanged {
                index,
                original,
                original_observer,
                new,
                new_observer,
            } => write!(
                f,
                "CommittedImmutable: index {index} was {original} on n{original_observer}, \
                 now {new} on n{new_observer}"
            ),
            Self::StateOrdering {
                node,
                lower,
                lower_index,
                higher,
                higher_index,
            } => write!(
                f,
                "StateOrdering on n{node}: {lower}={lower_index} > {higher}={higher_index}"
            ),
            Self::TermRegressed {
                node,
                previous,
                current,
            } => {
                write!(f, "MonotonicTerm: n{node} term {previous} -> {current}")
            }
            Self::CommitIndexRegressed {
                node,
                previous,
                current,
            } => {
                write!(f, "MonotonicCommitIndex: n{node} committed {previous} -> {current}")
            }
            Self::AppliedIndexRegressed {
                node,
                previous,
                current,
            } => {
                write!(f, "MonotonicAppliedIndex: n{node} applied {previous} -> {current}")
            }
            Self::VoteRegressed {
                node,
                previous,
                current,
            } => {
                write!(f, "MonotonicVote: n{node} vote {previous} -> {current}")
            }
        }
    }
}
