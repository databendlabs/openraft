use serde::Deserialize;
use serde::Serialize;

use crate::MessageSummary;

/// The identity of a raft log.
/// A term and an index identifies an log globally.
#[derive(Debug, Default, Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogId {
    pub term: u64,
    pub index: u64,
}

impl MessageSummary for Option<LogId> {
    fn summary(&self) -> String {
        match self {
            None => "None".to_string(),
            Some(log_id) => {
                format!("{}", log_id)
            }
        }
    }
}
