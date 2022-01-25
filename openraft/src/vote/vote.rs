use std::cmp::Ordering;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::NodeId;

#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommittedState {
    Uncommitted,
    Committed,
}

impl Default for CommittedState {
    fn default() -> Self {
        CommittedState::Uncommitted
    }
}

/// `Vote` represent the privilege of a node.
/// A `Vote` is basically a tuple of `(term, committed|uncommitted, node_id)`.
///
/// A uncommitted `Vote` is built to represent a Candidate when a node enters Candidate state.
/// A committed `Vote`(that is granted by a quorum) represents a leader.
///
/// A node can grant at most one `Vote`, by persistently storing a `Vote` in its storage.
///
/// Once a node granted a `Vote`, the next `Vote` can be granted only when some conditions are satisfied.
/// These conditions reflect the raft spec and it is a **partially ordered** relation between `Vote`s.
/// That's why we just impl a `PartialOrd` for `Vote`, that covers all rules defined in raft.
///
/// To use it, a node with `vote_1` just check whether `vote_1 <= vote_2` is hold to determine if it can grant
/// `vote_2`.
///
/// A `Vote` is uncommitted when it is firstly created.
/// A `Vote` is committed when a quorum granted it(i.e., the node with this node becomes a `Leader`).
///
/// E.g.:
///
/// - A node with `(term=1,uncommitted,None)` is able to grant `(term=2, *,*)`, `(term=1,*,Some(*))`;
///
/// - A node with `(term=1,uncommitted,Some(3))` is able to grant `(term=2,*,*)`, `(term=1,committed,Some(4))`. But it
///   can not grant `(term=1,uncommitted,Some(4))`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    pub term: u64,

    /// Whether this vote is committed, e.g., a vote is granted by a quorum.
    pub state: CommittedState,

    /// The id and the state of last voted node.
    pub voted_for: Option<NodeId>,
}

impl PartialOrd for Vote {
    /// The relation between votes are defined by a partially ordered relation.
    ///
    /// See [`Vote`].
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let term_ord = self.term.cmp(&other.term);
        if term_ord != Ordering::Equal {
            // Higher term overrides lower term.
            return Some(term_ord);
        }

        // Same term

        let st_ord = self.state.cmp(&other.state);
        if st_ord != Ordering::Equal {
            // Committed override non-committed.
            return Some(st_ord);
        }

        // Same term,state

        if self.state == CommittedState::Committed {
            assert_eq!(
                self.voted_for, other.voted_for,
                "Two committed votes have to be the same"
            );

            return Some(Ordering::Equal);
        }

        // Same term, both NonCommitted

        if self.voted_for.is_some() && other.voted_for.is_some() {
            if self.voted_for == other.voted_for {
                Some(Ordering::Equal)
            } else {
                // Two different Some() are incomparable
                None
            }
        } else {
            // At least one of them is None, always comparable
            Some(self.voted_for.cmp(&other.voted_for))
        }
    }
}

impl std::fmt::Display for Vote {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "vote:{}-{:?}-{:?}", self.term, self.state, self.voted_for)
    }
}

impl Vote {
    pub fn new_uncommitted(term: u64, node_id: Option<u64>) -> Self {
        Self {
            term,
            state: CommittedState::Uncommitted,
            voted_for: node_id,
        }
    }

    pub fn new_committed(term: u64, node_id: u64) -> Self {
        Self {
            term,
            state: CommittedState::Committed,
            voted_for: Some(node_id),
        }
    }

    // Mark this vote as committed
    pub fn commit(&mut self) {
        self.state = CommittedState::Committed;
    }

    /// Get the currently established Leader.
    ///
    /// If the vote is committed, the the `voted_for` may be a `Leader`, returns the leader id.
    /// Otherwise, there is not an established leader.
    pub fn leader(&self) -> Option<NodeId> {
        if self.state == CommittedState::Uncommitted {
            None
        } else {
            self.voted_for
        }
    }
}
