use openraft::raft::ConflictHint;

use crate::TypeConfig;
use crate::pb;

impl From<pb::ConflictHint> for ConflictHint<TypeConfig> {
    fn from(hint: pb::ConflictHint) -> Self {
        let log_ids = hint.log_ids.into_iter().map(Into::into).collect();
        ConflictHint::new(log_ids)
    }
}

impl From<ConflictHint<TypeConfig>> for pb::ConflictHint {
    fn from(hint: ConflictHint<TypeConfig>) -> Self {
        let log_ids = hint.log_ids.into_iter().map(Into::into).collect();
        pb::ConflictHint { log_ids }
    }
}
