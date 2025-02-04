use crate::pb;
use crate::typ::LogId;

impl From<LogId> for pb::LogId {
    fn from(log_id: LogId) -> Self {
        pb::LogId {
            term: *log_id.committed_leader_id(),
            index: log_id.index(),
        }
    }
}

impl From<pb::LogId> for LogId {
    fn from(proto_log_id: pb::LogId) -> Self {
        LogId::new(proto_log_id.term, proto_log_id.index)
    }
}
