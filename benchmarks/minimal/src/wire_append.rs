use openraft::alias::LogIdOf;
use openraft::alias::VoteOf;
use openraft::raft::AppendError;
use openraft::raft::AppendResult;
use openraft::raft::ConflictHint;

use crate::store::TypeConfig;

/// Preserve conflict hints when exercising an actual serialized append reply.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum WireAppend {
    Accepted(Option<LogIdOf<TypeConfig>>),
    Conflict(LogIdOf<TypeConfig>, Option<Vec<LogIdOf<TypeConfig>>>),
    HigherVote(VoteOf<TypeConfig>),
}

impl From<AppendResult<TypeConfig>> for WireAppend {
    fn from(result: AppendResult<TypeConfig>) -> Self {
        match result {
            Ok(log) => Self::Accepted(log),
            Err(AppendError::HigherVote(vote)) => Self::HigherVote(vote),
            Err(AppendError::Conflict { log_id, hint }) => Self::Conflict(log_id, hint.map(|hint| hint.log_ids)),
        }
    }
}

impl From<WireAppend> for AppendResult<TypeConfig> {
    fn from(result: WireAppend) -> Self {
        match result {
            WireAppend::Accepted(log) => Ok(log),
            WireAppend::HigherVote(vote) => Err(AppendError::HigherVote(vote)),
            WireAppend::Conflict(log_id, hint) => Err(AppendError::Conflict {
                log_id,
                hint: hint.map(ConflictHint::new),
            }),
        }
    }
}
