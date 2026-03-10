use std::fmt;

use openraft::EntryPayload;
use openraft::alias::LogIdOf;
use openraft::entry::RaftEntry;
use openraft::entry::RaftPayload;

use crate::TypeConfig;
use crate::protobuf as pb;
use crate::typ;

impl fmt::Display for pb::Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entry{{term={},index={}}}", self.term, self.index)
    }
}

impl RaftPayload<crate::NodeId, pb::Node> for pb::Entry {
    fn get_membership(&self) -> Option<typ::Membership> {
        self.membership.clone().map(Into::into)
    }
}

impl RaftEntry for pb::Entry {
    type CommittedLeaderId = u64;
    type D = pb::SetRequest;
    type NodeId = u64;
    type Node = pb::Node;

    fn new(log_id: LogIdOf<TypeConfig>, payload: EntryPayload<pb::SetRequest, u64, pb::Node>) -> Self {
        let mut app_data = None;
        let mut membership = None;
        match payload {
            EntryPayload::Blank => {}
            EntryPayload::Normal(data) => app_data = Some(data),
            EntryPayload::Membership(m) => membership = Some(m.into()),
        }

        Self {
            term: log_id.leader_id,
            index: log_id.index,
            app_data,
            membership,
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
