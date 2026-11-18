use openraft::Vote;
use openraft::impls::leader_id_adv::LeaderId;
use openraft::raft::AppendError;
use openraft::raft::AppendResult;
use openraft::raft::ConflictHint;
use openraft::type_config::alias::LogIdOf;
use openraft::vote::RaftLeaderId;

use crate::AppendReply;

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
