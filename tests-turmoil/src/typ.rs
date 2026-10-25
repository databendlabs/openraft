//! Type configuration for turmoil tests.

#[cfg(test)]
use std::collections::BTreeSet;
use std::io::Cursor;

use serde::Deserialize;
use serde::Serialize;

pub type NodeId = u64;

/// Node information including network address.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Node {
    pub addr: String,
}

impl std::fmt::Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.addr)
    }
}

/// Client request - a simple key-value write operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub client_id: String,
    pub serial: u64,
    pub key: String,
    pub value: String,
}

impl std::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Request{{client:{}, serial:{}, key:{}, value:{}}}",
            self.client_id, self.serial, self.key, self.value
        )
    }
}

/// Client response.
///
/// `prev` is the key's previous versioned value, read by the apply at state
/// `log_id - 1`: an exact, linearizable observation of the key immediately
/// below the write's own log position, which the oracle uses to verify the
/// per-key predecessor chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub prev: Option<crate::store::ValueMeta>,
}

/// The membership type that the cluster runs.
#[cfg(not(feature = "threshold-membership"))]
pub type Membership = openraft::Membership<NodeId, Node>;

/// The membership type that the cluster runs.
#[cfg(feature = "threshold-membership")]
pub type Membership = openraft::membership::ThresholdMembership<NodeId, Node>;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Node = Node,
        Membership = Membership,
        AsyncRuntime = openraft_rt::deterministic_rng::DeterministicRng<openraft_rt_tokio::TokioRuntime>,
);

pub type Raft = openraft::Raft<TypeConfig, std::sync::Arc<crate::store::StateMachine>>;
pub type Vote = openraft::type_config::alias::VoteOf<TypeConfig>;
pub type LogId = openraft::type_config::alias::LogIdOf<TypeConfig>;
pub type Entry = openraft::type_config::alias::EntryOf<TypeConfig>;
pub type SnapshotMeta = openraft::type_config::alias::SnapshotMetaOf<TypeConfig>;
pub type SnapshotData = Cursor<Vec<u8>>;
pub type Snapshot = openraft::type_config::alias::SnapshotOf<TypeConfig, SnapshotData>;
pub type StoredMembership = openraft::type_config::alias::StoredMembershipOf<TypeConfig>;

pub type AppendEntriesRequest = openraft::raft::AppendEntriesRequest<TypeConfig>;
pub type AppendEntriesResponse = openraft::raft::AppendEntriesResponse<TypeConfig>;
pub type VoteRequest = openraft::raft::VoteRequest<TypeConfig>;
pub type VoteResponse = openraft::raft::VoteResponse<TypeConfig>;
pub type SnapshotResponse = openraft::raft::SnapshotResponse<TypeConfig>;

pub type RaftMetrics = openraft::RaftMetrics<TypeConfig>;

/// Builds a membership with one config per voter set, each committing through a majority of its
/// voters, and with `learners` as learners.
#[cfg(all(test, not(feature = "threshold-membership")))]
pub(crate) fn majority_membership(voter_sets: Vec<BTreeSet<NodeId>>, learners: Vec<NodeId>) -> Membership {
    Membership::new_with_defaults(voter_sets, learners)
}

/// Builds a membership with one config per voter set, each committing through a majority of its
/// voters, and with `learners` as learners.
#[cfg(all(test, feature = "threshold-membership"))]
pub(crate) fn majority_membership(voter_sets: Vec<BTreeSet<NodeId>>, learners: Vec<NodeId>) -> Membership {
    use std::collections::BTreeMap;
    use std::num::NonZeroU64;
    use std::sync::Arc;

    use openraft::membership::Joint;
    use openraft::membership::NodeRole;
    use openraft::membership::RaftMembership;
    use openraft::membership::ThresholdQuorumSet;

    let voter_role = NodeRole::Campaigner | NodeRole::ReceiveLogs;
    let mut roles = BTreeMap::new();
    let mut read_rules = Vec::new();
    let mut write_rules = Vec::new();
    for voters in voter_sets {
        for id in &voters {
            roles.insert(*id, voter_role);
        }
        let majority = NonZeroU64::MIN.saturating_add(voters.len() as u64 / 2);
        let voters = Arc::new(voters);
        read_rules.push(ThresholdQuorumSet::new(majority, voters.clone()));
        write_rules.push(ThresholdQuorumSet::new(majority, voters));
    }
    for id in learners {
        roles.entry(id).or_insert(NodeRole::ReceiveLogs);
    }

    let nodes = roles.into_iter().map(|(id, role)| (id, Node::default(), role));
    RaftMembership::new(Joint::new(read_rules), Joint::new(write_rules), nodes)
}
