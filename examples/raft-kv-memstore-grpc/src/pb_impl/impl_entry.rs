use std::fmt;
use std::fmt::write;

use openraft::alias::LogIdOf;
use openraft::entry::RaftEntry;
use openraft::entry::RaftPayload;
use openraft::Membership;
use openraft::RaftNetwork;

use crate::protobuf as pb;
use crate::TypeConfig;

impl fmt::Display for pb::Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entry{{term={},index={}}}", self.term, self.index)
    }
}

impl RaftPayload<TypeConfig> for pb::Entry {
    fn is_blank(&self) -> bool {
        self.payload.is_none()
    }

    fn get_membership(&self) -> Option<&Membership<TypeConfig>> {
        todo!()
    }
}

impl RaftEntry<TypeConfig> for pb::Entry {
    fn new_blank(log_id: LogIdOf<TypeConfig>) -> Self {
        Self {
            term: log_id.leader_id,
            index: log_id.index,
            payload: None,
        }
    }

    fn new_normal(log_id: LogIdOf<TypeConfig>, data: pb::SetRequest) -> Self {
        Self {
            term: log_id.leader_id,
            index: log_id.index,
            payload: Some(pb::entry::Payload::Normal(data)),
        }
    }

    fn new_membership(log_id: LogIdOf<TypeConfig>, m: Membership<TypeConfig>) -> Self {
        Self {
            term: log_id.leader_id,
            index: log_id.index,
            payload: Some(pb::entry::Payload::Membership(m.into())),
        }
    }

    fn log_id_parts(&self) -> (&u64, u64) {
        (&self.term, self.index)
    }

    fn set_log_id(&mut self, new: LogIdOf<TypeConfig>) {
        self.term = new.leader_id;
        self.index = new.index;
    }
}
