use std::fmt;

use openraft::vote::RaftVote;

use crate::pb;
use crate::typ::LeaderId;
use crate::TypeConfig;

impl RaftVote<TypeConfig> for pb::Vote {
    fn from_leader_id(leader_id: LeaderId, committed: bool) -> Self {
        pb::Vote {
            leader_id: Some(leader_id),
            committed,
        }
    }

    fn leader_id(&self) -> Option<&LeaderId> {
        self.leader_id.as_ref()
    }

    fn is_committed(&self) -> bool {
        self.committed
    }
}

impl fmt::Display for pb::Vote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<{}:{}>",
            self.leader_id.as_ref().unwrap_or(&Default::default()),
            if self.is_committed() { "Q" } else { "-" }
        )
    }
}
