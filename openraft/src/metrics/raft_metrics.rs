use std::fmt;
use std::sync::Arc;

use crate::core::ServerState;
use crate::display_ext::DisplayBTreeMapOptValue;
use crate::display_ext::DisplayOption;
use crate::error::Fatal;
use crate::metrics::HeartbeatMetrics;
use crate::metrics::ReplicationMetrics;
use crate::metrics::SerdeInstant;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::SerdeInstantOf;
use crate::type_config::alias::VoteOf;
use crate::Instant;
use crate::RaftTypeConfig;
use crate::StoredMembership;

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
    pub current_term: C::Term,

    /// The last flushed vote.
    pub vote: VoteOf<C>,

    /// The last log index has been appended to this Raft node's log.
    pub last_log_index: Option<u64>,

    /// The last log index has been applied to this Raft node's state machine.
    pub last_applied: Option<LogIdOf<C>>,

    /// The id of the last log included in snapshot.
    /// If there is no snapshot, it is (0,0).
    pub snapshot: Option<LogIdOf<C>>,

    /// The last log id that has purged from storage, inclusive.
    ///
    /// `purged` is also the first log id Openraft knows, although the corresponding log entry has
    /// already been deleted.
    pub purged: Option<LogIdOf<C>>,

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
    ///
    /// Use `last_quorum_acked` instead, which is absolute timestamp.
    /// This value relates to the time when metrics is reported, which may behind the current time
    /// by an unknown duration(although it should be very small).
    #[deprecated(since = "0.10.0", note = "use `last_quorum_acked` instead.")]
    pub millis_since_quorum_ack: Option<u64>,

    /// For a leader, it is the most recently acknowledged timestamp by a quorum.
    ///
    /// It is `None` if this node is not leader, or the leader is not yet acknowledged by a quorum.
    /// Being acknowledged means receiving a reply of
    /// `AppendEntries`(`AppendEntriesRequest.vote.committed == true`).
    /// Receiving a reply of `RequestVote`(`RequestVote.vote.committed == false`) does not count,
    /// because a node will not maintain a lease for a vote with `committed == false`.
    ///
    /// This timestamp can be used by the application to assess the likelihood that the leader has
    /// lost synchronization with the cluster.
    /// An older value may suggest a higher probability of the leader being partitioned from the
    /// cluster.
    pub last_quorum_acked: Option<SerdeInstantOf<C>>,

    /// The current membership config of the cluster.
    pub membership_config: Arc<StoredMembership<C>>,

    /// Heartbeat metrics. It is Some() only when this node is leader.
    ///
    /// This field records a mapping between a node's ID and the time of the
    /// last acknowledged heartbeat or replication to this node.
    ///
    /// This duration since the recorded time can be used by applications to
    /// guess if a follwer/learner node is offline, longer duration suggests
    /// higher possibility of that.
    pub heartbeat: Option<HeartbeatMetrics<C>>,

    // ---
    // --- replication ---
    // ---
    /// The replication states. It is Some() only when this node is leader.
    pub replication: Option<ReplicationMetrics<C>>,
}

impl<C> fmt::Display for RaftMetrics<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Metrics{{")?;

        write!(
            f,
            "id:{}, {:?}, term:{}, vote:{}, last_log:{}, last_applied:{}, leader:{}",
            self.id,
            self.state,
            self.current_term,
            self.vote,
            DisplayOption(&self.last_log_index),
            DisplayOption(&self.last_applied),
            DisplayOption(&self.current_leader),
        )?;

        if let Some(quorum_acked) = &self.last_quorum_acked {
            write!(
                f,
                "(quorum_acked_time:{}, {:?} ago)",
                quorum_acked,
                quorum_acked.elapsed()
            )?;
        } else {
            write!(f, "(quorum_acked_time:None)")?;
        }

        write!(f, ", ")?;
        write!(
            f,
            "membership:{}, snapshot:{}, purged:{}, replication:{{{}}}, heartbeat:{{{}}}",
            self.membership_config,
            DisplayOption(&self.snapshot),
            DisplayOption(&self.purged),
            DisplayOption(&self.replication.as_ref().map(DisplayBTreeMapOptValue)),
            DisplayOption(&self.heartbeat.as_ref().map(DisplayBTreeMapOptValue)),
        )?;

        write!(f, "}}")?;
        Ok(())
    }
}

impl<C> RaftMetrics<C>
where C: RaftTypeConfig
{
    pub fn new_initial(id: C::NodeId) -> Self {
        #[allow(deprecated)]
        Self {
            running_state: Ok(()),
            id,

            current_term: Default::default(),
            vote: Default::default(),
            last_log_index: None,
            last_applied: None,
            snapshot: None,
            purged: None,

            state: ServerState::Follower,
            current_leader: None,
            millis_since_quorum_ack: None,
            last_quorum_acked: None,
            membership_config: Arc::new(StoredMembership::default()),
            replication: None,
            heartbeat: None,
        }
    }
}

/// Subset of RaftMetrics, only include data-related metrics
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct RaftDataMetrics<C: RaftTypeConfig> {
    pub last_log: Option<LogIdOf<C>>,
    pub last_applied: Option<LogIdOf<C>>,
    pub snapshot: Option<LogIdOf<C>>,
    pub purged: Option<LogIdOf<C>>,

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
    #[deprecated(since = "0.10.0", note = "use `last_quorum_acked` instead.")]
    pub millis_since_quorum_ack: Option<u64>,

    /// For a leader, it is the most recently acknowledged timestamp by a quorum.
    ///
    /// It is `None` if this node is not leader, or the leader is not yet acknowledged by a quorum.
    /// Being acknowledged means receiving a reply of
    /// `AppendEntries`(`AppendEntriesRequest.vote.committed == true`).
    /// Receiving a reply of `RequestVote`(`RequestVote.vote.committed == false`) does not count,
    /// because a node will not maintain a lease for a vote with `committed == false`.
    ///
    /// This timestamp can be used by the application to assess the likelihood that the leader has
    /// lost synchronization with the cluster.
    /// An older value may suggest a higher probability of the leader being partitioned from the
    /// cluster.
    pub last_quorum_acked: Option<SerdeInstant<InstantOf<C>>>,

    pub replication: Option<ReplicationMetrics<C>>,

    /// Heartbeat metrics. It is Some() only when this node is leader.
    ///
    /// This field records a mapping between a node's ID and the time of the
    /// last acknowledged heartbeat or replication to this node.
    ///
    /// This duration since the recorded time can be used by applications to
    /// guess if a follwer/learner node is offline, longer duration suggests
    /// higher possibility of that.
    pub heartbeat: Option<HeartbeatMetrics<C>>,
}

impl<C> fmt::Display for RaftDataMetrics<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DataMetrics{{")?;

        write!(
            f,
            "last_log:{}, last_applied:{}, snapshot:{}, purged:{}",
            DisplayOption(&self.last_log),
            DisplayOption(&self.last_applied),
            DisplayOption(&self.snapshot),
            DisplayOption(&self.purged),
        )?;

        if let Some(quorum_acked) = &self.last_quorum_acked {
            write!(
                f,
                ", quorum_acked_time:({}, {:?} ago)",
                quorum_acked,
                quorum_acked.elapsed()
            )?;
        } else {
            write!(f, ", quorum_acked_time:None")?;
        }

        write!(
            f,
            ", replication:{{{}}}, heartbeat:{{{}}}",
            DisplayOption(&self.replication.as_ref().map(DisplayBTreeMapOptValue)),
            DisplayOption(&self.heartbeat.as_ref().map(DisplayBTreeMapOptValue)),
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
    pub vote: VoteOf<C>,
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
