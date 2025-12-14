use openraft::raft::StreamAppendError;

use crate::pb;
use crate::typ::AppendEntriesResponse;
use crate::typ::StreamAppendResult;

impl From<pb::AppendEntriesResponse> for AppendEntriesResponse {
    fn from(r: pb::AppendEntriesResponse) -> Self {
        if let Some(higher) = r.rejected_by {
            return AppendEntriesResponse::HigherVote(higher);
        }

        if r.conflict {
            return AppendEntriesResponse::Conflict;
        }

        if let Some(log_id) = r.last_log_id {
            AppendEntriesResponse::PartialSuccess(Some(log_id.into()))
        } else {
            AppendEntriesResponse::Success
        }
    }
}

impl From<AppendEntriesResponse> for pb::AppendEntriesResponse {
    fn from(r: AppendEntriesResponse) -> Self {
        match r {
            AppendEntriesResponse::Success => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: None,
            },
            AppendEntriesResponse::PartialSuccess(p) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: p.map(|log_id| log_id.into()),
            },
            AppendEntriesResponse::Conflict => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
            },
            AppendEntriesResponse::HigherVote(v) => pb::AppendEntriesResponse {
                rejected_by: Some(v),
                conflict: false,
                last_log_id: None,
            },
        }
    }
}

impl From<StreamAppendResult> for pb::AppendEntriesResponse {
    fn from(result: StreamAppendResult) -> Self {
        match result {
            Ok(Some(log_id)) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: Some(log_id.into()),
            },
            Ok(None) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: None,
            },
            Err(StreamAppendError::Conflict(log_id)) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: Some(log_id.into()),
            },
            Err(StreamAppendError::HigherVote(vote)) => pb::AppendEntriesResponse {
                rejected_by: Some(vote),
                conflict: false,
                last_log_id: None,
            },
        }
    }
}
