use openraft::raft::AppendError;

use crate::pb;
use crate::typ::AppendEntriesResponse;
use crate::typ::AppendResult;

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
                hint: None,
            },
            AppendEntriesResponse::PartialSuccess(p) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: p.map(|log_id| log_id.into()),
                hint: None,
            },
            AppendEntriesResponse::Conflict => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
                hint: None,
            },
            AppendEntriesResponse::HigherVote(v) => pb::AppendEntriesResponse {
                rejected_by: Some(v),
                conflict: false,
                last_log_id: None,
                hint: None,
            },
        }
    }
}

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

#[cfg(test)]
mod tests {
    use prost::Message;

    use crate::pb;
    use crate::typ::AppendEntriesResponse;

    /// The hint field leaves the unary encoding unchanged.
    #[test]
    fn unary_conflict_wire_format_is_unchanged() {
        let response = pb::AppendEntriesResponse::from(AppendEntriesResponse::Conflict);
        let bytes = response.encode_to_vec();
        assert_eq!(vec![0x10, 0x01], bytes);
    }
}
