use openraft::raft::AppendError;

use crate::pb;
use crate::typ::AppendResult;

impl From<AppendResult> for pb::AppendEntriesResponse {
    fn from(result: AppendResult) -> Self {
        match result {
            Ok(Some(log_id)) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: Some(log_id.into()),
                hint: None,
            },
            Ok(None) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: None,
                hint: None,
            },
            Err(AppendError::Conflict { log_id, hint }) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: Some(log_id.into()),
                hint: hint.map(Into::into),
            },
            Err(AppendError::HigherVote(vote)) => pb::AppendEntriesResponse {
                rejected_by: Some(vote),
                conflict: false,
                last_log_id: None,
                hint: None,
            },
        }
    }
}
