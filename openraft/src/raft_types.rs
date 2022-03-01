use std::fmt::Display;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::LeaderId;
use crate::MessageSummary;
use crate::RaftTypeConfig;

/// The identity of a raft log.
/// A term, node_id and an index identifies an log globally.
#[derive(Debug, Default, Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogId<C: RaftTypeConfig> {
    pub leader_id: LeaderId<C>,
    pub index: u64,
}

impl<C: RaftTypeConfig> Display for LogId<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.leader_id, self.index)
    }
}

impl<C: RaftTypeConfig> MessageSummary for Option<LogId<C>> {
    fn summary(&self) -> String {
        match self {
            None => "None".to_string(),
            Some(x) => {
                format!("{}", x)
            }
        }
    }
}

impl<C: RaftTypeConfig> LogId<C> {
    pub fn new(leader_id: LeaderId<C>, index: u64) -> Self {
        if leader_id.term == 0 || index == 0 {
            assert_eq!(
                leader_id.term, 0,
                "zero-th log entry must be (0,0,0), but {} {}",
                leader_id, index
            );
            assert_eq!(
                leader_id.node_id,
                C::NodeId::default(),
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

pub trait LogIdOptionExt<C: RaftTypeConfig> {
    fn index(&self) -> Option<u64>;
    fn next_index(&self) -> u64;
}

impl<C: RaftTypeConfig> LogIdOptionExt<C> for Option<LogId<C>> {
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
#[derive(Debug, Default, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
pub enum Update<T> {
    Update(T),
    AsIs,
}

/// The changes of a state machine.
/// E.g. when applying a log to state machine, or installing a state machine from snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMachineChanges<C: RaftTypeConfig> {
    pub last_applied: LogId<C>,
    pub is_snapshot: bool,
}
