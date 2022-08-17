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

impl Display for LogId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.term, self.index)
    }
}
