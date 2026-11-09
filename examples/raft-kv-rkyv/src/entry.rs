use std::fmt;

use openraft::entry::RaftEntry;
use openraft::entry::RaftPayload;
use serde::Deserialize;
use serde::Serialize;

use crate::CommittedLeaderId;
use crate::LogId;
use crate::Membership;
use crate::payload::Payload;

/// The entry's log ID can change without rewriting its application archive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(crate) log_id: LogId,
    pub(crate) payload: Payload,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.log_id, self.payload)
    }
}

impl RaftEntry for Entry {
    type CommittedLeaderId = CommittedLeaderId;
    type Payload = Payload;

    fn new(log_id: LogId, payload: Payload) -> Self {
        Self { log_id, payload }
    }

    fn log_id_parts(&self) -> (&CommittedLeaderId, u64) {
        (&self.log_id.leader_id, self.log_id.index)
    }

    fn set_log_id(&mut self, new: LogId) {
        self.log_id = new;
    }

    fn get_membership(&self) -> Option<Membership> {
        self.payload.get_membership()
    }
}
