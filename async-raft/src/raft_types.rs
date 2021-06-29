use serde::Deserialize;
use serde::Serialize;

/// The identity of a raft log.
/// A term and an index identifies an log globally.
#[derive(Debug, Default, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogId {
    pub term: u64,
    pub index: u64,
}

// An update action with option to update with some value or just ignore this update.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq, Serialize, Deserialize)]
pub enum Update<T> {
    Update(T),
    Ignore,
}
