use openraft::RaftTypeConfig;
use openraft::raft::AppendError;
use openraft::raft::AppendResult;
use openraft::raft::ConflictHint;
use openraft::type_config::alias::LogIdOf;
use openraft::type_config::alias::VoteOf;
use serde::Deserialize;
use serde::Serialize;

/// The JSON body of an `/append` reply: an [`AppendResult`] in this crate's wire format.
///
/// [`AppendResult`] does not implement serde, so this crate defines how it travels. Unlike
/// [`AppendEntriesResponse`], this format keeps the log id the follower accepted and its conflict
/// hint, which lets the leader skip part of its binary search after a conflict.
///
/// Changing this type changes the bytes on the wire. A deployment that upgrades its nodes one at
/// a time must version this format, so that every node still understands the replies of a node
/// that runs the previous version.
///
/// [`AppendEntriesResponse`]: openraft::raft::AppendEntriesResponse
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum AppendReply<C>
where C: RaftTypeConfig
{
    /// The follower accepted the entries up to this log id.
    Accepted(Option<LogIdOf<C>>),

    /// The follower's log does not hold the request's `prev_log_id`.
    Conflict {
        /// The `prev_log_id` of the rejected request.
        log_id: LogIdOf<C>,

        /// The log ids of the follower's conflict hint, if it sent one.
        ///
        /// An empty list is a hint of its own: the follower's log is empty.
        hint: Option<Vec<LogIdOf<C>>>,
    },

    /// The follower has a higher vote than the leader.
    HigherVote(VoteOf<C>),
}

impl<C> From<AppendResult<C>> for AppendReply<C>
where C: RaftTypeConfig
{
    fn from(result: AppendResult<C>) -> Self {
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

impl<C> From<AppendReply<C>> for AppendResult<C>
where C: RaftTypeConfig
{
    fn from(reply: AppendReply<C>) -> Self {
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
    use openraft::Vote;
    use openraft::impls::leader_id_adv::LeaderId;
    use openraft::raft::AppendError;
    use openraft::raft::AppendResult;
    use openraft::raft::ConflictHint;
    use openraft::type_config::alias::LogIdOf;
    use openraft::vote::RaftLeaderId;

    use super::AppendReply;

    openraft::declare_raft_types!(TestConfig);

    fn log_id(term: u64, node_id: u64, index: u64) -> LogIdOf<TestConfig> {
        LogIdOf::<TestConfig>::new(LeaderId::new(term, node_id), index)
    }

    fn conflict(hint: Option<Vec<LogIdOf<TestConfig>>>) -> AppendResult<TestConfig> {
        Err(AppendError::Conflict {
            log_id: log_id(2, 1, 7),
            hint: hint.map(ConflictHint::new),
        })
    }

    /// Every result survives the JSON wire, including an empty hint, which stays distinct from
    /// no hint.
    #[test]
    fn reply_round_trips_through_json() -> Result<(), serde_json::Error> {
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
            let json = serde_json::to_string(&reply)?;
            let decoded: AppendReply<TestConfig> = serde_json::from_str(&json)?;
            let actual: AppendResult<TestConfig> = decoded.into();
            assert_eq!(expected, actual);
        }
        Ok(())
    }
}
