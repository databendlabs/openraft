use std::fmt::Display;
use std::fmt::Formatter;

use crate::LeaderId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftTypeConfig;

/// The identity of a raft log.
/// A term, node_id and an index identifies an log globally.
#[derive(Debug, Default, Copy, Clone, PartialOrd, Ord, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LogId<NID: NodeId> {
    pub leader_id: LeaderId<NID>,
    pub index: u64,
}

pub trait RaftLogId<NID: NodeId> {
    fn get_log_id(&self) -> &LogId<NID>;

    fn set_log_id(&mut self, log_id: &LogId<NID>);
}

impl<NID: NodeId> RaftLogId<NID> for LogId<NID> {
    fn get_log_id(&self) -> &LogId<NID> {
        self
    }

    fn set_log_id(&mut self, log_id: &LogId<NID>) {
        *self = *log_id
    }
}

impl<NID: NodeId> Display for LogId<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.leader_id, self.index)
    }
}

impl<NID: NodeId> MessageSummary<LogId<NID>> for Option<LogId<NID>> {
    fn summary(&self) -> String {
        match self {
            None => "None".to_string(),
            Some(x) => {
                format!("{}", x)
            }
        }
    }
}

impl<NID: NodeId> LogId<NID> {
    pub fn new(leader_id: LeaderId<NID>, index: u64) -> Self {
        if leader_id.term == 0 || index == 0 {
            assert_eq!(
                leader_id.term, 0,
                "zero-th log entry must be (0,0,0), but {} {}",
                leader_id, index
            );
            assert_eq!(
                leader_id.node_id,
                NID::default(),
                "zero-th log entry must be (0,0,0), but {} {}",
                leader_id,
                index
            );
            assert_eq!(
                index, 0,
                "zero-th log entry must be (0,0,0), but {} {}",
                leader_id, index
            );
        }
        LogId { leader_id, index }
    }
}

pub trait LogIdOptionExt {
    fn index(&self) -> Option<u64>;
    fn next_index(&self) -> u64;
}

impl<NID: NodeId> LogIdOptionExt for Option<LogId<NID>> {
    fn index(&self) -> Option<u64> {
        self.map(|x| x.index)
    }

    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(log_id) => log_id.index + 1,
        }
    }
}

pub trait LogIndexOptionExt {
    fn next_index(&self) -> u64;
    fn prev_index(&self) -> Self;
    fn add(&self, v: u64) -> Self;
}

impl LogIndexOptionExt for Option<u64> {
    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(v) => v + 1,
        }
    }

    fn prev_index(&self) -> Self {
        match self {
            None => {
                panic!("None has no previous value");
            }
            Some(v) => {
                if *v == 0 {
                    None
                } else {
                    Some(*v - 1)
                }
            }
        }
    }

    fn add(&self, v: u64) -> Self {
        Some(self.next_index() + v).prev_index()
    }
}

// Everytime a snapshot is created, it is assigned with a globally unique id.
pub type SnapshotId = String;

/// The identity of a segment of a snapshot.
#[derive(Debug, Default, Clone, PartialOrd, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct SnapshotSegmentId {
    pub id: SnapshotId,
    pub offset: u64,
}

impl<D: ToString> From<(D, u64)> for SnapshotSegmentId {
    fn from(v: (D, u64)) -> Self {
        SnapshotSegmentId {
            id: v.0.to_string(),
            offset: v.1,
        }
    }
}

impl Display for SnapshotSegmentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}+{}", self.id, self.offset)
    }
}

// An update action with option to update with some value or just leave it as is.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq)]
pub enum Update<T> {
    Update(T),
    AsIs,
}

/// Describes the need to update some aspect of the metrics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MetricsChangeFlags {
    /// Replication state changes. E.g., adding/removing replication, progress update.
    pub(crate) replication: bool,

    /// Local data changes. Such as vote, log, snapshot.
    pub(crate) local_data: bool,

    /// State related to cluster: server state, leader, membership etc.
    pub(crate) cluster: bool,
}

impl MetricsChangeFlags {
    pub(crate) fn changed(&self) -> bool {
        self.replication || self.local_data || self.cluster
    }

    pub(crate) fn reset(&mut self) {
        self.replication = false;
        self.local_data = false;
        self.cluster = false;
    }

    /// Includes state of replication to other nodes.
    pub(crate) fn set_replication_changed(&mut self) {
        self.replication = true
    }

    /// Includes raft log, snapshot, state machine etc.
    pub(crate) fn set_data_changed(&mut self) {
        self.local_data = true
    }

    /// Includes node role, membership config, leader node etc.
    pub(crate) fn set_cluster_changed(&mut self) {
        self.cluster = true
    }
}

/// The changes of a state machine.
/// E.g. when applying a log to state machine, or installing a state machine from snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMachineChanges<C: RaftTypeConfig> {
    pub last_applied: LogId<C::NodeId>,
    pub is_snapshot: bool,
}
