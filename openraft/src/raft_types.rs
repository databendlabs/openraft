use std::fmt::Display;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

/// The identity of a raft log.
/// A term and an index identifies an log globally.
#[derive(Debug, Default, Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogId {
    pub term: u64,
    pub index: u64,
}

impl From<(u64, u64)> for LogId {
    fn from(v: (u64, u64)) -> Self {
        LogId::new(v.0, v.1)
    }
}

impl Display for LogId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.term, self.index)
    }
}

impl LogId {
    pub fn new(term: u64, index: u64) -> Self {
        if term == 0 || index == 0 {
            assert_eq!(index, 0, "zero-th log entry must be (0,0), but {}, {}", term, index);
            assert_eq!(term, 0, "zero-th log entry must be (0,0), but {}, {}", term, index);
        }
        LogId { term, index }
    }
}

pub trait LogIdOptionExt {
    fn index(&self) -> Option<u64>;
    fn next_index(&self) -> u64;
}

impl LogIdOptionExt for Option<LogId> {
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

// An update action with option to update with some value or just ignore this update.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
pub enum Update<T> {
    Update(T),
    Ignore,
}

/// The changes of a state machine.
/// E.g. when applying a log to state machine, or installing a state machine from snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMachineChanges {
    // TODO(xp): it does not need to be an Option
    pub last_applied: Option<LogId>,
    pub is_snapshot: bool,
}
