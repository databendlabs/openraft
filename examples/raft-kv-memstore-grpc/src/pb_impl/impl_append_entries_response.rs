use openraft::raft::ConflictHint;
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
            return match r.conflict_hint {
                None => AppendEntriesResponse::Conflict,
                Some(hint) => AppendEntriesResponse::ConflictWithHint(ConflictHint {
                    last_log_id: hint.last_log_id.map(Into::into),
                    committed_log_id: hint.committed_log_id.map(Into::into),
                }),
            };
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
                conflict_hint: None,
            },
            AppendEntriesResponse::PartialSuccess(p) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: p.map(|log_id| log_id.into()),
                conflict_hint: None,
            },
            AppendEntriesResponse::Conflict => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
                conflict_hint: None,
            },
            AppendEntriesResponse::ConflictWithHint(hint) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
                conflict_hint: Some(pb::ConflictHint {
                    last_log_id: hint.last_log_id.map(Into::into),
                    committed_log_id: hint.committed_log_id.map(Into::into),
                }),
            },
            AppendEntriesResponse::HigherVote(v) => pb::AppendEntriesResponse {
                rejected_by: Some(v),
                conflict: false,
                last_log_id: None,
                conflict_hint: None,
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
                conflict_hint: None,
            },
            Ok(None) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: None,
                conflict_hint: None,
            },
            Err(StreamAppendError::Conflict(conflict)) => {
                let hint = conflict.hint.map(|hint| pb::ConflictHint {
                    last_log_id: hint.last_log_id.map(Into::into),
                    committed_log_id: hint.committed_log_id.map(Into::into),
                });
                pb::AppendEntriesResponse {
                    rejected_by: None,
                    conflict: true,
                    last_log_id: Some(conflict.expect.into()),
                    conflict_hint: hint,
                }
            }
            Err(StreamAppendError::HigherVote(vote)) => pb::AppendEntriesResponse {
                rejected_by: Some(vote),
                conflict: false,
                last_log_id: None,
                conflict_hint: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_hint_message_presence_distinguishes_legacy_from_empty_follower() {
        tracing::info!("decode a legacy conflict without a hint");
        {
            let response = pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
                conflict_hint: None,
            };
            assert_eq!(AppendEntriesResponse::Conflict, response.into());
        }

        tracing::info!("decode an explicit hint from an empty follower");
        {
            let response = pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
                conflict_hint: Some(pb::ConflictHint {
                    last_log_id: None,
                    committed_log_id: None,
                }),
            };
            assert_eq!(
                AppendEntriesResponse::ConflictWithHint(ConflictHint {
                    last_log_id: None,
                    committed_log_id: None,
                }),
                response.into()
            );
        }
    }
}
