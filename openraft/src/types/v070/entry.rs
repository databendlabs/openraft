use serde::Deserialize;
use serde::Serialize;

use super::AppData;
use super::LogId;
use super::Membership;

/// A Raft log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<D: AppData> {
    pub log_id: LogId,

    /// This entry's payload.
    #[serde(bound = "D: AppData")]
    pub payload: EntryPayload<D>,
}

/// Log entry payload variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryPayload<D: AppData> {
    /// An empty payload committed by a new cluster leader.
    Blank,

    #[serde(bound = "D: AppData")]
    Normal(D),

    /// A change-membership log entry.
    Membership(Membership),
}
