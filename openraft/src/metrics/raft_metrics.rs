use std::fmt;
use std::sync::Arc;

use crate::core::ServerState;
use crate::display_ext::display_option::DisplayOption;
use crate::display_ext::display_option::DisplayOptionExt;
use crate::error::Fatal;
use crate::metrics::ReplicationMetrics;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::StoredMembership;
use crate::Vote;

/// A set of metrics describing the current state of a Raft node.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct RaftMetrics<C: RaftTypeConfig> {
    pub running_state: Result<(), Fatal<C>>,

    /// The ID of the Raft node.
    pub id: C::NodeId,

    // ---
    // --- data ---
    // ---
    /// The current term of the Raft node.
    pub current_term: u64,

    /// The last accepted vote.
    pub vote: Vote<C::NodeId>,

    /// The last log index has been appended to this Raft node's log.
    pub last_log_index: Option<u64>,

    /// The last log index has been applied to this Raft node's state machine.
    pub last_applied: Option<LogId<C::NodeId>>,

    /// The id of the last log included in snapshot.
    /// If there is no snapshot, it is (0,0).
    pub snapshot: Option<LogId<C::NodeId>>,

    /// The last log id that has purged from storage, inclusive.
    ///
    /// `purged` is also the first log id Openraft knows, although the corresponding log entry has
    /// already been deleted.
    pub purged: Option<LogId<C::NodeId>>,

    // ---
    // --- cluster ---
    // ---
    /// The state of the Raft node.
    pub state: ServerState,

    /// The current cluster leader.
    pub current_leader: Option<C::NodeId>,

    /// For a leader, it is the elapsed time in milliseconds since the most recently acknowledged
    /// timestamp by a quorum.
    ///
    /// It is `None` if this node is not leader, or the leader is not yet acknowledged by a quorum.
    /// Being acknowledged means receiving a reply of
    /// `AppendEntries`(`AppendEntriesRequest.vote.committed == true`).
    /// Receiving a reply of `RequestVote`(`RequestVote.vote.committed == false`) does not count,
    /// because a node will not maintain a lease for a vote with `committed == false`.
    ///
    /// This duration is used by the application to assess the likelihood that the leader has lost
    /// synchronization with the cluster.
    /// A longer duration without acknowledgment may suggest a higher probability of the leader
    /// being partitioned from the cluster.
    pub millis_since_quorum_ack: Option<u64>,

    /// The current membership config of the cluster.
    pub membership_config: Arc<StoredMembership<C>>,

    // ---
    // --- replication ---
    // ---
    /// The replication states. It is Some() only when this node is leader.
    pub replication: Option<ReplicationMetrics<C::NodeId>>,
}

impl<C> fmt::Display for RaftMetrics<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Metrics{{")?;

        write!(
            f,
            "id:{}, {:?}, term:{}, vote:{}, last_log:{}, last_applied:{}, leader:{}(since_last_ack:{} ms)",
            self.id,
            self.state,
            self.current_term,
            self.vote,
            DisplayOption(&self.last_log_index),
            DisplayOption(&self.last_applied),
            DisplayOption(&self.current_leader),
            DisplayOption(&self.millis_since_quorum_ack),
        )?;

        write!(f, ", ")?;
        write!(
            f,
            "membership:{}, snapshot:{}, purged:{}, replication:{{{}}}",
            self.membership_config,
            DisplayOption(&self.snapshot),
            DisplayOption(&self.purged),
            self.replication
                .as_ref()
                .map(|x| { x.iter().map(|(k, v)| format!("{}:{}", k, DisplayOption(v))).collect::<Vec<_>>().join(",") })
                .unwrap_or_default(),
        )?;

        write!(f, "}}")?;
        Ok(())
    }
}

impl<C> RaftMetrics<C>
where C: RaftTypeConfig
{
    pub fn new_initial(id: C::NodeId) -> Self {
        Self {
            running_state: Ok(()),
            id,

            current_term: 0,
            vote: Vote::default(),
            last_log_index: None,
            last_applied: None,
            snapshot: None,
            purged: None,

            state: ServerState::Follower,
            current_leader: None,
            millis_since_quorum_ack: None,
            membership_config: Arc::new(StoredMembership::default()),
            replication: None,
        }
    }
}

/// Subset of RaftMetrics, only include data-related metrics
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct RaftDataMetrics<C: RaftTypeConfig> {
    pub last_log: Option<LogId<C::NodeId>>,
    pub last_applied: Option<LogId<C::NodeId>>,
    pub snapshot: Option<LogId<C::NodeId>>,
    pub purged: Option<LogId<C::NodeId>>,

    /// For a leader, it is the elapsed time in milliseconds since the most recently acknowledged
    /// timestamp by a quorum.
    ///
    /// It is `None` if this node is not leader, or the leader is not yet acknowledged by a quorum.
    /// Being acknowledged means receiving a reply of
    /// `AppendEntries`(`AppendEntriesRequest.vote.committed == true`).
    /// Receiving a reply of `RequestVote`(`RequestVote.vote.committed == false`) does not count,
    /// because a node will not maintain a lease for a vote with `committed == false`.
    ///
    /// This duration is used by the application to assess the likelihood that the leader has lost
    /// synchronization with the cluster.
    /// A longer duration without acknowledgment may suggest a higher probability of the leader
    /// being partitioned from the cluster.
    pub millis_since_quorum_ack: Option<u64>,

    pub replication: Option<ReplicationMetrics<C::NodeId>>,
}

impl<C> fmt::Display for RaftDataMetrics<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DataMetrics{{")?;

        write!(
            f,
            "last_log:{}, last_applied:{}, snapshot:{}, purged:{}, quorum_acked(leader):{} ms before, replication:{{{}}}",
            DisplayOption(&self.last_log),
            DisplayOption(&self.last_applied),
            DisplayOption(&self.snapshot),
            DisplayOption(&self.purged),
            self.millis_since_quorum_ack.display(),
            self.replication
                .as_ref()
                .map(|x| { x.iter().map(|(k, v)| format!("{}:{}", k, DisplayOption(v))).collect::<Vec<_>>().join(",") })
                .unwrap_or_default(),
        )?;

        write!(f, "}}")?;
        Ok(())
    }
}

/// Subset of RaftMetrics, only include server-related metrics
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct RaftServerMetrics<C: RaftTypeConfig> {
    pub id: C::NodeId,
    pub vote: Vote<C::NodeId>,
    pub state: ServerState,
    pub current_leader: Option<C::NodeId>,

    pub membership_config: Arc<StoredMembership<C>>,
}

impl<C> fmt::Display for RaftServerMetrics<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServerMetrics{{")?;

        write!(
            f,
            "id:{}, {:?}, vote:{}, leader:{}, membership:{}",
            self.id,
            self.state,
            self.vote,
            DisplayOption(&self.current_leader),
            self.membership_config,
        )?;

        write!(f, "}}")?;
        Ok(())
    }
}
