use openraft::raft::AppendError;
use openraft::raft::ConflictHint;
use serde::Deserialize;
use serde::Serialize;

use crate::typ::AppendResult;
use crate::typ::LogId;
use crate::typ::Vote;

/// An [`AppendResult`] in the simulated network's wire format.
///
/// [`AppendResult`] does not implement serde. This format keeps the log id the follower accepted
/// and its conflict hint, so the simulation exercises probes guided by hints.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
pub(super) enum AppendReply {
    /// The follower accepted the entries up to this log id.
    Accepted(Option<LogId>),

    /// The follower's log does not hold the request's `prev_log_id`.
    Conflict {
        /// The `prev_log_id` of the rejected request.
        log_id: LogId,

        /// The log ids of the follower's conflict hint, if it sent one.
        ///
        /// An empty list is a hint of its own: the follower's log is empty.
        hint: Option<Vec<LogId>>,
    },

    /// The follower has a higher vote than the leader.
    HigherVote(Vote),
}

impl From<AppendResult> for AppendReply {
    fn from(result: AppendResult) -> Self {
        match result {
            Ok(log_id) => AppendReply::Accepted(log_id),
            Err(AppendError::Conflict { log_id, hint }) => AppendReply::Conflict {
                log_id,
                hint: hint.map(|hint| hint.log_ids),
            },
            Err(AppendError::HigherVote(vote)) => AppendReply::HigherVote(vote),
        }
    }
}

impl From<AppendReply> for AppendResult {
    fn from(reply: AppendReply) -> Self {
        match reply {
            AppendReply::Accepted(log_id) => Ok(log_id),
            AppendReply::Conflict { log_id, hint } => Err(AppendError::Conflict {
                log_id,
                hint: hint.map(ConflictHint::new),
            }),
            AppendReply::HigherVote(vote) => Err(AppendError::HigherVote(vote)),
        }
    }
}

#[cfg(test)]
mod tests {
    use openraft::impls::leader_id_adv::LeaderId;
    use openraft::raft::AppendError;
    use openraft::raft::ConflictHint;
    use openraft::vote::RaftLeaderId;

    use super::AppendReply;
    use crate::typ::AppendResult;
    use crate::typ::LogId;
    use crate::typ::NodeId;
    use crate::typ::Vote;

    fn log_id(term: u64, node_id: NodeId, index: u64) -> LogId {
        LogId::new(LeaderId::new(term, node_id), index)
    }

    fn conflict(hint: Option<Vec<LogId>>) -> AppendResult {
        Err(AppendError::Conflict {
            log_id: log_id(2, 1, 7),
            hint: hint.map(ConflictHint::new),
        })
    }

    /// Every result survives the wire, including an empty hint, which stays distinct from no hint.
    #[test]
    fn reply_round_trips_through_bincode() {
        let committed = log_id(1, 1, 2);
        let tail = log_id(2, 1, 5);
        let results = vec![
            Ok(None),
            Ok(Some(tail)),
            conflict(None),
            conflict(Some(vec![])),
            conflict(Some(vec![committed, tail])),
            Err(AppendError::HigherVote(Vote::new(3, 2))),
        ];

        for expected in results {
            let reply = AppendReply::from(expected.clone());
            let bytes = bincode::serialize(&reply).unwrap();
            let decoded: AppendReply = bincode::deserialize(&bytes).unwrap();
            let actual = AppendResult::from(decoded);
            assert_eq!(expected, actual);
        }
    }
}
