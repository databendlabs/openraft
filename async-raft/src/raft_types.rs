use serde::Deserialize;
use serde::Serialize;

/// The identity of a raft log.
/// A term and an index identifies an log globally.
#[derive(Debug, Default, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogId {
    pub term: u64,
    pub index: u64,
}

impl From<(u64, u64)> for LogId {
    fn from(v: (u64, u64)) -> Self {
        LogId { term: v.0, index: v.1 }
    }
}

// An update action with option to update with some value or just ignore this update.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
pub enum Update<T> {
    Update(T),
    Ignore,
}
