
use std::{
    collections::BTreeMap,
    fmt,
};

use actix::prelude::*;
use futures::sync::{mpsc, oneshot};

use crate::{
    AppData, AppDataResponse, AppError, NodeId,
    common::{ClientPayloadWithIndex, ClientPayloadWithChan},
    messages::{MembershipConfig},
    network::RaftNetwork,
    replication::{ReplicationStream},
    storage::{InstallSnapshotChunk, RaftStorage},
};


/// The state of the Raft node.
pub(crate) enum RaftState<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> {
    /// A non-standard Raft state indicating that the node is initializing.
    Initializing,
    /// The node is completely passive; replicating entries, but not voting or timing out.
    ///
    /// When a node is a `NonVoter`, its behavior is the same as a follower with the added
    /// restrictions that it does not have an election timeout, it does not count when
    /// calculating replication majority, and it is not solicited for votes.
    ///
    /// A node will only be in this state when it first comes online without any existing config
    /// on disk, or when it has been removed from an existing cluster.
    ///
    /// In such a state, the parent application may have this new node added to an already running
    /// cluster, may have the node start as the leader of a new standalone cluster, may have the
    /// node initialize with a specific config, or may bring the node offline for the case where
    /// the node was removed from an existing cluster.
    NonVoter,
    /// The node is actively replicating logs from the leader.
    ///
    /// The node is passive when it is in this state. It issues no requests on its own but simply
    /// responds to requests from leaders and candidates.
    Follower(FollowerState),
    /// The node has detected an election timeout so is requesting votes to become leader.
    ///
    /// This state wraps struct which tracks outstanding requests to peers for requesting votes
    /// along with the number of votes granted.
    Candidate(CandidateState),
    /// The node is actively functioning as the Raft cluster leader.
    ///
    /// The leader handles all client requests. If a client contacts a follower, the follower must
    /// redirects it to the leader.
    Leader(LeaderState<D, R, E, N, S>),
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> RaftState<D, R, E, N, S> {
    /// Check if currently in follower state.
    pub fn is_follower(&self) -> bool {
        match self {
            RaftState::Follower(_) => true,
            _ => false,
        }
    }

    /// Check if currently in leader state.
    #[allow(dead_code)]
    pub fn is_leader(&self) -> bool {
        match self {
            RaftState::Leader(_) => true,
            _ => false,
        }
    }

    /// Check if currently in non-voter state.
    pub fn is_non_voter(&self) -> bool {
        match self {
            RaftState::NonVoter => true,
            _ => false,
        }
    }
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> fmt::Display for RaftState<D, R, E, N, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match self {
            RaftState::Initializing => "Initializing",
            RaftState::NonVoter => "NonVoter",
            RaftState::Follower(_) => "Follower",
            RaftState::Candidate(_) => "Candidate",
            RaftState::Leader(_) => "Leader",
        };
        write!(f, "{}", state)
    }
}

/// Volatile state specific to the Raft leader.
///
/// This state is reinitialized after an election.
pub(crate) struct LeaderState<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> {
    /// A mapping of node IDs the replication state of the target node.
    pub nodes: BTreeMap<NodeId, ReplicationState<D, R, E, N, S>>,
    /// A queue of client requests to be processed.
    pub client_request_queue: mpsc::UnboundedSender<ClientPayloadWithChan<D, R, E>>,
    /// A buffer of client requests which have been appended locally and are awaiting to be committed to the cluster.
    pub awaiting_committed: Vec<ClientPayloadWithIndex<D, R, E>>,
    /// A field tracking the cluster's current consensus state, which is used for dynamic membership.
    pub consensus_state: ConsensusState,
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> LeaderState<D, R, E, N, S> {
    /// Create a new instance.
    pub fn new(tx: mpsc::UnboundedSender<ClientPayloadWithChan<D, R, E>>, membership: &MembershipConfig) -> Self {
        let consensus_state = if membership.is_in_joint_consensus {
            ConsensusState::Joint{
                new_nodes: membership.non_voters.clone(),
                is_committed: false,
            }
        } else {
            ConsensusState::Uniform
        };
        Self{nodes: Default::default(), client_request_queue: tx, awaiting_committed: vec![], consensus_state}
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
pub(crate) struct ReplicationState<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> {
    pub match_index: u64,
    pub is_at_line_rate: bool,
    pub remove_after_commit: Option<u64>,
    pub addr: Addr<ReplicationStream<D, R, E, N, S>>,
}

pub(crate) enum ConsensusState {
    /// The cluster consensus is uniform; not in a joint consensus state.
    Uniform,
    /// The cluster is in a joint consensus state and is syncing new nodes.
    Joint {
        /// The new nodes which are being synced.
        new_nodes: Vec<NodeId>,
        /// A bool indicating if the associated config has been comitted yet.
        ///
        /// NOTE: when a new leader is elected, it will initialize this value to false, and then
        /// update this value to true once the new leader's blank payload has been committed.
        is_committed: bool,
    }
}

/// Volatile state specific to a Raft node in candidate state.
pub(crate) struct CandidateState {
    /// Current outstanding requests to peer nodes by node ID.
    pub(crate) requests: BTreeMap<NodeId, SpawnHandle>,
    /// The number of votes which have been granted by peer nodes.
    pub(crate) votes_granted: u64,
    /// The number of votes needed in order to become the Raft leader.
    pub(crate) votes_needed: u64,
}

impl CandidateState {
    /// Cleanup state resources.
    pub(crate) fn cleanup(&mut self) -> Vec<SpawnHandle> {
        let keys = self.requests.keys().map(|k| *k).collect::<Vec<_>>();
        let mut handles = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(f) = self.requests.remove(&key) {
                handles.push(f);
            }
        }
        handles
    }
}

/// Volatile state specific to a Raft node in follower state.
pub(crate) struct FollowerState {
    pub snapshot_state: SnapshotState,
}

impl Default for FollowerState {
    fn default() -> Self {
        Self{snapshot_state: SnapshotState::Idle}
    }
}

/// The current snapshot state of the Raft node.
pub(crate) enum SnapshotState {
    /// No snapshot operations are taking place.
    Idle,
    /// The Raft node is streaming in a snapshot from the leader.
    Streaming(Option<mpsc::UnboundedSender<InstallSnapshotChunk>>, Option<oneshot::Receiver<()>>),
}
