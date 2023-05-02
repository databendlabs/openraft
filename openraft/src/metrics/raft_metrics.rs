use std::fmt;
use std::sync::Arc;

use crate::core::ServerState;
use crate::display_ext::DisplayOption;
use crate::error::Fatal;
use crate::metrics::ReplicationMetrics;
use crate::node::Node;
use crate::summary::MessageSummary;
use crate::LogId;
use crate::NodeId;
use crate::StoredMembership;
use crate::Vote;

/// A set of metrics describing the current state of a Raft node.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct RaftMetrics<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub running_state: Result<(), Fatal<NID>>,

    /// The ID of the Raft node.
    pub id: NID,

    // ---
    // --- data ---
    // ---
    /// The current term of the Raft node.
    pub current_term: u64,

    /// The last accepted vote.
    pub vote: Vote<NID>,

    /// The last log index has been appended to this Raft node's log.
    pub last_log_index: Option<u64>,

    /// The last log index has been applied to this Raft node's state machine.
    pub last_applied: Option<LogId<NID>>,

    /// The id of the last log included in snapshot.
    /// If there is no snapshot, it is (0,0).
    pub snapshot: Option<LogId<NID>>,

    /// The last log id that has purged from storage, inclusive.
    ///
    /// `purged` is also the first log id Openraft knows, although the corresponding log entry has
    /// already been deleted.
    pub purged: Option<LogId<NID>>,

    // ---
    // --- cluster ---
    // ---
    /// The state of the Raft node.
    pub state: ServerState,

    /// The current cluster leader.
    pub current_leader: Option<NID>,

    /// The current membership config of the cluster.
    pub membership_config: Arc<StoredMembership<NID, N>>,

    // ---
    // --- replication ---
    // ---
    /// The replication states. It is Some() only when this node is leader.
    pub replication: Option<ReplicationMetrics<NID>>,
}

impl<NID, N> fmt::Display for RaftMetrics<NID, N>
where
    NID: NodeId,
    N: Node,
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

        write!(f, ", ")?;
        write!(
            f,
            "membership:{}, snapshot:{}, purged:{}, replication:{{{}}}",
            self.membership_config.summary(),
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
impl<NID, N> MessageSummary<RaftMetrics<NID, N>> for RaftMetrics<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn summary(&self) -> String {
        self.to_string()
    }
}

impl<NID, N> RaftMetrics<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub fn new_initial(id: NID) -> Self {
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
            membership_config: Arc::new(StoredMembership::default()),
            replication: None,
        }
    }
}
