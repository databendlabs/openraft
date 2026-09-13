use openraft::raft::StreamAppendError;
use openraft::raft::StreamAppendSuccess;

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

        if r.partial_success || r.last_log_id.is_some() {
            return AppendEntriesResponse::PartialSuccess(r.last_log_id.map(Into::into));
        }

        AppendEntriesResponse::Success
    }
}

impl From<AppendEntriesResponse> for pb::AppendEntriesResponse {
    fn from(r: AppendEntriesResponse) -> Self {
        match r {
            AppendEntriesResponse::Success => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: None,
                partial_success: false,
            },
            AppendEntriesResponse::PartialSuccess(p) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: p.map(|log_id| log_id.into()),
                partial_success: true,
            },
            AppendEntriesResponse::Conflict => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
                partial_success: false,
            },
            AppendEntriesResponse::HigherVote(v) => pb::AppendEntriesResponse {
                rejected_by: Some(v),
                conflict: false,
                last_log_id: None,
                partial_success: false,
            },
        }
    }
}

impl From<StreamAppendResult> for pb::AppendEntriesResponse {
    fn from(result: StreamAppendResult) -> Self {
        match result {
            Ok(StreamAppendSuccess::Full(log_id)) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: log_id.map(Into::into),
                partial_success: false,
            },
            Ok(StreamAppendSuccess::Partial(log_id)) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: log_id.map(Into::into),
                partial_success: true,
            },
            Err(StreamAppendError::Conflict(log_id)) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: Some(log_id.into()),
                partial_success: false,
            },
            Err(StreamAppendError::HigherVote(vote)) => pb::AppendEntriesResponse {
                rejected_by: Some(vote),
                conflict: false,
                last_log_id: None,
                partial_success: false,
            },
        }
    }
}
