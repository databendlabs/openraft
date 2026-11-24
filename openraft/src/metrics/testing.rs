//! Test fixtures for metrics of a custom vote type.

use std::fmt;

use crate::impls::leader_id_std::LeaderId;
use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;

crate::declare_raft_types!(
    /// A type config with standard Raft leader IDs and [`MetaVote`] as its vote.
    pub(crate) MetaVoteConfig:
        LeaderId = LeaderId<u64, u64>,
        Vote = MetaVote,
        AsyncRuntime = openraft_rt_tokio::TokioRuntime,
);

/// A custom vote whose equality also compares `meta`, a field the Raft protocol ignores, such as
/// storage metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct MetaVote {
    leader_id: LeaderId<u64, u64>,
    committed: bool,
    meta: u64,
}

impl MetaVote {
    /// Builds a vote for the leader ID `(term, node_id)` that carries `meta`.
    pub(crate) fn new(term: u64, node_id: u64, committed: bool, meta: u64) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed,
            meta,
        }
    }
}

impl fmt::Display for MetaVote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let committed = if self.committed { "Q" } else { "-" };
        write!(f, "<{}:{}, meta={}>", self.leader_id, committed, self.meta)
    }
}

impl RaftVote for MetaVote {
    type LeaderId = LeaderId<u64, u64>;

    fn from_leader_id(leader_id: Self::LeaderId, committed: bool) -> Self {
        Self {
            leader_id,
            committed,
            meta: 0,
        }
    }

    fn leader_id(&self) -> &Self::LeaderId {
        &self.leader_id
    }

    fn is_committed(&self) -> bool {
        self.committed
    }
}
