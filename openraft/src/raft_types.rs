use std::fmt::Display;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::LogId;
use crate::SnapshotSegmentId;

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
